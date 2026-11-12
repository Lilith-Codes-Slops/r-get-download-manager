//! Validation for a confirmed start before admitting a worker.
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ConfirmedStart {
    pub filename: String,
    pub folder: PathBuf,
}

pub(super) fn validate_filename(filename: &str) -> Result<()> {
    let mut components = Path::new(filename).components();
    if filename.trim().is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("confirmed filename must be a single safe path component".into());
    }
    Ok(())
}

/// Resolve the one-time directory without changing the daemon's saved setting.
/// Call off the async executor; the binding must be rechecked at admission.
#[allow(dead_code)]
pub(super) fn validate(filename: &str, folder: &Path) -> Result<ConfirmedStart> {
    validate_filename(filename)?;
    let folder = super::download_directory::validate(folder)?;
    Ok(ConfirmedStart {
        filename: filename.to_owned(),
        folder,
    })
}

/// A task keeps its identity, root and name while retrying. A different root
/// needs a new task, so the existing identity must first be removed (with data
/// kept if desired). Do not use an old partial under a different approval.
#[allow(dead_code)]
pub(super) fn validate_retry(
    existing_root: &Path,
    existing_name: Option<&str>,
    has_partial: bool,
    request: &ConfirmedStart,
) -> Result<()> {
    if existing_root != request.folder.as_path() {
        return Err(
            "remove the existing task before starting this source in a different folder".into(),
        );
    }
    if existing_name.is_some_and(|name| name != request.filename.as_str())
        || (has_partial && existing_name.is_none())
    {
        return Err("retry must keep the existing confirmed filename and partial data".into());
    }
    Ok(())
}

/// Staged implementation for confirmed admission. The IPC handler intentionally
/// does not call this until the retry and publication paths are ready.
#[allow(dead_code)]
pub(super) async fn start(
    request: downloader_ipc::ClientRequest,
    tasks: super::control::Tasks,
    controls: super::control::Controls,
    limiter: std::sync::Arc<super::bandwidth::BandwidthLimiter>,
    store: std::sync::Arc<super::persistence::Persistence>,
) -> downloader_ipc::DaemonEvent {
    use downloader_ipc::{ClientRequest, DaemonEvent};
    let ClientRequest::AddConfirmedTask {
        url,
        mirrors,
        connections,
        extract,
        category,
        not_before,
        filename,
        folder,
    } = request
    else {
        return DaemonEvent::TaskFailed {
            task_id: String::new(),
            error: "expected confirmed start".into(),
        };
    };
    if !(1..=32).contains(&connections)
        || !mirrors.is_empty()
        || extract
        || category.is_some()
        || not_before.is_some()
    {
        return DaemonEvent::TaskFailed { task_id: url, error: "connections must be 1–32; mirrors, extraction, categories and scheduling are not yet supported".into() };
    }
    let confirmed =
        match tokio::task::spawn_blocking(move || validate(&filename, Path::new(&folder))).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                return DaemonEvent::TaskFailed {
                    task_id: url,
                    error: error.to_string(),
                }
            }
            Err(error) => {
                return DaemonEvent::TaskFailed {
                    task_id: url,
                    error: format!("could not validate confirmed destination: {error}"),
                }
            }
        };
    if super::torrent_transfer_dispatch::is_torrent(&url) {
        return super::torrent_transfer_dispatch::add_confirmed(
            url,
            connections,
            confirmed,
            tasks,
            controls,
            limiter,
            store,
        )
        .await;
    }
    if !matches!(reqwest::Url::parse(&url), Ok(ref value) if matches!(value.scheme(), "http" | "https"))
    {
        return DaemonEvent::TaskFailed {
            task_id: url,
            error: "only valid HTTP and HTTPS URLs are supported".into(),
        };
    }
    admit_http(url, connections, confirmed, tasks, controls, limiter, store).await
}

/// Caller must provide a validated destination. Serialize duplicate checking,
/// durable binding and in-memory insertion with the normal queue snapshot gate.
async fn admit_http(
    url: String,
    connections: u8,
    confirmed: ConfirmedStart,
    tasks: super::control::Tasks,
    controls: super::control::Controls,
    limiter: std::sync::Arc<super::bandwidth::BandwidthLimiter>,
    store: std::sync::Arc<super::persistence::Persistence>,
) -> downloader_ipc::DaemonEvent {
    use super::control::TransferControl;
    use downloader_ipc::{DaemonEvent, DownloadInfo};
    let admission = store.gate.lock().await;
    let mut guard = tasks.lock().await;
    if guard.contains_key(&url) {
        return DaemonEvent::TaskFailed {
            task_id: url,
            error: "download already exists; remove the existing task first".into(),
        };
    }
    let task = DownloadInfo {
        url: url.clone(),
        state: "downloading".into(),
        filename: None,
        downloaded: 0,
        total: None,
        speed_bps: 0,
        final_path: None,
        destination_path: None,
        suggested_filename: None,
        confirmed_filename: Some(confirmed.filename),
        torrent: None,
        resume_supported: false,
        partial_name: None,
        requested_connections: Some(connections),
        active_connections: None,
        segment_count: None,
        connection_note: None,
        live_connections_supported: false,
    };
    if let Err(error) = store
        .bind_task_directory_locked_at(&task, false, confirmed.folder)
        .await
    {
        return DaemonEvent::TaskFailed {
            task_id: url,
            error: format!("could not bind task directory: {error}"),
        };
    }
    guard.insert(url.clone(), task);
    let (sender, receiver) = tokio::sync::watch::channel(TransferControl::Running);
    controls.lock().await.insert(url.clone(), sender);
    // Start while admission is still exclusive: removal must not complete and
    // admit a replacement between installing controls and launching the worker.
    spawn_http(
        url.clone(),
        connections,
        None,
        tasks.clone(),
        limiter.clone(),
        store.clone(),
        receiver,
    );
    drop(guard);
    drop(admission);
    DaemonEvent::TaskStateChanged {
        task_id: url,
        state: "downloading".into(),
        destination_path: None,
    }
}

/// A retry is an operation on an existing task, never another add request.
/// It has no new folder or filename inputs and preserves the saved partial.
/// Kept internal until retry and collision handling are verified end-to-end.
#[allow(dead_code)]
pub(super) async fn retry_http(
    id: String,
    tasks: super::control::Tasks,
    controls: super::control::Controls,
    limiter: std::sync::Arc<super::bandwidth::BandwidthLimiter>,
    store: std::sync::Arc<super::persistence::Persistence>,
) -> downloader_ipc::DaemonEvent {
    use super::control::TransferControl;
    use downloader_ipc::DaemonEvent;
    let admission = store.gate.lock().await;
    let mut guard = tasks.lock().await;
    let Some(prior) = guard.get(&id).cloned() else {
        return DaemonEvent::TaskFailed {
            task_id: id,
            error: "task not found".into(),
        };
    };
    if prior.torrent.is_some()
        || prior.confirmed_filename.is_none()
        || !(prior.state.starts_with("failed:") || prior.state == "canceled")
    {
        return DaemonEvent::TaskFailed {
            task_id: id,
            error: "task is not a stopped confirmed HTTP download".into(),
        };
    }
    let root = match store.task_directory(&id).await {
        Ok(root) => root,
        Err(error) => {
            return DaemonEvent::TaskFailed {
                task_id: id,
                error: format!("could not resolve task directory: {error}"),
            }
        }
    };
    let connections = prior.requested_connections.unwrap_or(1);
    if !(1..=32).contains(&connections) {
        return DaemonEvent::TaskFailed {
            task_id: id,
            error: "invalid saved connection count".into(),
        };
    }
    let mut next = prior.clone();
    next.state = "downloading".into();
    next.speed_bps = 0;
    next.active_connections = None;
    if let Err(error) = store.bind_task_directory_locked_at(&next, true, root).await {
        return DaemonEvent::TaskFailed {
            task_id: id,
            error: format!("could not retain task directory: {error}"),
        };
    }
    guard.insert(id.clone(), next);
    let (sender, receiver) = tokio::sync::watch::channel(TransferControl::Running);
    controls.lock().await.insert(id.clone(), sender);
    spawn_http(
        id.clone(),
        connections,
        Some(prior),
        tasks.clone(),
        limiter.clone(),
        store.clone(),
        receiver,
    );
    drop(guard);
    drop(admission);
    DaemonEvent::TaskStateChanged {
        task_id: id,
        state: "downloading".into(),
        destination_path: None,
    }
}

fn spawn_http(
    id: String,
    connections: u8,
    prior: Option<downloader_ipc::DownloadInfo>,
    tasks: super::control::Tasks,
    limiter: std::sync::Arc<super::bandwidth::BandwidthLimiter>,
    store: std::sync::Arc<super::persistence::Persistence>,
    receiver: tokio::sync::watch::Receiver<super::control::TransferControl>,
) {
    use super::control::TransferControl;
    tokio::spawn(async move {
        let result = super::retry::run(
            &id,
            connections,
            prior,
            tasks.clone(),
            limiter.clone(),
            store.clone(),
            receiver.clone(),
        )
        .await;
        {
            let mut guard = tasks.lock().await;
            if let Some(task) = guard.get_mut(&id) {
                task.speed_bps = 0;
                task.active_connections = Some(0);
                task.live_connections_supported = false;
                if *receiver.borrow() == TransferControl::Canceled {
                    task.state = "canceled".into();
                    task.resume_supported = false;
                } else {
                    match result {
                        Ok(path) => {
                            task.state = "completed".into();
                            task.final_path = Some(path.to_string_lossy().into_owned());
                            // Confirmed HTTP cleanup is deferred until it can be
                            // performed relative to the verified root. Keep the
                            // durable task-to-partial association meanwhile.
                            if task.confirmed_filename.is_none() {
                                task.partial_name = None;
                            }
                        }
                        Err(error) => {
                            task.state = format!("failed: {error}");
                            task.resume_supported = false;
                        }
                    }
                }
            }
        }
        super::progress_events::notify();
        if let Err(error) = store.save(&tasks, &limiter).await {
            eprintln!("queue checkpoint failed: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_filename_and_canonicalizes_folder_without_guessing() {
        let root = std::env::temp_dir().join(format!(
            "rget-confirmed-start-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let folder = root.join("selected");
        std::fs::create_dir(&folder).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        let selected = validate(" chosen.txt ", &link).unwrap();
        assert_eq!(selected.filename, " chosen.txt ");
        assert_eq!(selected.folder, folder.canonicalize().unwrap());
        for bad in ["", " ", ".", "..", "a/b", "a/", "a/.", "a\\b", "a\0b"] {
            assert!(validate(bad, &folder).is_err(), "{bad:?}");
        }
        assert!(validate("a", Path::new("relative")).is_err());
        assert!(validate("a", &root.join("missing")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_keeps_root_name_and_partial_identity() {
        let request = ConfirmedStart {
            filename: "file.txt".into(),
            folder: "/downloads".into(),
        };
        assert!(validate_retry(Path::new("/downloads"), Some("file.txt"), true, &request).is_ok());
        assert!(validate_retry(Path::new("/other"), Some("file.txt"), true, &request).is_err());
        assert!(
            validate_retry(Path::new("/downloads"), Some("other.txt"), true, &request).is_err()
        );
        assert!(validate_retry(Path::new("/downloads"), None, true, &request).is_err());
    }
}

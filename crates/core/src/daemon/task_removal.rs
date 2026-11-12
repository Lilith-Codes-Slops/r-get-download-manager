//! Cancel a transfer before removing its record and, optionally, its owned files.
use super::bandwidth::BandwidthLimiter;
use super::control::{Controls, Tasks, TransferControl};
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Duration;

fn active(state: &str) -> bool {
    matches!(
        state,
        "queued"
            | "metadata"
            | "peers"
            | "connecting"
            | "downloading"
            | "paused"
            | "pausing"
            | "resuming"
            | "finished"
    )
}

pub(super) async fn cancel(tasks: &Tasks, controls: &Controls, id: String) -> DaemonEvent {
    let mut guard = tasks.lock().await;
    let Some(task) = guard.get_mut(&id) else {
        return failed(id, "task not found");
    };
    if task.state == "canceling" || task.state == "canceled" {
        return DaemonEvent::TaskStateChanged {
            task_id: id,
            state: task.state.clone(),
            destination_path: super::destination::path(task),
        };
    }
    if !active(&task.state) {
        return failed(id, "task is not an active download");
    }
    let senders = controls.lock().await;
    let Some(sender) = senders.get(&id) else {
        return failed(id, "transfer control unavailable");
    };
    task.state = "canceling".into();
    task.speed_bps = 0;
    sender.send_replace(TransferControl::Canceled);
    DaemonEvent::TaskStateChanged {
        task_id: id,
        state: "canceling".into(),
        destination_path: super::destination::path(task),
    }
}

pub(super) async fn remove(
    tasks: &Tasks,
    controls: &Controls,
    limiter: &Arc<BandwidthLimiter>,
    store: &super::persistence::Persistence,
    id: String,
    delete_file: bool,
    root: Option<&Path>,
) -> DaemonEvent {
    // Wait for the worker to close/clean its partial before unlinking or allowing
    // a new transfer with the same URL. Do not hold the task lock while waiting.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let mut guard = tasks.lock().await;
        let Some(task) = guard.get(&id) else {
            return failed(id, "task not found");
        };
        if active(&task.state) {
            drop(guard);
            if let DaemonEvent::TaskFailed { error, .. } = cancel(tasks, controls, id.clone()).await
            {
                // The transfer may have completed between observing and canceling.
                if error != "task is not an active download" {
                    return failed(id, &error);
                }
            }
        } else if matches!(task.state.as_str(), "canceling" | "finalizing") {
            drop(guard);
        } else {
            #[cfg(target_os = "linux")]
            if task.torrent.is_none() {
                // Both removal modes must retain the task and journal while
                // publication is unresolved, so a later task cannot inherit
                // or discard an old attempt's evidence.
                let database = store.database_path();
                let task_id = id.clone();
                let pending = tokio::task::spawn_blocking(move || {
                    let conn = super::persistence::open(&database)?;
                    super::http_publication_intent::exists(&conn, &task_id)
                }).await;
                match pending {
                    Ok(Ok(false)) => {}
                    Ok(Ok(true)) => return failed(id, "publication outcome uncertain—inspection required; task and data retained"),
                    Ok(Err(error)) => return failed(id, &format!("could not inspect HTTP publication journal: {error}")),
                    Err(error) => return failed(id, &format!("could not inspect HTTP publication journal: {error}")),
                }
            }
            if delete_file {
                // Until removal is bound to the durable inode reservation and
                // lifecycle barrier, a name alone cannot authorize deletion.
                if task.confirmed_filename.is_some()
                    && task.torrent.is_none()
                    && task.partial_name.is_some()
                {
                    return failed(id, "confirmed partial deletion is unavailable until ownership-safe removal is implemented; data retained");
                }
                #[cfg(target_os = "linux")]
                match store.task_directory(&id).await {
                    Ok(current) if root == Some(current.as_path()) => {}
                    Ok(_) => return failed(id, "task directory changed during removal; retry"),
                    Err(error) => {
                        return failed(id, &format!("cannot resolve task directory: {error}"))
                    }
                }
                #[cfg(not(target_os = "linux"))]
                let _ = store;
                if let Err(error) = remove_owned_files(task, root).await {
                    return failed(id, &error);
                }
            }
            guard.remove(&id);
            controls.lock().await.remove(&id);
            limiter.remove(&id).await;
            return DaemonEvent::TaskStateChanged {
                task_id: id,
                state: "removed".into(),
                destination_path: None,
            };
        }
        if tokio::time::Instant::now() >= deadline {
            return failed(
                id,
                "timed out waiting for the transfer to stop; task was not removed",
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn failed(id: String, error: &str) -> DaemonEvent {
    DaemonEvent::TaskFailed {
        task_id: id,
        error: error.into(),
    }
}

fn single_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.contains('/')
        && !name.contains('\\')
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
}

async fn unlink_if_present(path: &Path) -> Result<(), String> {
    match tokio::fs::symlink_metadata(path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not inspect {}: {e}", path.display())),
        Ok(meta) if !meta.is_file() => Err(format!(
            "refusing to delete non-regular file: {}",
            path.display()
        )),
        Ok(_) => match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not delete {}: {e}", path.display())),
        },
    }
}

async fn remove_owned_files(task: &DownloadInfo, root: Option<&Path>) -> Result<(), String> {
    if task.final_path.is_none()
        && task.partial_name.is_none()
        && task
            .torrent
            .as_ref()
            .and_then(|t| t.stage_name.as_ref())
            .is_none()
    {
        return Ok(());
    }
    let root = root.ok_or("task download root is unavailable")?;
    remove_files_in(task, root).await
}

async fn remove_files_in(task: &DownloadInfo, root: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    super::torrent_cleanup::remove_stage(task, root).await?;
    if let Some(final_path) = &task.final_path {
        let name = task
            .filename
            .as_deref()
            .filter(|name| single_name(name))
            .ok_or("unsafe or missing final filename")?;
        let expected = root.join(name);
        if Path::new(final_path) != expected {
            return Err("refusing to delete a file outside the task's Downloads path".into());
        }
        unlink_if_present(&expected).await?;
    }
    if let Some(name) = &task.partial_name {
        if !single_name(name) || !name.starts_with(".rget-") || !name.ends_with(".part") {
            return Err("refusing to delete an unsafe partial filename".into());
        }
        let part = root.join(name);
        unlink_if_present(&part).await?;
        let sidecar = root.join(format!("{name}.meta"));
        unlink_if_present(&sidecar).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_outside_paths_and_does_not_delete_unowned_final_files() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rget-delete-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join("existing.bin");
        tokio::fs::write(&path, b"unrelated").await.unwrap();
        let mut task = DownloadInfo {
            url: "https://example.test".into(),
            state: "failed".into(),
            filename: Some("existing.bin".into()),
            downloaded: 0,
            total: None,
            speed_bps: 0,
            final_path: None,
            destination_path: None,
            suggested_filename: None,
            confirmed_filename: None,
            torrent: None,
            resume_supported: false,
            partial_name: None,
            requested_connections: None,
            active_connections: None,
            segment_count: None,
            connection_note: None,
            live_connections_supported: false,
        };
        remove_files_in(&task, &root).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"unrelated");
        task.final_path = Some(root.join("../outside").to_string_lossy().into_owned());
        assert!(remove_files_in(&task, &root).await.is_err());
        task.final_path = None;
        task.partial_name = Some(".rget-../outside.part".into());
        assert!(remove_files_in(&task, &root).await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"unrelated");
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

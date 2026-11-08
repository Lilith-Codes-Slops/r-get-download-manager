use downloader_ipc::transport::{EndpointPathResolver, IpcConnectionPool};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(target_os = "linux")]
use tokio::net::UnixListener;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

pub struct ServerLoop {
    pool: IpcConnectionPool,
}

impl ServerLoop {
    pub fn new(pool: IpcConnectionPool) -> Self {
        Self { pool }
    }

    pub async fn run(&self, mut rx: mpsc::Receiver<()>) {
        let endpoint = match EndpointPathResolver::resolve() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("failed to resolve endpoint: {e}");
                return;
            }
        };

        #[cfg(target_os = "linux")]
        {
            let lock_path = format!("{endpoint}.lock");
            let _lock = match super::SingleInstanceLock::acquire(std::path::Path::new(&lock_path)) {
                Ok(lock) => lock,
                Err(e) => { eprintln!("failed to acquire daemon lock: {e}"); return; }
            };
            let store = match super::persistence::Persistence::default_path() {
                Ok(path) => Arc::new(super::persistence::Persistence::new(path)),
                Err(e) => { eprintln!("failed to configure persistent state: {e}"); return; }
            };
            let saved = match store.load().await {
                Ok(saved) => saved,
                Err(e) => { eprintln!("failed to load persistent state: {e}"); return; }
            };
            let root = std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join("Downloads"));
            if root.is_none() { eprintln!("HOME is not set; cannot recover download files"); return; }
            let (saved, recovered, split_recovered, root) = match tokio::task::spawn_blocking(move || {
                let mut saved = saved;
                let split_recovered = root.as_ref().map(|root| super::split::startup::reconcile(&mut saved, root)).unwrap_or_default();
                let recovered = root.as_ref().map(|root| super::recovery::reconcile(&mut saved, root)).unwrap_or_default();
                (saved, recovered, split_recovered, root)
            }).await {
                Ok(value) => value,
                Err(e) => { eprintln!("failed to prepare recovery: {e}"); return; }
            };
            let tasks = Arc::new(Mutex::new(saved.tasks));
            let limiter = Arc::new(super::bandwidth::BandwidthLimiter::new());
            if let Err(e) = limiter.set(None, saved.global_kbps).await {
                eprintln!("failed to restore global speed limit: {e}"); return;
            }
            for (id, rate) in saved.task_kbps {
                if let Err(e) = limiter.set(Some(&id), rate).await {
                    eprintln!("failed to restore task speed limit: {e}"); return;
                }
            }
            for item in &recovered {
                if let Err(e) = store.save_recovery(&tasks, &item.record).await {
                    eprintln!("failed to reconcile recovered checkpoint: {e}"); return;
                }
            }
            for item in &split_recovered {
                if let Err(e) = store.save_split_recovery(&tasks, &item.record).await {
                    eprintln!("failed to reconcile recovered split checkpoint: {e}"); return;
                }
            }
            if let Err(e) = store.save(&tasks, &limiter).await {
                eprintln!("failed to save recovered state: {e}"); return;
            }
            // Only remove a stale socket, never an unrelated file or symlink.
            use std::os::unix::fs::FileTypeExt;
            match std::fs::symlink_metadata(&endpoint) {
                Ok(metadata) if metadata.file_type().is_socket() => {
                    if let Err(e) = std::fs::remove_file(&endpoint) {
                        eprintln!("failed to remove stale socket {endpoint}: {e}");
                        return;
                    }
                }
                Ok(_) => { eprintln!("refusing to replace non-socket endpoint {endpoint}"); return; }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => { eprintln!("failed to inspect endpoint {endpoint}: {e}"); return; }
            }
            let listener = match UnixListener::bind(&endpoint) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("failed to bind {endpoint}: {e}");
                    return;
                }
            };
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)) {
                eprintln!("failed to secure daemon socket: {e}");
                let _ = std::fs::remove_file(&endpoint);
                return;
            }
            let controls = Arc::new(Mutex::new(BTreeMap::<String, tokio::sync::watch::Sender<super::control::TransferControl>>::new()));
            for item in recovered {
                super::recovered_transfer::spawn(item, tasks.clone(), controls.clone(), limiter.clone(), store.clone()).await;
            }
            if let Some(root) = &root {
                for item in split_recovered {
                    super::split::live::spawn(item, root, tasks.clone(), controls.clone(), limiter.clone(), store.clone()).await;
                }
            }
            eprintln!("daemon listening on {endpoint}");
            let checkpoint = {
                let store = store.clone();
                let tasks = tasks.clone();
                let limiter = limiter.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
                    loop {
                        interval.tick().await;
                        if let Err(e) = store.save(&tasks, &limiter).await { eprintln!("queue checkpoint failed: {e}"); }
                    }
                })
            };

            loop {
                tokio::select! {
                    _ = rx.recv() => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut stream, _)) => {
                                let tasks = tasks.clone();
                                let limiter = limiter.clone();
                                let controls = controls.clone();
                                let store = store.clone();

                                tokio::spawn(async move {
                                    let mut prefix = [0u8; 4];
                                    if stream.read_exact(&mut prefix).await.is_err() {
                                        return;
                                    }
                                    let len = u32::from_le_bytes(prefix) as usize;
                                    if len == 0 || len > 16 * 1024 * 1024 {
                                        return;
                                    }
                                    let mut payload = vec![0u8; len];
                                    if stream.read_exact(&mut payload).await.is_err() {
                                        return;
                                    }

                                    match serde_json::from_slice::<ClientRequest>(&payload) {
                                        Ok(req) => {
                                            if !matches!(&req, ClientRequest::GetDetailedQueue) { eprintln!("received: {req:?}"); }
                                            let should_save = matches!(&req, ClientRequest::AddTask { .. } | ClientRequest::Pause { .. } | ClientRequest::Resume { .. } | ClientRequest::Cancel { .. } | ClientRequest::RemoveTask { .. } | ClientRequest::SetLimit { .. });
                                            let reply = match req {
                                                ClientRequest::AddTask { url, mirrors, connections, extract, category, not_before } => {
                                                    if !(1..=32).contains(&connections) || !mirrors.is_empty() || extract || category.is_some() || not_before.is_some() {
                                                        DaemonEvent::TaskFailed { task_id: url, error: "connections must be 1–32; mirrors, extraction, categories and scheduling are not yet supported".into() }
                                                    } else if !matches!(reqwest::Url::parse(&url), Ok(ref u) if matches!(u.scheme(), "http" | "https")) {
                                                        DaemonEvent::TaskFailed { task_id: url, error: "only valid HTTP and HTTPS URLs are supported".into() }
                                                    } else {
                                                        let mut guard = tasks.lock().await;
                                                        if guard.get(&url).is_some_and(|state| !state.state.starts_with("failed:") && state.state != "canceled") {
                                                            DaemonEvent::TaskFailed { task_id: url, error: "task already exists".into() }
                                                        } else {
                                                            guard.insert(url.clone(), DownloadInfo { url: url.clone(), state: "downloading".into(), filename: None, downloaded: 0, total: None, speed_bps: 0, final_path: None, resume_supported: false, partial_name: None, requested_connections: Some(connections), active_connections: None, segment_count: None, connection_note: None });
                                                            let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(super::control::TransferControl::Running);
                                                            controls.lock().await.insert(url.clone(), cancel_tx);
                                                            drop(guard);
                                                            let task_id = url.clone();
                                                            let task_states = tasks.clone();
                                                            let task_limiter = limiter.clone();
                                                            let limiter_for_task_save = limiter.clone();
                                                            let task_store = store.clone();
                                                            let transfer_store = store.clone();
                                                            tokio::spawn(async move {
                                                                let result = super::transfer::download_file(&task_id, connections, task_states.clone(), task_limiter, transfer_store, cancel_rx.clone()).await;
                                                                let mut guard = task_states.lock().await;
                                                                if let Some(task) = guard.get_mut(&task_id) {
                                                                    task.speed_bps = 0; task.active_connections = Some(0);
                                                                    if *cancel_rx.borrow() == super::control::TransferControl::Canceled {
                                                                        task.state = "canceled".into();
                                                                        task.resume_supported = false;
                                                                    } else {
                                                                        match result {
                                                                            Ok(path) => { task.state = "completed".into(); task.final_path = Some(path.to_string_lossy().into_owned()); task.partial_name = None; }
                                                                            Err(e) => { task.state = format!("failed: {e}"); task.resume_supported = false; }
                                                                        }
                                                                    }
                                                                }
                                                                drop(guard);
                                                                if let Err(e) = task_store.save(&task_states, &limiter_for_task_save).await { eprintln!("queue checkpoint failed: {e}"); }
                                                            });
                                                            DaemonEvent::TaskStateChanged { task_id: url, state: "downloading".into() }
                                                        }
                                                    }
                                                }
                                                ClientRequest::Pause { task_id } => super::control::set_paused(&tasks, &controls, task_id, true).await,
                                                ClientRequest::Resume { task_id } => super::control::set_paused(&tasks, &controls, task_id, false).await,
                                                ClientRequest::Cancel { task_id } => super::task_removal::cancel(&tasks, &controls, task_id).await,
                                                ClientRequest::RemoveTask { task_id, delete_file } => {
                                                    super::task_removal::remove(&tasks, &controls, &limiter, task_id, delete_file).await
                                                }
                                                ClientRequest::SetLimit { task_id, kbps } => {
                                                    let exists = match task_id.as_deref() {
                                                        Some(id) => tasks.lock().await.contains_key(id),
                                                        None => true,
                                                    };
                                                    if !exists {
                                                        DaemonEvent::TaskFailed { task_id: task_id.unwrap_or_default(), error: "task not found".into() }
                                                    } else {
                                                        match limiter.set(task_id.as_deref(), kbps).await {
                                                            Ok(()) => DaemonEvent::TaskStateChanged {
                                                                task_id: task_id.unwrap_or_default(),
                                                                state: if kbps == 0 { "speed limit disabled".into() } else { format!("speed limit set to {kbps} kbps") },
                                                            },
                                                            Err(error) => DaemonEvent::TaskFailed { task_id: task_id.unwrap_or_default(), error: error.into() },
                                                        }
                                                    }
                                                }
                                                ClientRequest::VerifyChecksum { task_id, algorithm, expected } => {
                                                    let path = {
                                                        let guard = tasks.lock().await;
                                                        guard.get(&task_id).filter(|task| task.state == "completed").and_then(|task| task.final_path.clone())
                                                    };
                                                    match path {
                                                        None => DaemonEvent::TaskFailed { task_id, error: "completed task or file path not found".into() },
                                                        Some(path) => match super::checksum::verify(path.into(), algorithm.clone(), expected).await {
                                                            Ok(true) => DaemonEvent::ChecksumVerified { task_id, algorithm },
                                                            Ok(false) => DaemonEvent::TaskFailed { task_id, error: "checksum mismatch".into() },
                                                            Err(error) => DaemonEvent::TaskFailed { task_id, error },
                                                        }
                                                    }
                                                }
                                                ClientRequest::GetDetailedQueue => {
                                                    let guard = tasks.lock().await;
                                                    DaemonEvent::DetailedQueue { tasks: guard.values().cloned().collect() }
                                                }
                                                ClientRequest::ListQueue | ClientRequest::GetSnapshot { task_id: None } => {
                                                    let guard = tasks.lock().await;
                                                    DaemonEvent::QueueSnapshot { tasks: guard.iter().map(|(id, state)| format!("{id}: {}", state.state)).collect() }
                                                }
                                                ClientRequest::GetSnapshot { task_id: Some(id) } => {
                                                    let guard = tasks.lock().await;
                                                    match guard.get(&id) {
                                                        Some(state) => DaemonEvent::TaskStateChanged { task_id: id, state: state.state.clone() },
                                                        None => DaemonEvent::TaskFailed { task_id: id, error: "task not found".into() },
                                                    }
                                                }
                                                _ => DaemonEvent::TaskFailed { task_id: String::new(), error: "command is not yet supported".into() },
                                            };
                                            let reply = if should_save && !matches!(&reply, DaemonEvent::TaskFailed { .. }) {
                                                match store.save(&tasks, &limiter).await {
                                                    Ok(()) => reply,
                                                    Err(e) => DaemonEvent::TaskFailed { task_id: String::new(), error: format!("operation applied, but persistent state could not be saved: {e}") },
                                                }
                                            } else { reply };
                                            if let Ok(bytes) = serde_json::to_vec(&reply) {
                                                let len = (bytes.len() as u32).to_le_bytes();
                                                let _ = stream.write_all(&len).await;
                                                let _ = stream.write_all(&bytes).await;
                                                let _ = stream.flush().await;
                                            }
                                        }
                                        Err(e) => eprintln!("parse failed: {e}"),
                                    }
                                });
                            }
                            Err(e) => eprintln!("accept error: {e}"),
                        }
                    }
                }
            }
            checkpoint.abort();
            if let Err(e) = store.save(&tasks, &limiter).await { eprintln!("final queue checkpoint failed: {e}"); }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = endpoint;
            while rx.recv().await.is_some() {}
        }
    }
}

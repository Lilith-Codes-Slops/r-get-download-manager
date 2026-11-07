use downloader_ipc::transport::{EndpointPathResolver, IpcConnectionPool};
use downloader_ipc::{ClientRequest, DaemonEvent};
use std::path::PathBuf;
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
            let tasks = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
            eprintln!("daemon listening on {endpoint}");

            loop {
                tokio::select! {
                    _ = rx.recv() => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut stream, _)) => {
                                let tasks = tasks.clone();
                                eprintln!("client connected");

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
                                            eprintln!("received: {req:?}");
                                            let reply = match req {
                                                ClientRequest::AddTask { url, mirrors, connections, extract, category, not_before } => {
                                                    if connections != 1 || !mirrors.is_empty() || extract || category.is_some() || not_before.is_some() {
                                                        DaemonEvent::TaskFailed { task_id: url, error: "only one connection is supported; mirrors, extraction, categories and scheduling are not yet supported".into() }
                                                    } else if !matches!(reqwest::Url::parse(&url), Ok(ref u) if matches!(u.scheme(), "http" | "https")) {
                                                        DaemonEvent::TaskFailed { task_id: url, error: "only valid HTTP and HTTPS URLs are supported".into() }
                                                    } else {
                                                        let mut guard = tasks.lock().await;
                                                        if guard.get(&url).is_some_and(|state| !state.starts_with("failed:")) {
                                                            DaemonEvent::TaskFailed { task_id: url, error: "task already exists".into() }
                                                        } else {
                                                            guard.insert(url.clone(), "downloading".into());
                                                            drop(guard);
                                                            let task_id = url.clone();
                                                            let task_states = tasks.clone();
                                                            tokio::spawn(async move {
                                                                let state = match download_file(&task_id).await {
                                                                    Ok(()) => "completed".to_string(),
                                                                    Err(e) => format!("failed: {e}"),
                                                                };
                                                                task_states.lock().await.insert(task_id, state);
                                                            });
                                                            DaemonEvent::TaskStateChanged { task_id: url, state: "downloading".into() }
                                                        }
                                                    }
                                                }
                                                ClientRequest::ListQueue | ClientRequest::GetSnapshot { task_id: None } => {
                                                    let guard = tasks.lock().await;
                                                    DaemonEvent::QueueSnapshot { tasks: guard.iter().map(|(id, state)| format!("{id}: {state}")).collect() }
                                                }
                                                ClientRequest::GetSnapshot { task_id: Some(id) } => {
                                                    let guard = tasks.lock().await;
                                                    match guard.get(&id) {
                                                        Some(state) => DaemonEvent::TaskStateChanged { task_id: id, state: state.clone() },
                                                        None => DaemonEvent::TaskFailed { task_id: id, error: "task not found".into() },
                                                    }
                                                }
                                                _ => DaemonEvent::TaskFailed { task_id: String::new(), error: "command is not yet supported".into() },
                                            };
                                            if let Ok(bytes) = serde_json::to_vec(&reply) {
                                                let len = (bytes.len() as u32).to_le_bytes();
                                                let _ = stream.write_all(&len).await;
                                                let _ = stream.write_all(&bytes).await;
                                                let _ = stream.flush().await;
                                                eprintln!("reply sent");
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
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = endpoint;
            while rx.recv().await.is_some() {}
        }
    }
}

/// Minimal single-stream HTTP download into ~/Downloads with live progress.
async fn download_file(url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }

    let total = resp.content_length();
    let filename = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(|cd| {
            cd.split("filename=")
                .nth(1)
                .map(|s| s.split(';').next().unwrap_or("").trim().trim_matches('"').to_string())
        })
        .or_else(|| {
            resp.url()
                .path_segments()
                .and_then(|mut s| s.next_back())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "download.bin".into());

    let filename = safe_filename(&filename)?;
    let dest = dirs_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
        .join(&filename);

    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Stream into a uniquely-created side file. Publishing with hard_link is
    // atomic and fails if the final name already exists, unlike rename on Unix.
    let (temporary, mut file) = create_temporary_file(&dest).await?;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    let mut downloaded: u64 = 0;
    let start = std::time::Instant::now();
    let mut last_print = start;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => { drop(file); let _ = tokio::fs::remove_file(&temporary).await; return Err(e.into()); }
        };
        if let Err(e) = file.write_all(&chunk).await {
            drop(file); let _ = tokio::fs::remove_file(&temporary).await; return Err(e.into());
        }
        downloaded += chunk.len() as u64;

        if last_print.elapsed() >= std::time::Duration::from_millis(500) {
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            let speed = downloaded as f64 / elapsed;
            let pct = total.map(|t| if t > 0 { (downloaded as f64 / t as f64) * 100.0 } else { 0.0 });
            match pct {
                Some(p) => eprintln!(
                    "\r{}  {:>6.1}%  {:>8.1} MB/s  {}/{} MB",
                    filename,
                    p,
                    speed / 1_048_576.0,
                    downloaded / 1_048_576,
                    total.unwrap_or(0) / 1_048_576
                ),
                None => eprintln!(
                    "\r{}  {:>8.1} MB/s  {} MB",
                    filename,
                    speed / 1_048_576.0,
                    downloaded / 1_048_576
                ),
            }
            last_print = std::time::Instant::now();
        }
    }
    if let Err(e) = file.flush().await {
        drop(file); let _ = tokio::fs::remove_file(&temporary).await; return Err(e.into());
    }
    drop(file);
    if let Err(e) = tokio::fs::hard_link(&temporary, &dest).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(e.into());
    }
    // A cleanup failure leaves only a redundant .part file, not a corrupted
    // final file. Never delete the successfully published destination.
    let _ = tokio::fs::remove_file(&temporary).await;
    eprintln!("\nsaved {} bytes to {}", downloaded, dest.display());
    Ok(())
}

async fn create_temporary_file(dest: &std::path::Path) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = dest.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination has no parent"))?;
    for _ in 0..100 {
        let path = parent.join(format!(".rget-{}-{}.part", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not allocate a temporary download file"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
fn safe_filename(name: &str) -> Result<&str, std::io::Error> {
    use std::path::Component;
    let mut components = std::path::Path::new(name).components();
    if name.contains('\\') || !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() || name == "." || name == ".." {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsafe download filename"));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::safe_filename;

    #[test]
    fn rejects_unsafe_names() {
        for name in ["", ".", "..", "../outside", "/tmp/outside", "dir/file", "dir\\file"] {
            assert!(safe_filename(name).is_err(), "accepted {name:?}");
        }
        assert_eq!(safe_filename("file.txt").unwrap(), "file.txt");
    }
}

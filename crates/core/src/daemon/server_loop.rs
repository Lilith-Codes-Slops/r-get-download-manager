use downloader_ipc::transport::{EndpointPathResolver, IpcConnectionPool};
use downloader_ipc::{ClientRequest, DaemonEvent};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
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
            let _ = std::fs::remove_file(&endpoint);
            let listener = match UnixListener::bind(&endpoint) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("failed to bind {endpoint}: {e}");
                    return;
                }
            };
            eprintln!("daemon listening on {endpoint}");

            loop {
                tokio::select! {
                    _ = rx.recv() => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut stream, _)) => {
                                let (_id, _rx) = self.pool.register().await;
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
                                            let reply = match &req {
                                                ClientRequest::AddTask { url, .. } => {
                                                    let url = url.clone();
                                                    let url_for_dl = url.clone();
                                                    tokio::spawn(async move {
                                                        if let Err(e) = download_file(&url_for_dl).await {
                                                            eprintln!("download failed: {e}");
                                                        }
                                                    });
                                                    DaemonEvent::TaskStateChanged {
                                                        task_id: url,
                                                        state: "downloading".into(),
                                                    }
                                                }
                                                ClientRequest::ListQueue => {
                                                    DaemonEvent::QueueSnapshot {
                                                        tasks: vec!["(see ~/Downloads)".into()],
                                                    }
                                                }
                                                _ => DaemonEvent::TaskStateChanged {
                                                    task_id: "n/a".into(),
                                                    state: "ok".into(),
                                                },
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
        .timeout(std::time::Duration::from_secs(300))
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
                .map(|s| s.trim_matches('"').trim().to_string())
        })
        .or_else(|| {
            resp.url()
                .path_segments()
                .and_then(|mut s| s.next_back())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "download.bin".into());

    let dest = dirs_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
        .join(&filename);

    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut file = tokio::fs::File::create(&dest).await?;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    let mut downloaded: u64 = 0;
    let start = std::time::Instant::now();
    let mut last_print = start;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
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
    file.flush().await?;
    eprintln!("\nsaved {} bytes to {}", downloaded, dest.display());
    Ok(())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
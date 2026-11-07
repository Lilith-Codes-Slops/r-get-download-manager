#![forbid(unsafe_code)]

use downloader_core::daemon::ServerLoop;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let pool = downloader_ipc::transport::IpcConnectionPool::new();
    let loop_ = ServerLoop::new(pool);
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    loop_.run(rx).await;
    let _ = tx;
    Ok(())
}

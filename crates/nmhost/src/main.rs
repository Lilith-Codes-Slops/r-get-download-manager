#![forbid(unsafe_code)]

use downloader_ipc::transport::DaemonAutospawner;
use std::path::PathBuf;

mod daemon_socket_relay;
mod stdio_frame_reader;
mod stdio_frame_writer;

#[cfg(target_os = "linux")]
mod linux_manifest_registrar;

#[cfg(target_os = "windows")]
mod windows_registry_registrar;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let daemon = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(if cfg!(windows) { "downloader-core.exe" } else { "downloader-core" })))
        .filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("downloader-core"));
    let spawner = DaemonAutospawner::new(daemon);
    spawner.ensure_running().await?;
    let reader = stdio_frame_reader::StdioFrameReader::new();
    let writer = stdio_frame_writer::StdioFrameWriter::new();
    daemon_socket_relay::DaemonSocketRelay::run(reader, writer).await?;
    Ok(())
}

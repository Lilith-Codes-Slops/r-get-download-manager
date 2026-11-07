use downloader_ipc::transport::DaemonAutospawner;
use std::path::PathBuf;

pub struct IpcBridgeWorker;

impl IpcBridgeWorker {
    pub async fn connect() -> Result<(), Box<dyn std::error::Error>> {
        let spawner = DaemonAutospawner::new(PathBuf::from("downloader-core"));
        spawner.ensure_running().await?;
        Ok(())
    }
}

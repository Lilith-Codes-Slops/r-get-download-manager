use crate::{EndpointPathResolver, IpcError, IpcResult};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

/// Automatically spawns the detached downloader-core daemon if the IPC endpoint is offline.
pub struct DaemonAutospawner {
    binary_path: PathBuf,
}

impl DaemonAutospawner {
    pub fn new(binary_path: PathBuf) -> Self {
        Self { binary_path }
    }

    /// Attempt connection; if it fails, spawn the daemon detached and poll until ready (max 500 ms).
    pub async fn ensure_running(&self) -> IpcResult<()> {
        let endpoint = EndpointPathResolver::resolve()?;
        if self.try_connect(&endpoint).await.is_ok() {
            return Ok(());
        }
        self.spawn_detached()?;
        let mut delay = Duration::from_millis(20);
        let deadline = Duration::from_secs(3);
        let mut elapsed = Duration::ZERO;
        while elapsed < deadline {
            sleep(delay).await;
            elapsed += delay;
            if self.try_connect(&endpoint).await.is_ok() {
                return Ok(());
            }
            delay = (delay * 2).min(Duration::from_millis(100));
        }
        Err(IpcError::DaemonNotReady(
            "daemon did not become ready within 3s".into(),
        ))
    }

    fn spawn_detached(&self) -> IpcResult<()> {
        let mut cmd = Command::new(&self.binary_path);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().map_err(|e| IpcError::SpawnFailed(e.to_string()))?;
        Ok(())
    }

    async fn try_connect(&self, endpoint: &str) -> IpcResult<()> {
        #[cfg(target_os = "linux")]
        {
            let path = std::path::Path::new(endpoint);
            tokio::net::UnixStream::connect(path)
                .await
                .map(|_| ())
                .map_err(IpcError::Io)
        }
        #[cfg(target_os = "windows")]
        {
            tokio::net::windows::named_pipe::ClientOptions::new()
                .open(endpoint)
                .await
                .map(|_| ())
                .map_err(IpcError::Io)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(IpcError::EndpointResolution("unsupported platform".into()))
        }
    }
}
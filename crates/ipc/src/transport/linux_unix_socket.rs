#![cfg(target_os = "linux")]

use crate::{IpcError, IpcResult};
use std::path::Path;
use tokio::net::{UnixListener, UnixStream};

/// Linux Unix Domain Socket transport wrapper.
pub struct LinuxUnixSocket;

impl LinuxUnixSocket {
    /// Bind a UnixListener at the given path, removing any stale socket file first.
    pub async fn bind(path: &Path) -> IpcResult<UnixListener> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(path).map_err(IpcError::Io)?;
        Ok(listener)
    }

    /// Connect a UnixStream to the given path.
    pub async fn connect(path: &Path) -> IpcResult<UnixStream> {
        let stream = UnixStream::connect(path).await.map_err(IpcError::Io)?;
        Ok(stream)
    }
}

#![cfg(target_os = "windows")]

use crate::{IpcError, IpcResult};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions, ClientOptions};

/// Windows Named Pipe transport wrapper.
pub struct WindowsNamedPipe;

impl WindowsNamedPipe {
    /// Create the first-instance named pipe server.
    pub fn create_server(pipe_name: &str) -> IpcResult<NamedPipeServer> {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)
            .map_err(IpcError::Io)?;
        Ok(server)
    }

    /// Connect a client to the named pipe.
    pub async fn connect(pipe_name: &str) -> IpcResult<tokio::net::windows::named_pipe::NamedPipeClient> {
        let client = ClientOptions::new()
            .open(pipe_name)
            .await
            .map_err(IpcError::Io)?;
        Ok(client)
    }
}

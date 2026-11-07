pub mod daemon_autospawner;
pub mod endpoint_path_resolver;
pub mod ipc_connection_pool;

#[cfg(target_os = "linux")]
pub mod linux_unix_socket;

#[cfg(target_os = "windows")]
pub mod windows_named_pipe;

pub use daemon_autospawner::DaemonAutospawner;
pub use endpoint_path_resolver::EndpointPathResolver;
pub use ipc_connection_pool::IpcConnectionPool;

#[cfg(target_os = "linux")]
pub use linux_unix_socket::LinuxUnixSocket;

#[cfg(target_os = "windows")]
pub use windows_named_pipe::WindowsNamedPipe;

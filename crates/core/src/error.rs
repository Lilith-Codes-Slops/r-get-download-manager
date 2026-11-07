use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("db error: {0}")]
    Db(String),
    #[error("ipc error: {0}")]
    Ipc(#[from] downloader_ipc::IpcError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

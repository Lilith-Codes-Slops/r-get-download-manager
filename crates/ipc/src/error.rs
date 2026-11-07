use thiserror::Error;

/// Errors that can occur during IPC framing, serialization, transport, or daemon lifecycle.
#[derive(Error, Debug)]
pub enum IpcError {
    #[error("framing error: {0}")]
    Framing(String),

    #[error("invalid length prefix: {0}")]
    InvalidLengthPrefix(String),

    #[error("payload too large: {size} bytes exceeds maximum of {max}")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("incomplete frame: expected {expected} bytes, received {received}")]
    IncompleteFrame { expected: usize, received: usize },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol version mismatch: expected {expected}, received {received}")]
    VersionMismatch { expected: u32, received: u32 },

    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u32),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("connection timed out")]
    Timeout,

    #[error("daemon spawn failed: {0}")]
    SpawnFailed(String),

    #[error("daemon not ready after spawn: {0}")]
    DaemonNotReady(String),

    #[error("single-instance lock failed: {0}")]
    LockFailed(String),

    #[error("endpoint path resolution failed: {0}")]
    EndpointResolution(String),

    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),

    #[error("native messaging origin rejected: {0}")]
    OriginRejected(String),

    #[error("native messaging binary mode required on Windows")]
    BinaryModeRequired,
}

/// Convenient result alias for IPC operations.
pub type IpcResult<T> = Result<T, IpcError>;

#![forbid(unsafe_code)]

pub mod codec;
pub mod error;
pub mod protocol;
pub mod transport;

pub use codec::{FrameDecoder, FrameEncoder};
pub use error::{IpcError, IpcResult};
pub use protocol::*;
pub use transport::{DaemonAutospawner, EndpointPathResolver, IpcConnectionPool};

/// Current IPC framing protocol version.
/// The first frame of every connection carries this value for negotiation.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum allowed payload size for a single length-prefixed frame (16 MiB).
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;

/// Length prefix size in bytes (little-endian u32).
pub const LENGTH_PREFIX_BYTES: usize = 4;

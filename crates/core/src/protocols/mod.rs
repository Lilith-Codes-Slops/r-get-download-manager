pub mod http;
pub mod protocol_contract;
pub mod protocol_registry;

#[cfg(feature = "torrent")]
pub mod torrent;

pub mod hls_dash;
pub mod ytdlp;

pub use protocol_contract::ProtocolHandler;
pub use protocol_registry::ProtocolRegistry;

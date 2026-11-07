use serde::{Deserialize, Serialize};

/// Protocol version carried in the first frame of every IPC connection.
pub const PROTOCOL_VERSION: u32 = 1;

/// First-frame handshake sent by both client and daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolHandshake {
    pub version: u32,
}

impl ProtocolHandshake {
    pub fn current() -> Self {
        Self {
            version: PROTOCOL_VERSION,
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.version == PROTOCOL_VERSION
    }
}

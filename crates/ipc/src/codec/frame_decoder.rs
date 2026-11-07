use crate::{IpcError, IpcResult, LENGTH_PREFIX_BYTES, MAX_FRAME_PAYLOAD};
use serde::de::DeserializeOwned;
use std::io::Read;

/// Reads and deserializes length-prefixed JSON frames.
pub struct FrameDecoder;

impl FrameDecoder {
    /// Read exactly one frame from a synchronous reader and deserialize the payload.
    pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> IpcResult<T> {
        let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
        match reader.read_exact(&mut prefix) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(IpcError::ConnectionClosed);
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
        let len = u32::from_le_bytes(prefix) as usize;
        if len > MAX_FRAME_PAYLOAD {
            return Err(IpcError::PayloadTooLarge {
                size: len,
                max: MAX_FRAME_PAYLOAD,
            });
        }
        if len == 0 {
            return Err(IpcError::InvalidLengthPrefix("zero-length payload".into()));
        }
        let mut payload = vec![0u8; len];
        match reader.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(IpcError::IncompleteFrame {
                    expected: len,
                    received: 0,
                });
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
        let value = serde_json::from_slice(&payload)?;
        Ok(value)
    }

    /// Decode a complete frame buffer (length prefix already consumed) into a value.
    pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> IpcResult<T> {
        if payload.len() > MAX_FRAME_PAYLOAD {
            return Err(IpcError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_FRAME_PAYLOAD,
            });
        }
        Ok(serde_json::from_slice(payload)?)
    }
}

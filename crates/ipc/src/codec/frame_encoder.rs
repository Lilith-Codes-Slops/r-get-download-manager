use crate::{IpcError, IpcResult, LENGTH_PREFIX_BYTES, MAX_FRAME_PAYLOAD};
use serde::Serialize;
use std::io::Write;

/// Serializes a value into a length-prefixed JSON frame.
pub struct FrameEncoder;

impl FrameEncoder {
    /// Encode a serializable value into a complete frame (4-byte LE length + JSON payload).
    pub fn encode<T: Serialize>(value: &T) -> IpcResult<Vec<u8>> {
        let payload = serde_json::to_vec(value)?;
        if payload.len() > MAX_FRAME_PAYLOAD {
            return Err(IpcError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_FRAME_PAYLOAD,
            });
        }
        let mut frame = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
        let len = payload.len() as u32;
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Write a length-prefixed frame directly to a synchronous writer.
    pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> IpcResult<()> {
        let frame = Self::encode(value)?;
        writer.write_all(&frame)?;
        writer.flush()?;
        Ok(())
    }
}

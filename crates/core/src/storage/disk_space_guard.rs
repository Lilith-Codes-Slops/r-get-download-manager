use fs4::free_space;
use std::path::Path;
use thiserror::Error;

const DEFAULT_SAFETY_BUFFER: u64 = 1 << 30; // 1 GiB

#[derive(Error, Debug)]
pub enum DiskSpaceError {
    #[error(
        "insufficient free space: available {available}, required {required} (buffer {buffer})"
    )]
    InsufficientSpace {
        available: u64,
        required: u64,
        buffer: u64,
    },
    #[error("failed to query free space: {0}")]
    QueryFailed(#[from] std::io::Error),
}

pub struct DiskSpaceGuard {
    safety_buffer: u64,
}

impl DiskSpaceGuard {
    pub fn new(safety_buffer: u64) -> Self {
        Self { safety_buffer }
    }

    pub fn with_default() -> Self {
        Self::new(DEFAULT_SAFETY_BUFFER)
    }

    /// Returns Ok if allocating `bytes` would leave at least `safety_buffer` free.
    pub fn check(&self, path: &Path, bytes: u64) -> Result<(), DiskSpaceError> {
        let available = free_space(path)?;
        let required = bytes.saturating_add(self.safety_buffer);
        if available < required {
            return Err(DiskSpaceError::InsufficientSpace {
                available,
                required,
                buffer: self.safety_buffer,
            });
        }
        Ok(())
    }
}

impl Default for DiskSpaceGuard {
    fn default() -> Self {
        Self::with_default()
    }
}

#![cfg(target_os = "linux")]

use std::fs::File;
use std::os::unix::fs::FileExt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PwriteError {
    #[error("positional write failed: {0}")]
    Write(#[from] std::io::Error),
}

/// Lockless pwrite64 via FileExt::write_all_at on a shared &File.
pub struct PwriteLinux;

impl PwriteLinux {
    /// Write the entire buffer at the exact offset. Safe to call concurrently
    /// from multiple threads on the same File because pwrite64 does not move the cursor.
    pub fn write_all_at(file: &File, buf: &[u8], offset: u64) -> Result<(), PwriteError> {
        file.write_all_at(buf, offset)?;
        Ok(())
    }
}

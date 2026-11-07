#![cfg(target_os = "windows")]

use std::fs::File;
use std::os::windows::fs::FileExt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SeekWriteError {
    #[error("seek_write failed: {0}")]
    Write(#[from] std::io::Error),
    #[error("try_clone failed: {0}")]
    Clone(std::io::Error),
}

/// Positional writes on Windows. Each worker must hold its own cloned handle
/// because seek_write serializes on the HANDLE cursor.
pub struct SeekWriteWindows;

impl SeekWriteWindows {
    /// Clone the file handle for exclusive use by one worker.
    pub fn clone_for_worker(file: &File) -> Result<File, SeekWriteError> {
        file.try_clone().map_err(SeekWriteError::Clone)
    }

    /// Write the buffer at the given offset using the worker-private handle.
    pub fn write_at(file: &File, buf: &[u8], offset: u64) -> Result<usize, SeekWriteError> {
        file.seek_write(buf, offset).map_err(SeekWriteError::Write)
    }
}

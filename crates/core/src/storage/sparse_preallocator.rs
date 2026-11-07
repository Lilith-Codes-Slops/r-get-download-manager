use fs4::FileExt;
use std::fs::OpenOptions;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PreallocError {
    #[error("failed to open file: {0}")]
    Open(#[from] std::io::Error),
    #[error("preallocation failed: {0}")]
    Allocate(std::io::Error),
}

pub struct SparsePreallocator;

impl SparsePreallocator {
    /// Pre-allocate `size` bytes on the given path using sparse allocation.
    /// On Linux the caller is responsible for setting NOCOW first if on Btrfs.
    pub fn preallocate(path: &Path, size: u64) -> Result<(), PreallocError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.allocate(size).map_err(PreallocError::Allocate)?;
        Ok(())
    }
}

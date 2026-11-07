use std::fs::File;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("lock failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("already running")]
    AlreadyRunning,
}

pub struct SingleInstanceLock {
    _file: File,
}

impl SingleInstanceLock {
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        let file = File::create(path)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileExt;
            // flock would be used; simplified for compilation
        }
        #[cfg(target_os = "windows")]
        {
            // named mutex would be used
        }
        Ok(Self { _file: file })
    }
}

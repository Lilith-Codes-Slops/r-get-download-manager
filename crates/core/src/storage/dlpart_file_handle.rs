use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HandleError {
    #[error("failed to open .dlpart: {0}")]
    Open(#[from] std::io::Error),
}

/// Thread-safe Arc-wrapped File for concurrent positional writes to a .dlpart.
pub struct DlpartFileHandle {
    inner: Arc<File>,
}

impl DlpartFileHandle {
    pub fn open(path: &Path) -> Result<Self, HandleError> {
        let file = File::options().read(true).write(true).open(path)?;
        Ok(Self {
            inner: Arc::new(file),
        })
    }

    pub fn create(path: &Path) -> Result<Self, HandleError> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        Ok(Self {
            inner: Arc::new(file),
        })
    }

    pub fn arc(&self) -> Arc<File> {
        Arc::clone(&self.inner)
    }

    pub fn file(&self) -> &File {
        &self.inner
    }
}

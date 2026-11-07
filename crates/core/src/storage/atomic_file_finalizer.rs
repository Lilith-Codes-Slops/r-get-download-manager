use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FinalizeError {
    #[error("flush failed: {0}")]
    Flush(std::io::Error),
    #[error("rename failed: {0}")]
    Rename(#[from] std::io::Error),
}

pub struct AtomicFileFinalizer;

impl AtomicFileFinalizer {
    /// Flush any pending data and atomically rename .dlpart to the final name.
    pub fn finalize(dlpart: &Path, final_path: &Path) -> Result<(), FinalizeError> {
        if let Ok(file) = fs::File::open(dlpart) {
            file.sync_all().map_err(FinalizeError::Flush)?;
        }
        fs::rename(dlpart, final_path)?;
        // Best-effort removal of sidecar
        let meta = dlpart.with_extension("dlpart.meta");
        let _ = fs::remove_file(meta);
        Ok(())
    }
}

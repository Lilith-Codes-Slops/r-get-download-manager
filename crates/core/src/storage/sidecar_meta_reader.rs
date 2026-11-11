use super::sidecar_meta_writer::SidecarMeta;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SidecarReadError {
    #[error("deserialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct SidecarMetaReader;

impl SidecarMetaReader {
    pub fn read(path: &Path) -> Result<SidecarMeta, SidecarReadError> {
        let mut file = File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let meta = serde_json::from_slice(&buf)?;
        Ok(meta)
    }
}

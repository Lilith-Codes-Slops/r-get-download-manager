use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SidecarWriteError {
    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarMeta {
    pub version: u32,
    pub monotonic_seq_no: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub total_size: Option<u64>,
    pub segments: Vec<SidecarSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarSegment {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

pub struct SidecarMetaWriter;

impl SidecarMetaWriter {
    /// Write and fsync the sidecar next to the .dlpart file.
    pub fn write(path: &Path, meta: &SidecarMeta) -> Result<(), SidecarWriteError> {
        let mut file = File::create(path)?;
        let json = serde_json::to_vec_pretty(meta)?;
        file.write_all(&json)?;
        file.sync_all()?;
        Ok(())
    }
}

use reqwest::header::{HeaderMap, IF_RANGE};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EtagError {
    #[error("weak etag rejected")]
    WeakEtag,
    #[error("file changed (200 instead of 206)")]
    FileChanged,
}

pub struct EtagRangeGuard;

impl EtagRangeGuard {
    pub fn build_if_range(etag: Option<&str>, last_modified: Option<&str>) -> Option<String> {
        if let Some(e) = etag {
            if e.starts_with("W/") {
                return None;
            }
            return Some(e.to_string());
        }
        last_modified.map(|s| s.to_string())
    }

    pub fn validate_response(
        status: reqwest::StatusCode,
        expected_partial: bool,
    ) -> Result<(), EtagError> {
        if expected_partial && status == reqwest::StatusCode::OK {
            return Err(EtagError::FileChanged);
        }
        Ok(())
    }
}

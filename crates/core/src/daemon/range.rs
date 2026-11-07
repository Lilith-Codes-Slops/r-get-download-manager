//! Conservative range resumption: require a strong validator and exact ranges.
use reqwest::{header, Response, StatusCode};

#[derive(Clone, Debug)]
pub(super) struct ResumeInfo {
    pub etag: String,
    pub total: u64,
    pub url: reqwest::Url,
}

pub(super) fn detect(response: &Response) -> Option<ResumeInfo> {
    let headers = response.headers();
    let ranges = headers.get(header::ACCEPT_RANGES)?.to_str().ok()?;
    if !ranges.split(',').any(|s| s.trim().eq_ignore_ascii_case("bytes")) { return None; }
    let etag = headers.get(header::ETAG)?.to_str().ok()?;
    if etag.starts_with("W/") || !etag.starts_with('"') || !etag.ends_with('"') || etag.len() < 2 { return None; }
    if let Some(encoding) = headers.get(header::CONTENT_ENCODING) {
        if !encoding.to_str().ok()?.eq_ignore_ascii_case("identity") { return None; }
    }
    Some(ResumeInfo { etag: etag.into(), total: response.content_length()?, url: response.url().clone() })
}

pub(super) fn validate(response: &Response, resume: &ResumeInfo, start: u64) -> Result<(), String> {
    if response.url() != &resume.url { return Err("resume redirected to a different resource".into()); }
    let etag = response.headers().get(header::ETAG).and_then(|v| v.to_str().ok());
    if etag != Some(resume.etag.as_str()) { return Err("resume validator changed or was omitted".into()); }
    if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING) {
        if !encoding.to_str().map(|s| s.eq_ignore_ascii_case("identity")).unwrap_or(false) {
            return Err("resumed response is encoded".into());
        }
    }
    if start == 0 {
        if response.status() != StatusCode::OK || response.content_length() != Some(resume.total) {
            return Err("server returned an invalid full response during resume".into());
        }
    } else {
        if response.status() != StatusCode::PARTIAL_CONTENT { return Err("server did not honor the resume range".into()); }
        let expected = format!("bytes {}-{}/{}", start, resume.total - 1, resume.total);
        let actual = response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok());
        if actual != Some(expected.as_str()) || response.content_length() != Some(resume.total - start) {
            return Err("server returned an invalid Content-Range or length".into());
        }
    }
    Ok(())
}

/// Validate an exact bounded range before any worker writes its body.
#[cfg(target_os = "linux")]
pub(super) fn validate_bounded(response: &Response, resume: &ResumeInfo, start: u64, end: u64) -> Result<(), String> {
    if start >= end || end > resume.total { return Err("invalid split range".into()); }
    if resume.etag.starts_with("W/") || !resume.etag.starts_with('"') || !resume.etag.ends_with('"') || resume.etag.len() < 2 {
        return Err("split requires a strong ETag".into());
    }
    if response.status() != StatusCode::PARTIAL_CONTENT || response.url() != &resume.url {
        return Err("server did not honor the split range or redirected to a different resource".into());
    }
    let etag = response.headers().get(header::ETAG).and_then(|value| value.to_str().ok());
    if etag != Some(resume.etag.as_str()) { return Err("split validator changed or was omitted".into()); }
    if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING) {
        if !encoding.to_str().map(|value| value.eq_ignore_ascii_case("identity")).unwrap_or(false) {
            return Err("split response is encoded".into());
        }
    }
    let expected = format!("bytes {}-{}/{}", start, end - 1, resume.total);
    let actual = response.headers().get(header::CONTENT_RANGE).and_then(|value| value.to_str().ok());
    if actual != Some(expected.as_str()) || response.content_length() != Some(end - start) {
        return Err("server returned an invalid split Content-Range or length".into());
    }
    Ok(())
}

/// A complete partial can only be published after revalidating the origin.
pub(super) fn validate_probe(response: &Response, resume: &ResumeInfo) -> Result<(), String> {
    if response.status() != StatusCode::PARTIAL_CONTENT || response.url() != &resume.url {
        return Err("server did not honor the validation probe".into());
    }
    let etag = response.headers().get(header::ETAG).and_then(|v| v.to_str().ok());
    if etag != Some(resume.etag.as_str()) { return Err("resume validator changed or was omitted".into()); }
    if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING) {
        if !encoding.to_str().map(|s| s.eq_ignore_ascii_case("identity")).unwrap_or(false) {
            return Err("validation response is encoded".into());
        }
    }
    let expected = format!("bytes 0-0/{}", resume.total);
    let actual = response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok());
    if actual != Some(expected.as_str()) || response.content_length() != Some(1) {
        return Err("invalid validation probe range".into());
    }
    Ok(())
}

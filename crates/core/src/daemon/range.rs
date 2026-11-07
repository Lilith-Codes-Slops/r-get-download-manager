//! Conservative range resumption: require a strong validator and exact ranges.
use reqwest::{header, Response, StatusCode};

#[derive(Clone, Debug)]
pub(super) struct ResumeInfo {
    pub etag: String,
    pub total: u64,
    pub url: reqwest::Url,
}

/// Reject weak, malformed, or ambiguous entity tags rather than trusting a
/// server or checkpoint that cannot identify one exact representation.
pub(super) fn strong_etag(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'
        && bytes[1..bytes.len() - 1].iter().all(|&b| b == 0x21 || (0x23..=0x7e).contains(&b) || b >= 0x80)
}

fn one_header<'a>(headers: &'a header::HeaderMap, name: header::HeaderName) -> Result<Option<&'a str>, String> {
    let values = headers.get_all(name);
    let mut iter = values.iter();
    let Some(first) = iter.next() else { return Ok(None) };
    if iter.next().is_some() { return Err("duplicate response header".into()); }
    Ok(Some(first.to_str().map_err(|_| "invalid response header")?))
}

fn valid_encoding(response: &Response) -> Result<(), String> {
    if let Some(encoding) = one_header(response.headers(), header::CONTENT_ENCODING)? {
        if !encoding.eq_ignore_ascii_case("identity") { return Err("range response is encoded".into()); }
    }
    if let Some(kind) = one_header(response.headers(), header::CONTENT_TYPE)? {
        if kind.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("multipart/byteranges") {
            return Err("multipart range responses are not supported".into());
        }
    }
    Ok(())
}

fn validate_identity(response: &Response, resume: &ResumeInfo) -> Result<(), String> {
    if !strong_etag(&resume.etag) { return Err("range requires a strong ETag".into()); }
    if response.url() != &resume.url { return Err("range redirected to a different resource".into()); }
    if one_header(response.headers(), header::ETAG)? != Some(resume.etag.as_str()) {
        return Err("range validator changed or was omitted".into());
    }
    valid_encoding(response)
}

pub(super) fn detect(response: &Response) -> Option<ResumeInfo> {
    let headers = response.headers();
    let ranges = one_header(headers, header::ACCEPT_RANGES).ok()??;
    if !ranges.split(',').any(|s| s.trim().eq_ignore_ascii_case("bytes")) { return None; }
    let etag = one_header(headers, header::ETAG).ok()??;
    if !strong_etag(etag) || valid_encoding(response).is_err() { return None; }
    Some(ResumeInfo { etag: etag.into(), total: response.content_length()?, url: response.url().clone() })
}

pub(super) fn validate(response: &Response, resume: &ResumeInfo, start: u64) -> Result<(), String> {
    validate_identity(response, resume)?;
    if start > resume.total || (start > 0 && start == resume.total) { return Err("invalid resume offset".into()); }
    if start == 0 {
        if response.status() != StatusCode::OK || response.content_length() != Some(resume.total) {
            return Err("server returned an invalid full response during resume".into());
        }
    } else {
        if response.status() != StatusCode::PARTIAL_CONTENT { return Err("server did not honor the resume range".into()); }
        let expected = format!("bytes {}-{}/{}", start, resume.total - 1, resume.total);
        let actual = one_header(response.headers(), header::CONTENT_RANGE)?;
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
    validate_identity(response, resume)?;
    if response.status() != StatusCode::PARTIAL_CONTENT { return Err("server did not honor the split range".into()); }
    let expected = format!("bytes {}-{}/{}", start, end - 1, resume.total);
    let actual = one_header(response.headers(), header::CONTENT_RANGE)?;
    let length = one_header(response.headers(), header::CONTENT_LENGTH)?
        .and_then(|value| value.parse::<u64>().ok());
    if actual != Some(expected.as_str()) || length != Some(end - start) || response.content_length() != Some(end - start) {
        return Err("server returned an invalid split Content-Range or length".into());
    }
    Ok(())
}

/// A complete partial can only be published after revalidating the origin.
pub(super) fn validate_probe(response: &Response, resume: &ResumeInfo) -> Result<(), String> {
    if resume.total == 0 || response.status() != StatusCode::PARTIAL_CONTENT { return Err("server did not honor the validation probe".into()); }
    validate_identity(response, resume)?;
    let expected = format!("bytes 0-0/{}", resume.total);
    let actual = one_header(response.headers(), header::CONTENT_RANGE)?;
    if actual != Some(expected.as_str()) || response.content_length() != Some(1) {
        return Err("invalid validation probe range".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::strong_etag;
    #[test]
    fn accepts_only_unambiguous_strong_entity_tags() {
        for tag in ["\"\"", "\"v1\"", "\"with!#chars\""] { assert!(strong_etag(tag), "{tag:?}"); }
        for tag in ["W/\"v1\"", "v1", "\"a\"b\"", "\"a b\"", "\"a\nb\""] { assert!(!strong_etag(tag), "{tag:?}"); }
    }
}

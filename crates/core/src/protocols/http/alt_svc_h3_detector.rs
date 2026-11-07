use reqwest::header::HeaderMap;

pub struct AltSvcH3Detector;

impl AltSvcH3Detector {
    pub fn detects_h3(headers: &HeaderMap) -> bool {
        headers
            .get("alt-svc")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.contains("h3"))
            .unwrap_or(false)
    }
}

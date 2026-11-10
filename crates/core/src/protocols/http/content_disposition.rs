use reqwest::header::HeaderMap;

pub struct ContentDisposition;

impl ContentDisposition {
    pub fn extract_filename(headers: &HeaderMap, url: &str) -> String {
        if let Some(cd) = headers.get("content-disposition") {
            if let Ok(s) = cd.to_str() {
                if let Some(name) = Self::parse_filename(s) {
                    return name;
                }
            }
        }
        url.split('/').last().unwrap_or("download").to_string()
    }

    fn parse_filename(cd: &str) -> Option<String> {
        cd.split(';')
            .map(|p| p.trim())
            .find_map(|p| {
                p.strip_prefix("filename=")
                    .or_else(|| p.strip_prefix("filename*="))
            })
            .map(|s| s.trim_matches('"').to_string())
    }
}

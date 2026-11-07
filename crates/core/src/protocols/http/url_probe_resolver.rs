use reqwest::{Client, StatusCode};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProbeError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("unexpected status: {0}")]
    Status(StatusCode),
}

pub struct UrlProbeResolver;

impl UrlProbeResolver {
    pub async fn probe(client: &Client, url: &str) -> Result<(Option<u64>, bool), ProbeError> {
        let resp = client.head(url).send().await?;
        if !resp.status().is_success() && resp.status() != StatusCode::PARTIAL_CONTENT {
            return Err(ProbeError::Status(resp.status()));
        }
        let len = resp.content_length();
        let accept_ranges = resp
            .headers()
            .get("accept-ranges")
            .map(|v| v.to_str().unwrap_or("").contains("bytes"))
            .unwrap_or(false);
        Ok((len, accept_ranges))
    }
}

use reqwest::Client;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SwapError {
    #[error("overlap mismatch")]
    OverlapMismatch,
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
}

pub struct PresignedUrlSwapper;

impl PresignedUrlSwapper {
    pub async fn verify_overlap(
        client: &Client,
        new_url: &str,
        offset: u64,
        expected: &[u8],
    ) -> Result<bool, SwapError> {
        let end = offset + expected.len() as u64 - 1;
        let range = format!("bytes={}-{}", offset, end);
        let resp = client.get(new_url).header("Range", range).send().await?;
        let bytes = resp.bytes().await?;
        Ok(bytes.as_ref() == expected)
    }
}

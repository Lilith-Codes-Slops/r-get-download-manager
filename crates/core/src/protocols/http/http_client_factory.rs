use reqwest::{Client, ClientBuilder};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientFactoryError {
    #[error("failed to build client: {0}")]
    Build(#[from] reqwest::Error),
}

pub struct HttpClientFactory;

impl HttpClientFactory {
    pub fn build() -> Result<Client, ClientFactoryError> {
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(32)
            .http2_adaptive_window(true)
            .use_rustls_tls()
            .build()?;
        Ok(client)
    }
}

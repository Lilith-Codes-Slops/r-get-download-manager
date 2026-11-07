use reqwest::Proxy;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProxyError {
    #[error("invalid proxy url: {0}")]
    Invalid(#[from] reqwest::Error),
}

pub struct ProxyRouter {
    proxy: Option<Proxy>,
}

impl ProxyRouter {
    pub fn new(proxy_url: Option<&str>) -> Result<Self, ProxyError> {
        let proxy = if let Some(url) = proxy_url {
            Some(Proxy::all(url)?)
        } else {
            None
        };
        Ok(Self { proxy })
    }

    pub fn proxy(&self) -> Option<&Proxy> {
        self.proxy.as_ref()
    }
}

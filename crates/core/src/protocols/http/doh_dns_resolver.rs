use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DohError {
    #[error("resolver error: {0}")]
    Resolver(String),
}

pub struct DohDnsResolver {
    resolver: TokioAsyncResolver,
}

impl DohDnsResolver {
    pub async fn new() -> Result<Self, DohError> {
        let config = ResolverConfig::cloudflare();
        let opts = ResolverOpts::default();
        let resolver = TokioAsyncResolver::tokio(config, opts);
        Ok(Self { resolver })
    }

    pub async fn lookup_ip(&self, host: &str) -> Result<Vec<std::net::IpAddr>, DohError> {
        let lookup = self
            .resolver
            .lookup_ip(host)
            .await
            .map_err(|e| DohError::Resolver(e.to_string()))?;
        Ok(lookup.iter().collect())
    }
}

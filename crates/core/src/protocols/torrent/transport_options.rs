//! Capabilities in locked librqbit; availability is not an active setting.
#[derive(Debug, PartialEq, Eq)]
pub struct TransportCapabilities {
    pub tcp_first_connect: Option<bool>,
    pub mse_control: Option<bool>,
    pub bind_device: Option<bool>,
    pub upnp_port_forwarding: Option<bool>,
    pub nat_pmp_or_pcp_control: Option<bool>,
}

pub fn current() -> TransportCapabilities {
    let known = librqbit::version() == "9.1.0";
    TransportCapabilities { tcp_first_connect: known.then_some(true), mse_control: known.then_some(false),
        bind_device: known.then_some(true), upnp_port_forwarding: known.then_some(true),
        nat_pmp_or_pcp_control: known.then_some(false) }
}

pub fn log() {
    tracing::info!(version = librqbit::version(), capabilities = ?current(),
        "torrent transport capabilities; no interface or port mapping override configured");
}

#[cfg(test)]
mod tests {
    #[test]
    fn capabilities_are_version_gated() {
        let c = super::current();
        if librqbit::version() == "9.1.0" {
            assert_eq!(c.tcp_first_connect, Some(true)); assert_eq!(c.mse_control, Some(false));
            assert_eq!(c.bind_device, Some(true)); assert_eq!(c.upnp_port_forwarding, Some(true));
        } else { assert_eq!(c.tcp_first_connect, None); assert_eq!(c.mse_control, None); }
    }
}

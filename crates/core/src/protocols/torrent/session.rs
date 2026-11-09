//! Safe defaults for a download-only librqbit session.

/// Disable upload at the engine level, rather than merely stopping after the
/// download has finished. Local discovery is out of scope for Phase 1.
///
/// This does not start a session or stop it after completion; the daemon
/// integration must do both explicitly.
pub fn download_only_options() -> librqbit::SessionOptions {
    librqbit::SessionOptions {
        disable_upload: true,
        disable_local_service_discovery: true,
        dht: Some(librqbit::DhtSessionConfig {
            persistence: None,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_defaults_forbid_upload_and_local_discovery() {
        let options = super::download_only_options();
        assert!(options.disable_upload);
        assert!(options.disable_local_service_discovery);
        assert!(options.dht.as_ref().is_some_and(|config| config.persistence.is_none()));
        assert!(!options.disable_trackers);
    }
}

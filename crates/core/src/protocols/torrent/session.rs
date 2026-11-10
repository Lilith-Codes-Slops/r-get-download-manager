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
        assert!(options
            .dht
            .as_ref()
            .is_some_and(|config| config.persistence.is_none()));
        assert!(!options.disable_trackers);
    }
    #[test]
    fn discovery_keeps_upload_disabled_and_fastresume_off() {
        let options = super::with_discovery("/tmp/rget-dht-test/dht.json".into());
        assert!(options.disable_upload);
        assert!(!options.fastresume);
        assert!(!options.disable_local_service_discovery);
        assert!(options.listen.is_some());
        assert!(options
            .dht
            .as_ref()
            .is_some_and(|dht| dht.persistence.is_some()));
    }
}

/// Discovery for one task. Each info-hash uses its own cache directory because
/// librqbit 9.1.0 sessions otherwise share a process-wide temp filename.
pub fn with_discovery(cache_file: std::path::PathBuf) -> librqbit::SessionOptions {
    let mut options = download_only_options();
    options.disable_local_service_discovery = false;
    options.listen = Some(librqbit::ListenerOptions::default());
    options.dht = Some(librqbit::DhtSessionConfig {
        persistence: Some(librqbit::dht::DhtPersistenceConfig {
            config_filename: Some(cache_file),
            dump_interval: Some(std::time::Duration::from_secs(10)),
        }),
        ..Default::default()
    });
    options
}

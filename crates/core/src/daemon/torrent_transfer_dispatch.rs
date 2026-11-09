//! Feature-gated torrent ingress: never pass torrent inputs to HTTP recovery.
use super::{bandwidth::BandwidthLimiter, control::{Controls, Tasks}, persistence::Persistence};
use downloader_ipc::DaemonEvent;
use std::{path::Path, sync::Arc};

pub(super) fn is_torrent(source: &str) -> bool {
    source.starts_with("magnet:?") || (!source.contains("://") && Path::new(source).extension()
        .and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("torrent")))
}

pub(super) async fn add(url: String, connections: u8, tasks: Tasks, controls: Controls,
    limiter: Arc<BandwidthLimiter>, store: Arc<Persistence>) -> DaemonEvent {
    #[cfg(feature = "torrent")]
    { super::torrent_transfer::add(url, connections, tasks, controls, limiter, store).await }
    #[cfg(not(feature = "torrent"))]
    {
        let _ = (connections, tasks, controls, limiter, store);
        DaemonEvent::TaskFailed { task_id: url, error: "torrent support is disabled in this build".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::is_torrent;
    #[test]
    fn distinguishes_local_torrents_and_magnets_from_http_urls() {
        assert!(is_torrent("magnet:?xt=urn:btih:abc"));
        assert!(is_torrent("/tmp/file.TORRENT"));
        assert!(!is_torrent("https://example.test/file.torrent"));
        assert!(!is_torrent("file:///tmp/file.torrent"));
        assert!(!is_torrent("/tmp/file.bin"));
    }
}

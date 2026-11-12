//! Classify torrent ingress without treating HTTP URLs as local paths.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentSource {
    Magnet(String),
    MetainfoFile(PathBuf),
    MetainfoUrl(String),
}

/// Classification only: the engine must still validate the magnet or metainfo.
/// Relative paths are interpreted by the daemon, not the CLI; callers should
/// send an absolute path if their working directory differs from the daemon's.
pub fn classify(source: &str) -> Option<TorrentSource> {
    if source.starts_with("magnet:?") {
        return Some(TorrentSource::Magnet(source.to_owned()));
    }
    if source.contains("://") {
        let url = reqwest::Url::parse(source).ok()?;
        if matches!(url.scheme(), "http" | "https")
            && Path::new(url.path())
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
        {
            return Some(TorrentSource::MetainfoUrl(source.to_owned()));
        }
        return None;
    }
    if source.is_empty() {
        return None;
    }
    let path = Path::new(source);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
    {
        return Some(TorrentSource::MetainfoFile(path.to_owned()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_magnet_and_local_torrent_path() {
        assert!(matches!(
            classify("magnet:?xt=urn:btih:abc"),
            Some(TorrentSource::Magnet(_))
        ));
        assert_eq!(
            classify("/tmp/My File.TORRENT"),
            Some(TorrentSource::MetainfoFile("/tmp/My File.TORRENT".into()))
        );
        assert!(matches!(
            classify("relative.torrent"),
            Some(TorrentSource::MetainfoFile(_))
        ));
    }

    #[test]
    fn does_not_confuse_http_or_other_inputs_with_local_files() {
        assert_eq!(
            classify("https://example.test/file.torrent?token=x"),
            Some(TorrentSource::MetainfoUrl(
                "https://example.test/file.torrent?token=x".into()
            ))
        );
        assert_eq!(classify("file:///tmp/file.torrent"), None);
        assert_eq!(classify("magnet:bad"), None);
        assert_eq!(classify(""), None);
        assert_eq!(classify("/tmp/file.bin"), None);
    }
}

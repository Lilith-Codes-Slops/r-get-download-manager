//! Stable IDs and early v1 hash extraction for deduplication.
use super::{metadata::MetadataError, source::TorrentSource};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncReadExt;

const MAX_METAINFO_BYTES: u64 = 8 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub enum PreparedSource {
    Magnet(String),
    Metainfo(Vec<u8>),
}

pub struct Prepared {
    pub source: PreparedSource,
    pub info_hash: String,
}

/// Capture metainfo bytes once; never hash one version and download another if
/// the source file changes between identity lookup and engine initialization.
pub async fn prepare(source: &TorrentSource) -> Result<Prepared, MetadataError> {
    match source {
        TorrentSource::Magnet(uri) => {
            if uri.len() > 64 * 1024 { return Err(MetadataError::Invalid("magnet exceeds 64 KiB".into())); }
            let parsed = reqwest::Url::parse(uri).map_err(|e| MetadataError::Invalid(e.to_string()))?;
            // librqbit expands `so` ranges during parsing; selective downloads
            // are unsupported here, and hostile ranges could exhaust memory.
            if parsed.query_pairs().any(|(key, _)| key == "so") {
                return Err(MetadataError::Invalid("selective file indices are unsupported".into()));
            }
            let magnet = librqbit::Magnet::parse(uri).map_err(|e| MetadataError::Invalid(format!("{e:#}")))?;
            let id = magnet.as_id20().ok_or_else(|| MetadataError::Invalid("v1 info-hash required".into()))?;
            Ok(Prepared { source: PreparedSource::Magnet(uri.clone()), info_hash: id.as_string() })
        }
        TorrentSource::MetainfoFile(path) => {
            let mut options = tokio::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)] { options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
            let file = options.open(path).await?;
            if !file.metadata().await?.is_file() { return Err(MetadataError::Invalid("metainfo must be a regular file".into())); }
            let mut bytes = Vec::new();
            file.take(MAX_METAINFO_BYTES + 1).read_to_end(&mut bytes).await?;
            if bytes.len() as u64 > MAX_METAINFO_BYTES { return Err(MetadataError::Invalid("metainfo exceeds 8 MiB".into())); }
            let hash = librqbit::torrent_from_bytes(&bytes)
                .map_err(|e| MetadataError::Invalid(format!("{e}")))?.info_hash.as_string();
            Ok(Prepared { source: PreparedSource::Metainfo(bytes), info_hash: hash })
        }
    }
}

pub fn new_id() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("torrent-{}-{now}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

pub fn created_at() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_distinct() { assert_ne!(new_id(), new_id()); }

    #[tokio::test]
    async fn magnet_hash_is_normalized_and_invalid_input_is_rejected() {
        let uri = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567";
        let prepared = prepare(&TorrentSource::Magnet(uri.into())).await.unwrap();
        assert_eq!(prepared.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert!(prepare(&TorrentSource::Magnet("magnet:?bad".into())).await.is_err());
        let with_selection = format!("{uri}&so=0-4294967295");
        assert!(prepare(&TorrentSource::Magnet(with_selection)).await.is_err());
    }
}

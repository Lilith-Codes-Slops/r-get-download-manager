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
            if uri.len() > 64 * 1024 {
                return Err(MetadataError::Invalid("magnet exceeds 64 KiB".into()));
            }
            let parsed =
                reqwest::Url::parse(uri).map_err(|e| MetadataError::Invalid(e.to_string()))?;
            // librqbit expands `so` ranges during parsing; selective downloads
            // are unsupported here, and hostile ranges could exhaust memory.
            if parsed.query_pairs().any(|(key, _)| key == "so") {
                return Err(MetadataError::Invalid(
                    "selective file indices are unsupported".into(),
                ));
            }
            let magnet = librqbit::Magnet::parse(uri)
                .map_err(|e| MetadataError::Invalid(format!("{e:#}")))?;
            let id = magnet
                .as_id20()
                .ok_or_else(|| MetadataError::Invalid("v1 info-hash required".into()))?;
            Ok(Prepared {
                source: PreparedSource::Magnet(uri.clone()),
                info_hash: id.as_string(),
            })
        }
        TorrentSource::MetainfoFile(path) => {
            let mut options = tokio::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(path).await?;
            if !file.metadata().await?.is_file() {
                return Err(MetadataError::Invalid(
                    "metainfo must be a regular file".into(),
                ));
            }
            let mut bytes = Vec::new();
            file.take(MAX_METAINFO_BYTES + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() as u64 > MAX_METAINFO_BYTES {
                return Err(MetadataError::Invalid("metainfo exceeds 8 MiB".into()));
            }
            prepare_bytes(bytes)
        }
        TorrentSource::MetainfoUrl(url) => {
            use futures_util::StreamExt;
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .read_timeout(std::time::Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .map_err(|e| MetadataError::Invalid(e.to_string()))?;
            let response = client
                .get(url)
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|e| MetadataError::Invalid(e.to_string()))?
                .error_for_status()
                .map_err(|e| MetadataError::Invalid(e.to_string()))?;
            if !matches!(response.url().scheme(), "http" | "https")
                || response
                    .content_length()
                    .is_some_and(|size| size > MAX_METAINFO_BYTES)
            {
                return Err(MetadataError::Invalid(
                    "invalid or oversized remote metainfo".into(),
                ));
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| MetadataError::Invalid(e.to_string()))?;
                if chunk.len() > (MAX_METAINFO_BYTES as usize).saturating_sub(bytes.len()) {
                    return Err(MetadataError::Invalid("metainfo exceeds 8 MiB".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            prepare_bytes(bytes)
        }
    }
}

fn prepare_bytes(bytes: Vec<u8>) -> Result<Prepared, MetadataError> {
    let hash = librqbit::torrent_from_bytes(&bytes)
        .map_err(|e| MetadataError::Invalid(format!("{e}")))?
        .info_hash
        .as_string();
    Ok(Prepared {
        source: PreparedSource::Metainfo(bytes),
        info_hash: hash,
    })
}

pub fn new_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "torrent-{}-{now}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn created_at() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn remote_metainfo_is_captured_and_hashed_before_admission() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut bytes =
            b"d4:infod6:lengthi3e4:name8:test.bin12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[0; 20]);
        bytes.extend_from_slice(b"ee");
        let expected_hash = prepare_bytes(bytes.clone()).unwrap().info_hash;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/file.torrent?source=1",
            listener.local_addr().unwrap()
        );
        let expected_bytes = bytes.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        });
        let classified = super::super::source::classify(&url).unwrap();
        let prepared = prepare(&classified).await.unwrap();
        assert_eq!(prepared.info_hash, expected_hash);
        assert!(
            matches!(prepared.source, PreparedSource::Metainfo(ref data) if data == &expected_bytes)
        );
        server.await.unwrap();
    }

    #[test]
    fn ids_are_distinct() {
        assert_ne!(new_id(), new_id());
    }

    #[tokio::test]
    async fn magnet_hash_is_normalized_and_invalid_input_is_rejected() {
        let uri = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567";
        let prepared = prepare(&TorrentSource::Magnet(uri.into())).await.unwrap();
        assert_eq!(
            prepared.info_hash,
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert!(prepare(&TorrentSource::Magnet("magnet:?bad".into()))
            .await
            .is_err());
        let with_selection = format!("{uri}&so=0-4294967295");
        assert!(prepare(&TorrentSource::Magnet(with_selection))
            .await
            .is_err());
    }
}

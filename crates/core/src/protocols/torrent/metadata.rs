//! Resolve v1 metadata without starting a payload download.
//!
//! This is an integration building block; the daemon does not yet call it.
use super::source::TorrentSource;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, Session};
use std::{
    path::{Component, Path},
    sync::Arc,
    time::Duration,
};

const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("metadata timeout")]
    Timeout,
    #[error("invalid torrent: {0}")]
    Invalid(String),
    #[error("torrent file I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve metadata using librqbit's list-only mode. No output file is created
/// by this operation. A successful result must still be downloaded, verified,
/// stopped and published by the caller. Never use metainfo names as paths until
/// they pass the single-component check below.
pub async fn resolve(
    session: &Arc<Session>,
    source: &TorrentSource,
) -> Result<ListOnlyResponse, MetadataError> {
    tokio::time::timeout(METADATA_TIMEOUT, resolve_inner(session, source))
        .await
        .map_err(|_| MetadataError::Timeout)?
}

async fn resolve_inner(
    session: &Arc<Session>,
    source: &TorrentSource,
) -> Result<ListOnlyResponse, MetadataError> {
    let prepared = super::task_identity::prepare(source).await?;
    resolve_prepared_inner(session, prepared.source).await
}

pub async fn resolve_prepared(
    session: &Arc<Session>,
    source: super::task_identity::PreparedSource,
) -> Result<ListOnlyResponse, MetadataError> {
    tokio::time::timeout(METADATA_TIMEOUT, resolve_prepared_inner(session, source))
        .await
        .map_err(|_| MetadataError::Timeout)?
}

async fn resolve_prepared_inner(
    session: &Arc<Session>,
    source: super::task_identity::PreparedSource,
) -> Result<ListOnlyResponse, MetadataError> {
    use super::task_identity::PreparedSource;
    let add = match source {
        PreparedSource::Magnet(uri) => AddTorrent::from_url(uri),
        PreparedSource::Metainfo(bytes) => AddTorrent::from_bytes(bytes),
    };
    let opts = AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };
    let result = session
        .add_torrent(add, Some(opts))
        .await
        .map_err(|error| MetadataError::Invalid(format!("{error:#}")))?;
    let response = match result {
        AddTorrentResponse::ListOnly(response) => response,
        _ => {
            return Err(MetadataError::Invalid(
                "torrent is already managed by this session".into(),
            ))
        }
    };
    validate_single_file(&response)?;
    Ok(response)
}

fn validate_single_file(response: &ListOnlyResponse) -> Result<(), MetadataError> {
    let info = response.info.info();
    // A one-entry multifile torrent is still out of scope.
    if info.files.is_some() || info.length.is_none() || response.only_files.is_some() {
        return Err(MetadataError::Invalid(
            "only an unfiltered single-file v1 torrent is supported".into(),
        ));
    }
    let file = response
        .info
        .iter_file_details()
        .next()
        .ok_or_else(|| MetadataError::Invalid("missing file".into()))?;
    let attrs = file.attrs();
    if attrs.symlink || attrs.padding || file.symlink_path.is_some() {
        return Err(MetadataError::Invalid(
            "symlink and padding files are unsupported".into(),
        ));
    }
    let name = response
        .info
        .name()
        .ok_or_else(|| MetadataError::Invalid("missing file name".into()))?;
    if name == ".rget-owner" || !safe_name(&name) {
        return Err(MetadataError::Invalid("unsafe file name".into()));
    }
    Ok(())
}

fn safe_name(name: &str) -> bool {
    if name.chars().any(|c| matches!(c, '/' | '\\' | '\0')) {
        return false;
    }
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_destination() {
        assert!(safe_name("example file.bin"));
        for name in ["", ".", "..", "../file", "/file", "a/b", "a\\b", "x\0y"] {
            assert!(!safe_name(name), "accepted {name:?}");
        }
    }

    #[tokio::test]
    async fn list_only_accepts_single_file_and_rejects_multifile() {
        let root = std::env::temp_dir().join(format!(
            "rget-meta-parse-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let mut options = super::super::session::download_only_options();
        options.dht = None;
        let session = Session::new_with_opts(root.clone(), options).await.unwrap();
        let path = root.join("fixture.torrent");
        let mut single =
            b"d4:infod6:lengthi3e4:name8:test.bin12:piece lengthi16384e6:pieces20:".to_vec();
        single.extend_from_slice(&[0; 20]);
        single.extend_from_slice(b"ee");
        tokio::fs::write(&path, &single).await.unwrap();
        let response = resolve(&session, &TorrentSource::MetainfoFile(path.clone()))
            .await
            .unwrap();
        assert_eq!(response.info.name().as_deref(), Some("test.bin"));
        assert_eq!(response.info.info().length, Some(3));
        let mut multi = b"d4:infod5:filesld6:lengthi3e4:pathl8:test.bineee4:name3:dir12:piece lengthi16384e6:pieces20:".to_vec();
        multi.extend_from_slice(&[0; 20]);
        multi.extend_from_slice(b"ee");
        tokio::fs::write(&path, multi).await.unwrap();
        let error = resolve(&session, &TorrentSource::MetainfoFile(path)).await;
        assert!(
            matches!(error, Err(MetadataError::Invalid(message)) if message.contains("single-file"))
        );
        session.stop().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_large_local_metainfo_before_parsing() {
        let root = std::env::temp_dir().join(format!(
            "rget-meta-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join("large.torrent");
        let f = tokio::fs::File::create(&path).await.unwrap();
        f.set_len(8 * 1024 * 1024 + 1).await.unwrap();
        drop(f);
        let mut options = super::super::session::download_only_options();
        options.dht = None;
        let session = Session::new_with_opts(root.clone(), options).await.unwrap();
        let result = resolve(&session, &TorrentSource::MetainfoFile(path)).await;
        assert!(
            matches!(result, Err(MetadataError::Invalid(message)) if message.contains("8 MiB"))
        );
        session.stop().await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

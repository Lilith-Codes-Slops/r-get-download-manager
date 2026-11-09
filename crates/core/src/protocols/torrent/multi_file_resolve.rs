//! List-only v1 multi-file metadata resolution; no payload storage is opened.
//! This is not yet connected to download ingress.
use super::{metadata::MetadataError, multi_file_metadata, staging_manifest::ValidatedManifest, task_identity::PreparedSource};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, Session};
use std::{sync::Arc, time::Duration};

pub struct ResolvedMultiFile {
    pub response: ListOnlyResponse,
    pub manifest: ValidatedManifest,
}

fn invalid(message: impl Into<String>) -> MetadataError { MetadataError::Invalid(message.into()) }

/// Resolve without requesting payload storage. Local bytes are preflighted
/// before librqbit sees them. The exact returned bytes and both info-hashes
/// are checked again before returning any manifest. Magnet metadata is first
/// validated by librqbit internally, before this function sees its raw bytes;
/// upstream's unchecked length sum remains a limitation for untrusted magnets.
/// Never use this result to allocate staging or start a download until that
/// upstream limitation and the lifecycle integration are addressed.
pub async fn resolve(session: &Arc<Session>, source: PreparedSource, expected_hash: &str) -> Result<ResolvedMultiFile, MetadataError> {
    if expected_hash.len() != 40 || !expected_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid v1 info-hash"));
    }
    let add = match source {
        PreparedSource::Metainfo(bytes) => {
            if bytes.len() > 8 * 1024 * 1024 { return Err(invalid("metainfo exceeds 8 MiB")); }
            let manifest = multi_file_metadata::preflight_local(&bytes).map_err(|e| invalid(e.to_string()))?;
            if manifest.is_none() { return Err(invalid("multi-file metadata required")); }
            let parsed = librqbit::torrent_from_bytes(&bytes).map_err(|e| invalid(e.to_string()))?;
            if parsed.info_hash.as_string() != expected_hash { return Err(invalid("metainfo info-hash changed")); }
            AddTorrent::from_bytes(bytes)
        }
        PreparedSource::Magnet(uri) => {
            // Reuse the bounded magnet parser and reject selective indices
            // before librqbit can expand potentially hostile `so` ranges.
            let prepared = super::task_identity::prepare(&super::source::TorrentSource::Magnet(uri.clone())).await?;
            if prepared.info_hash != expected_hash { return Err(invalid("magnet info-hash changed")); }
            AddTorrent::from_url(uri)
        },
    };
    let opts = AddTorrentOptions { list_only: true, ..Default::default() };
    let result = tokio::time::timeout(Duration::from_secs(60), session.add_torrent(add, Some(opts))).await
        .map_err(|_| MetadataError::Timeout)?
        .map_err(|e| invalid(format!("{e:#}")))?;
    let response = match result {
        AddTorrentResponse::ListOnly(response) => response,
        _ => return Err(invalid("torrent is already managed by this session")),
    };
    if response.only_files.is_some() { return Err(invalid("filtered multi-file metadata is unsupported")); }
    if response.torrent_bytes.len() > 8 * 1024 * 1024 { return Err(invalid("resolved metainfo exceeds 8 MiB")); }
    if response.info_hash.as_string() != expected_hash { return Err(invalid("resolved info-hash changed")); }
    let parsed = librqbit::torrent_from_bytes(&response.torrent_bytes).map_err(|e| invalid(e.to_string()))?;
    if parsed.info_hash.as_string() != expected_hash { return Err(invalid("resolved bytes info-hash changed")); }
    let manifest = multi_file_metadata::preflight_local(&response.torrent_bytes).map_err(|e| invalid(e.to_string()))?
        .ok_or_else(|| invalid("resolved metadata is not multi-file"))?;
    if response.info.name().as_deref() != Some(manifest.root_name())
        || response.info.lengths().total_length() != manifest.total_bytes() {
        return Err(invalid("resolved engine metadata differs from manifest"));
    }
    let details: Vec<_> = response.info.iter_file_details().collect();
    if details.len() != manifest.files().len() || details.iter().zip(manifest.files()).any(|(file, expected)|
        file.filename.to_vec() != expected.components || file.len != expected.length) {
        return Err(invalid("resolved engine files differ from manifest"));
    }
    Ok(ResolvedMultiFile { response, manifest })
}

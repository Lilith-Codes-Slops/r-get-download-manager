//! Fail-closed extraction of a v1 multi-file manifest before storage starts.
//! Multi-file transfers remain disabled until durable staging is integrated.
use super::staging_manifest::{ManifestFile, ValidatedManifest};
use downloader_ipc::TorrentFile;
use librqbit::TorrentMetaV1Info;
use std::io;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Validate raw info before calling librqbit's validator. Its locked length
/// calculation uses an unchecked sum, so avoid passing overflowing lengths or
/// a zero piece length into it. Require the engine's decoded paths to match the
/// original UTF-8 byte components exactly: no lossy or alternate decoding.
pub fn extract<B: AsRef<[u8]> + Clone>(
    info: &TorrentMetaV1Info<B>,
) -> io::Result<ValidatedManifest> {
    if info.private {
        return Err(invalid("private torrents are unsupported"));
    }
    if info.length.is_some() {
        return Err(invalid(
            "multi-file info also contains a single-file length",
        ));
    }
    let files = info
        .files
        .as_ref()
        .ok_or_else(|| invalid("multi-file list missing"))?;
    if files.is_empty() || files.len() > 100_000 {
        return Err(invalid("invalid torrent file count"));
    }
    let raw_name = info
        .name
        .as_ref()
        .ok_or_else(|| invalid("torrent root name missing"))?
        .as_ref();
    let root = std::str::from_utf8(raw_name)
        .map_err(|_| invalid("torrent root name is not UTF-8"))?
        .to_owned();
    let mut total = 0u64;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        if file.symlink_path.is_some()
            || file
                .attr
                .as_ref()
                .is_some_and(|attrs| attrs.as_ref().iter().any(|b| matches!(*b, b'l' | b'p')))
        {
            return Err(invalid("symlink and padding files are unsupported"));
        }
        if file.path.len() > 128 {
            return Err(invalid("too many path components"));
        }
        let mut components = Vec::with_capacity(file.path.len());
        for part in &file.path {
            components.push(
                std::str::from_utf8(part.as_ref())
                    .map_err(|_| invalid("torrent path is not UTF-8"))?
                    .to_owned(),
            );
        }
        total = total
            .checked_add(file.length)
            .ok_or_else(|| invalid("torrent length overflow"))?;
        entries.push(ManifestFile {
            components,
            length: file.length,
        });
    }
    let manifest = ValidatedManifest::new(root, entries, total)?;
    if total == 0 || info.piece_length == 0 {
        return Err(invalid(
            "zero total length or piece length unsupported by engine",
        ));
    }
    let count = total.div_ceil(u64::from(info.piece_length));
    if count > u64::from(u32::MAX)
        || count.checked_mul(20) != Some(info.pieces.as_ref().len() as u64)
    {
        return Err(invalid("torrent piece hashes do not match its length"));
    }
    let validated = info
        .clone()
        .validate()
        .map_err(|_| invalid("engine rejected torrent metadata"))?;
    if validated.name().as_deref() != Some(manifest.root_name())
        || validated.lengths().total_length() != total
    {
        return Err(invalid("engine root name or length differs from manifest"));
    }
    for (engine, expected) in validated.iter_file_details().zip(manifest.files()) {
        if engine.filename.to_vec() != expected.components || engine.len != expected.length {
            return Err(invalid("engine decoded path differs from manifest"));
        }
    }
    Ok(manifest)
}

/// Inspect original local metainfo before any torrent stage is allocated.
/// `None` means the metadata is not multi-file and follows the existing path.
pub fn preflight_local(bytes: &[u8]) -> io::Result<Option<ValidatedManifest>> {
    let parsed = librqbit::torrent_from_bytes(bytes).map_err(|_| invalid("invalid metainfo"))?;
    let info = &parsed.info.data;
    if info.files.is_none() {
        return Ok(None);
    }
    extract(info).map(Some)
}

/// Records are stored only; selection and priority are not applied.
pub fn file_records(manifest: &ValidatedManifest) -> Vec<TorrentFile> {
    manifest
        .files()
        .iter()
        .map(|file| TorrentFile {
            path: file.components.join("/"),
            length: file.length,
            priority: super::file_priorities::NORMAL,
        })
        .collect()
}

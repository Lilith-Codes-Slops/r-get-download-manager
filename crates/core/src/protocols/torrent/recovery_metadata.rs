//! Bounded v1 info-dictionary evidence for future crash-time verification.
//! This stores no top-level tracker or webseed declarations. The info dictionary
//! itself may contain unknown fields and must still be treated as sensitive.
use super::{multi_file_metadata, staging_manifest::ValidatedManifest};
use std::io;

const MAX_INFO_BYTES: usize = 8 * 1024 * 1024;
fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// Extract exact bencoded info bytes, after checking both their hash and the
/// independently validated manifest. No paths or files are opened here.
pub fn from_metainfo(bytes: &[u8], expected_hash: &str, manifest: &ValidatedManifest) -> io::Result<Vec<u8>> {
    if bytes.len() > MAX_INFO_BYTES { return Err(invalid("metainfo too large")); }
    let parsed = librqbit::torrent_from_bytes(bytes).map_err(|_| invalid("invalid metainfo"))?;
    if parsed.info_hash.as_string() != expected_hash { return Err(invalid("metainfo info-hash mismatch")); }
    if multi_file_metadata::preflight_local(bytes)?.as_ref() != Some(manifest) { return Err(invalid("metainfo manifest mismatch")); }
    Ok(parsed.info.raw_bytes.as_ref().to_vec())
}

/// Re-parse stored bytes and verify they still match the expected v1 hash and
/// manifest. This does not check payload content or authorize crash recovery.
pub fn validate_info(info: &[u8], expected_hash: &str, manifest: &ValidatedManifest) -> io::Result<()> {
    if info.len() > MAX_INFO_BYTES - 8 { return Err(invalid("info dictionary too large")); }
    let mut wrapped = Vec::with_capacity(info.len() + 8);
    wrapped.extend_from_slice(b"d4:info"); wrapped.extend_from_slice(info); wrapped.push(b'e');
    let extracted = from_metainfo(&wrapped, expected_hash, manifest)?;
    if extracted != info { return Err(invalid("info dictionary encoding mismatch")); }
    Ok(())
}

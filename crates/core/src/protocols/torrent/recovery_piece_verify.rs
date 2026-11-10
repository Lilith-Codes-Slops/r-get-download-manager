//! Streaming v1 piece verification for an exclusively controlled published tree.
//! This is evidence only; it is not wired to automatic recovery or downloading.
use super::{recovery_metadata, staging_manifest::ValidatedManifest, staging_verify};
use librqbit_sha1_wrapper::{ISha1, Sha1};
use nix::{
    fcntl::{openat, OFlag},
    sys::stat::Mode,
    unistd::geteuid,
};
use std::{
    fs::File,
    io::{self, Read},
    os::unix::fs::MetadataExt,
};

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn open_file(root: &File, parts: &[String], device: u64, length: u64) -> io::Result<File> {
    let mut parent = root.try_clone()?;
    let (name, dirs) = parts.split_last().ok_or_else(|| invalid("empty path"))?;
    for dir in dirs {
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        let fd = openat(&parent, dir.as_str(), flags, Mode::empty())
            .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
        let child = File::from(fd);
        let meta = child.metadata()?;
        if !meta.is_dir() || meta.dev() != device || meta.uid() != geteuid().as_raw() {
            return Err(invalid("directory identity or owner mismatch"));
        }
        parent = child;
    }
    let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
    let fd = openat(&parent, name.as_str(), flags, Mode::empty())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let file = File::from(fd);
    check_file(&file, device, length)?;
    Ok(file)
}
fn check_file(file: &File, device: u64, length: u64) -> io::Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.dev() != device
        || meta.uid() != geteuid().as_raw()
        || meta.nlink() != 1
        || meta.len() != length
    {
        return Err(invalid("file type, owner, links or length mismatch"));
    }
    Ok(())
}

/// Check stored info-hash and manifest, exact tree layout, then hash torrent
/// pieces over files in metadata order. Files are opened descriptor-relative,
/// never through symlinks; memory use is bounded. The caller must independently
/// establish directory identity and prevent concurrent modifications throughout
/// and after this function. It neither syncs nor adopts or deletes anything.
pub fn verify(
    root: &File,
    device: u64,
    manifest: &ValidatedManifest,
    info: &[u8],
    expected_hash: &str,
) -> io::Result<()> {
    recovery_metadata::validate_info(info, expected_hash, manifest)?;
    let wrapped = recovery_metadata::wrap_info(info)?;
    let parsed =
        librqbit::torrent_from_bytes(&wrapped).map_err(|_| invalid("invalid stored info"))?;
    let length = u64::from(parsed.info.data.piece_length);
    let hashes = parsed.info.data.pieces.as_ref();
    if length == 0 {
        return Err(invalid("zero piece length"));
    }
    staging_verify::verify_published_readonly(root, manifest, device)?;
    let mut digest = Sha1::new();
    let mut in_piece = 0u64;
    let mut index = 0usize;
    let mut buf = [0u8; 65536];
    for entry in manifest.files() {
        let mut file = open_file(root, &entry.components, device, entry.length)?;
        let mut remaining = entry.length;
        while remaining > 0 {
            let take = remaining.min(length - in_piece).min(buf.len() as u64) as usize;
            file.read_exact(&mut buf[..take])?;
            digest.update(&buf[..take]);
            remaining -= take as u64;
            in_piece += take as u64;
            if in_piece == length {
                let hash = std::mem::replace(&mut digest, Sha1::new()).finish();
                if hashes.get(index..index + 20) != Some(hash.as_slice()) {
                    return Err(invalid("torrent piece hash mismatch"));
                }
                index += 20;
                in_piece = 0;
            }
        }
        check_file(&file, device, entry.length)?;
    }
    if in_piece != 0 {
        let hash = digest.finish();
        if hashes.get(index..index + 20) != Some(hash.as_slice()) {
            return Err(invalid("torrent piece hash mismatch"));
        }
        index += 20;
    }
    if index != hashes.len() {
        return Err(invalid("torrent piece count mismatch"));
    }
    staging_verify::verify_published_readonly(root, manifest, device)?;
    Ok(())
}

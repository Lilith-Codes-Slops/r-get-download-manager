//! Descriptor-relative, fail-closed verification for an owned payload tree.
//! The caller must stop and join the engine first, and prevent modifications
//! through verification and publication. This module is not wired to downloads.
use super::{staging_manifest::ValidatedManifest, staging_tree::{StageArea, StageRecord}};
use nix::{dir::Dir, errno::Errno, fcntl::{openat, OFlag}, sys::stat::Mode, unistd::geteuid};
use std::{collections::{BTreeMap, BTreeSet}, fs::File, io, os::unix::fs::MetadataExt};

enum Node { Directory(BTreeMap<String, Node>), File(u64) }
fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn err(error: Errno) -> io::Error { io::Error::from_raw_os_error(error as i32) }

fn add(nodes: &mut BTreeMap<String, Node>, parts: &[String], length: u64) -> io::Result<()> {
    let (name, rest) = parts.split_first().ok_or_else(|| invalid("empty manifest path"))?;
    if rest.is_empty() {
        if nodes.insert(name.clone(), Node::File(length)).is_some() { return Err(invalid("duplicate manifest file")); }
    } else {
        let node = nodes.entry(name.clone()).or_insert_with(|| Node::Directory(BTreeMap::new()));
        match node { Node::Directory(children) => add(children, rest, length)?, Node::File(_) => return Err(invalid("manifest path conflict")) }
    }
    Ok(())
}

fn opened(parent: &File, name: &str, directory: bool) -> io::Result<File> {
    let mut flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
    if directory { flags |= OFlag::O_DIRECTORY; }
    Ok(File::from(openat(parent, name, flags, Mode::empty()).map_err(err)?))
}

fn walk(dir: &File, expected: &BTreeMap<String, Node>, device: u64) -> io::Result<()> {
    let mut listing = Dir::openat(dir, ".", OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, Mode::empty()).map_err(err)?;
    let mut found = BTreeSet::new();
    for entry in listing.iter() {
        let entry = entry.map_err(err)?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." { continue; }
        let name = std::str::from_utf8(bytes).map_err(|_| invalid("unexpected non-UTF-8 staging entry"))?;
        let node = expected.get(name).ok_or_else(|| invalid("unexpected staging entry"))?;
        if !found.insert(name.to_owned()) { return Err(invalid("duplicate directory entry")); }
        let file = opened(dir, name, matches!(node, Node::Directory(_)))?;
        let meta = file.metadata()?;
        if meta.dev() != device || meta.uid() != geteuid().as_raw() { return Err(invalid("staging entry device or owner mismatch")); }
        match node {
            Node::File(length) => {
                if !meta.is_file() || meta.nlink() != 1 || meta.len() != *length { return Err(invalid("staged file type, links, or length mismatch")); }
                file.sync_all()?;
            }
            Node::Directory(children) => {
                if !meta.is_dir() { return Err(invalid("staged path is not a directory")); }
                walk(&file, children, device)?;
            }
        }
    }
    if found.len() != expected.len() { return Err(invalid("staging tree is missing files or directories")); }
    dir.sync_all()
}

/// Verify exactly the manifest entries, reject extra entries and symlinks,
/// sync every regular file and directory, then sync the owning stage root.
/// This checks sizes and layout, not cryptographic hashes or ownership of a
/// post-publication path. On failure the caller must retain the stage.
pub fn verify_and_sync(area: &StageArea, record: &StageRecord, manifest: &ValidatedManifest) -> io::Result<()> {
    let stage = area.verify(record)?;
    let payload = opened(&stage, "payload", true)?;
    let metadata = payload.metadata()?;
    if metadata.dev() != record.device || metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(invalid("payload root device, owner or permissions mismatch"));
    }
    let mut entries = BTreeMap::new();
    for item in manifest.files() { add(&mut entries, &item.components, item.length)?; }
    walk(&payload, &entries, record.device)?;
    stage.sync_all()
}

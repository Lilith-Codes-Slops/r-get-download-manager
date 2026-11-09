//! Isolated descriptor-relative staging cleanup; not wired to the daemon.
//! Only call with an exact record loaded from the DB after the engine stops.
//! Exclusive control of the private tree is required throughout: this is not
//! secure against a same-user process concurrently replacing entries.
use super::staging_tree::{StageArea, StageRecord};
use nix::{dir::Dir, errno::Errno, fcntl::{openat, AtFlags, OFlag}, sys::stat::{fstatat, Mode}, unistd::{geteuid, unlinkat, UnlinkatFlags}};
use std::{fs::File, io, os::unix::fs::MetadataExt};

const MAX_DEPTH: usize = 128;
const MAX_ENTRIES: usize = 1_000_000;
fn error(e: Errno) -> io::Error { io::Error::from_raw_os_error(e as i32) }
fn invalid(s: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, s) }
fn open(parent: &File, name: &str, directory: bool) -> io::Result<File> {
    let mut flags = OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_PATH;
    if directory { flags = OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_DIRECTORY | OFlag::O_RDONLY; }
    Ok(File::from(openat(parent, name, flags, Mode::empty()).map_err(error)?))
}
fn names(dir: &File) -> io::Result<Vec<String>> {
    let mut list = Dir::openat(dir, ".", OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, Mode::empty()).map_err(error)?;
    let mut out = Vec::new();
    for entry in list.iter() {
        let entry = entry.map_err(error)?; let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." { continue; }
        let name = std::str::from_utf8(raw).map_err(|_| invalid("non-UTF-8 entry in staging"))?;
        super::multi_file_layout::validate_component(name)?;
        if out.len() >= MAX_ENTRIES { return Err(invalid("too many staging entries")); }
        out.push(name.to_owned());
    }
    Ok(out)
}
pub(super) fn unchanged(parent: &File, name: &str, file: &File) -> io::Result<()> {
    let now = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(error)?;
    let opened = file.metadata()?;
    if now.st_dev as u64 != opened.dev() || now.st_ino as u64 != opened.ino() { return Err(invalid("staging entry changed during cleanup")); }
    Ok(())
}
pub(super) fn walk(dir: &File, device: u64, depth: usize, count: &mut usize, delete: bool) -> io::Result<()> {
    if depth > MAX_DEPTH { return Err(invalid("staging tree too deep")); }
    for name in names(dir)? {
        *count = count.checked_add(1).ok_or_else(|| invalid("staging entry count overflow"))?;
        if *count > MAX_ENTRIES { return Err(invalid("too many staging entries")); }
        let entry = open(dir, &name, false)?;
        let metadata = entry.metadata()?;
        if metadata.dev() != device || metadata.uid() != geteuid().as_raw() { return Err(invalid("staging entry owner or device mismatch")); }
        if metadata.is_dir() {
            let child = open(dir, &name, true)?; unchanged(dir, &name, &child)?;
            let child_meta = child.metadata()?;
            if child_meta.dev() != device || child_meta.uid() != geteuid().as_raw() || child_meta.ino() != metadata.ino() {
                return Err(invalid("staging directory identity or owner mismatch"));
            }
            walk(&child, device, depth + 1, count, delete)?;
            if delete { unchanged(dir, &name, &child)?; unlinkat(dir, name.as_str(), UnlinkatFlags::RemoveDir).map_err(error)?; }
        } else if metadata.is_file() {
            if metadata.nlink() != 1 { return Err(invalid("refusing hardlinked staging file")); }
            if delete { unchanged(dir, &name, &entry)?; unlinkat(dir, name.as_str(), UnlinkatFlags::NoRemoveDir).map_err(error)?; }
        } else { return Err(invalid("refusing symlink or special staging entry")); }
    }
    if delete { dir.sync_all()?; }
    Ok(())
}

/// Clean a stopped, exclusively controlled stage. Database membership and
/// policy (including retained verification failures) MUST be checked by caller.
/// Two passes detect unsafe entries before any deletion. On a later error the
/// tree may be partially removed: keep the DB record and report the failure.
pub fn cleanup(area: &StageArea, record: &StageRecord) -> io::Result<()> {
    let stage = area.verify(record)?;
    let entries = names(&stage)?;
    if entries.iter().any(|name| name != ".rget-owner" && name != "payload") { return Err(invalid("unexpected staging root entry")); }
    if entries.iter().any(|name| name == "payload") {
        let payload = open(&stage, "payload", true)?;
        let meta = payload.metadata()?;
        if meta.dev() != record.device || meta.uid() != geteuid().as_raw() || meta.mode() & 0o077 != 0 { return Err(invalid("invalid payload root")); }
        walk(&payload, record.device, 0, &mut 0, false)?;
        walk(&payload, record.device, 0, &mut 0, true)?;
        unchanged(&stage, "payload", &payload)?;
        unlinkat(&stage, "payload", UnlinkatFlags::RemoveDir).map_err(error)?;
        stage.sync_all()?;
    }
    // Re-check marker and stage identity before the final two unlinks.
    let checked = area.verify(record)?;
    unchanged(area.parent(), &record.name, &checked)?;
    unlinkat(&checked, ".rget-owner", UnlinkatFlags::NoRemoveDir).map_err(error)?;
    checked.sync_all()?;
    unchanged(area.parent(), &record.name, &checked)?;
    unlinkat(area.parent(), record.name.as_str(), UnlinkatFlags::RemoveDir).map_err(error)?;
    area.parent().sync_all()
}

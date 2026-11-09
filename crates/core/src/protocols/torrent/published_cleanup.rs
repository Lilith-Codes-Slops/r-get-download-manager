//! Descriptor-relative deletion for an identity-recorded published directory.
//! Not connected to task removal. Exclusive control of the directory is needed;
//! hostile concurrent same-user changes are outside this primitive's guarantee.
use super::{multi_file_layout::validate_component, staging_cleanup};
use nix::{fcntl::{openat, OFlag}, sys::stat::Mode, unistd::{geteuid, unlinkat, UnlinkatFlags}};
use std::{fs::File, io, os::unix::fs::MetadataExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedIdentity {
    pub name: String, pub root_device: u64, pub root_inode: u64, pub device: u64, pub inode: u64,
}
fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// Delete only a proven published directory under its original opened root.
/// Caller must load the identity from durable storage, verify task ownership,
/// exclude publication/recovery/removal and concurrent writers, and retain the
/// task and record on error. A late failure can leave a partially removed tree.
/// A missing/replaced directory is an error, not proof of successful deletion.
pub fn cleanup(root: &File, identity: &PublishedIdentity) -> io::Result<()> {
    validate_component(&identity.name)?;
    let parent = root.metadata()?;
    if !parent.is_dir() || parent.dev() != identity.root_device || parent.ino() != identity.root_inode { return Err(invalid("download root identity changed")); }
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let fd = openat(root, identity.name.as_str(), flags, Mode::empty()).map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let dir = File::from(fd); let meta = dir.metadata()?;
    if !meta.is_dir() || meta.dev() != identity.device || meta.ino() != identity.inode
        || meta.uid() != geteuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(invalid("published directory identity or ownership changed"));
    }
    staging_cleanup::unchanged(root, &identity.name, &dir)?;
    staging_cleanup::walk(&dir, identity.device, 0, &mut 0, false)?;
    staging_cleanup::walk(&dir, identity.device, 0, &mut 0, true)?;
    staging_cleanup::unchanged(root, &identity.name, &dir)?;
    unlinkat(root, identity.name.as_str(), UnlinkatFlags::RemoveDir).map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    root.sync_all()
}

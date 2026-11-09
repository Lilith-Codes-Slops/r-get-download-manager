//! Private, descriptor-anchored staging allocation. Not wired to downloads.
//!
//! This is not yet a cleanup or restart protocol. The caller must durably
//! record the returned identity before payload writes, keep exclusive control
//! of the staging area, and verify it again before using or deleting content.
use super::multi_file_layout::validate_component;
use serde::{Deserialize, Serialize};
use nix::{errno::Errno, fcntl::{openat, OFlag}, sys::stat::{mkdirat, Mode}, unistd::{geteuid, unlinkat, UnlinkatFlags}};
use std::{fs::File, io::{self, Read, Write}, os::unix::fs::MetadataExt, sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};

const PARENT: &str = ".rget-staging";
const MARKER: &str = ".rget-owner";
static NEXT: AtomicU64 = AtomicU64::new(0);

fn err(error: Errno) -> io::Error { io::Error::from_raw_os_error(error as i32) }
fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn open_dir(parent: &File, name: &str) -> io::Result<File> {
    let fd = openat(parent, name, OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, Mode::empty()).map_err(err)?;
    Ok(File::from(fd))
}
fn private_dir(dir: &File) -> io::Result<()> {
    let metadata = dir.metadata()?;
    if !metadata.is_dir() || metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(invalid("staging directory must be private and owned by the daemon user"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRecord {
    pub name: String,
    pub task_id: String,
    pub device: u64,
    pub inode: u64,
}

pub struct StageArea { parent: File }
impl StageArea {
    pub(super) fn parent(&self) -> &File { &self.parent }

    /// `download_root` must already be a trusted, opened directory. A symlink
    /// at the staging-parent name is rejected rather than followed.
    pub fn open(download_root: &File) -> io::Result<Self> {
        if !download_root.metadata()?.is_dir() { return Err(invalid("download root is not a directory")); }
        match mkdirat(download_root, PARENT, Mode::from_bits_truncate(0o700)) {
            Ok(()) => download_root.sync_all()?,
            Err(Errno::EEXIST) => {},
            Err(e) => return Err(err(e)),
        }
        let parent = open_dir(download_root, PARENT)?;
        private_dir(&parent)?;
        Ok(Self { parent })
    }

    /// Open an existing staging parent without creating anything. Recovery must
    /// not manufacture a missing parent or follow a symlink in its place.
    pub fn open_existing(download_root: &File) -> io::Result<Self> {
        if !download_root.metadata()?.is_dir() { return Err(invalid("download root is not a directory")); }
        let parent = open_dir(download_root, PARENT)?;
        private_dir(&parent)?;
        Ok(Self { parent })
    }

    /// Generate a candidate without touching the filesystem. Persist a
    /// reservation for this exact name before calling `create_reserved`.
    pub fn candidate_name() -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        format!("task-{}-{now}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// Creates exactly the reserved name and never adopts an existing entry.
    /// The caller must have durably reserved this name for the task first.
    pub fn create_reserved(&self, name: &str, task_id: &str) -> io::Result<StageRecord> {
        validate_component(name)?;
        if !name.starts_with("task-") || task_id.is_empty() || task_id.len() > 512 { return Err(invalid("invalid staging reservation")); }
        private_dir(&self.parent)?;
        mkdirat(&self.parent, name, Mode::from_bits_truncate(0o700)).map_err(err)?;
        let result = self.initialize(name, task_id);
        if result.is_err() {
            // Only remove an empty directory; retain unexpected content.
            let _ = unlinkat(&self.parent, name, UnlinkatFlags::RemoveDir);
            let _ = self.parent.sync_all();
        }
        result
    }

    /// Legacy isolated helper for tests; not suitable for a download until a
    /// database reservation has been committed.
    pub fn create(&self, task_id: &str) -> io::Result<StageRecord> {
        for _ in 0..100 {
            match self.create_reserved(&Self::candidate_name(), task_id) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                result => return result,
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "staging name limit exhausted"))
    }

    fn initialize(&self, name: &str, task_id: &str) -> io::Result<StageRecord> {
        let dir = open_dir(&self.parent, name)?; private_dir(&dir)?;
        let fd = openat(&dir, MARKER, OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::from_bits_truncate(0o600)).map_err(err)?;
        let mut marker = File::from(fd); marker.write_all(task_id.as_bytes())?; marker.sync_all()?;
        dir.sync_all()?; self.parent.sync_all()?;
        let metadata = dir.metadata()?;
        Ok(StageRecord { name: name.into(), task_id: task_id.into(), device: metadata.dev(), inode: metadata.ino() })
    }

    /// Create the payload directory only after recording the stage identity.
    /// Existing entries are never adopted.
    pub fn create_payload(&self, record: &StageRecord) -> io::Result<File> {
        let dir = self.verify(record)?;
        mkdirat(&dir, "payload", Mode::from_bits_truncate(0o700)).map_err(err)?;
        let payload = open_dir(&dir, "payload")?;
        private_dir(&payload)?;
        payload.sync_all()?;
        dir.sync_all()?;
        Ok(payload)
    }

    /// Opens a recorded directory without following symlinks, and checks its
    /// directory identity, permissions and marker. This does not establish
    /// that any nested file is safe or unchanged.
    pub fn verify(&self, record: &StageRecord) -> io::Result<File> {
        validate_component(&record.name)?;
        if !record.name.starts_with("task-") || record.task_id.is_empty() || record.task_id.len() > 512 { return Err(invalid("invalid staging record")); }
        private_dir(&self.parent)?;
        let dir = open_dir(&self.parent, &record.name)?; private_dir(&dir)?;
        let metadata = dir.metadata()?;
        if metadata.dev() != record.device || metadata.ino() != record.inode { return Err(invalid("staging directory identity changed")); }
        let fd = openat(&dir, MARKER, OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC, Mode::empty()).map_err(err)?;
        let marker = File::from(fd); let meta = marker.metadata()?;
        if !meta.is_file() || meta.uid() != geteuid().as_raw() || meta.mode() & 0o077 != 0 || meta.nlink() != 1 || meta.len() > 512 {
            return Err(invalid("invalid staging ownership marker"));
        }
        let mut owner = Vec::new(); marker.take(513).read_to_end(&mut owner)?;
        if owner != record.task_id.as_bytes() { return Err(invalid("staging ownership marker mismatch")); }
        Ok(dir)
    }
}

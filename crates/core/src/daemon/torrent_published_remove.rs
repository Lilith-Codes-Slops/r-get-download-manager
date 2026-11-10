//! Isolated removal of an identity-recorded published directory.
//! Not connected to task removal or crash recovery.
use super::{torrent_publish_intent, torrent_published_store};
use crate::protocols::torrent::published_cleanup::{self, PublishedIdentity};
use downloader_ipc::DownloadInfo;
use rusqlite::OptionalExtension;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Caller must hold the shared lifecycle guard, prevent external writes and
/// serialize this with task snapshot persistence. This intentionally leaves
/// both task and identity records; integrating their eventual deletion requires
/// a durable remove intent to handle crashes after filesystem removal.
#[allow(dead_code)]
pub(super) fn delete_files(
    database: &Path,
    root_path: &Path,
    root: &File,
    task: &DownloadInfo,
) -> Result<()> {
    if task.state != "completed" || task.torrent.is_none() {
        return Err("torrent is not completed".into());
    }
    let published = torrent_published_store::load(database, &task.url)?
        .ok_or("published directory identity missing")?;
    if torrent_publish_intent::has_intent(database, &task.url)? {
        return Err("publication intent requires reconciliation".into());
    }
    let expected_path = root_path.join(&published.name);
    if task.filename.as_deref() != Some(published.name.as_str())
        || task.final_path.as_deref().map(Path::new) != Some(expected_path.as_path())
    {
        return Err("task destination does not match published identity".into());
    }
    let conn = super::persistence::open(database)?;
    let json: Option<String> = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&task.url],
            |row| row.get(0),
        )
        .optional()?;
    let durable: DownloadInfo = serde_json::from_str(&json.ok_or("durable task missing")?)?;
    if durable.state != "completed"
        || durable.url != task.url
        || durable.filename != task.filename
        || durable.final_path != task.final_path
        || durable.torrent != task.torrent
    {
        return Err("durable task identity or destination changed".into());
    }
    // Confirm the supplied path still names the same opened root, not a new or
    // symlinked directory. Cleanup itself checks the persisted root identity.
    let actual = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root_path)?;
    use std::os::unix::fs::MetadataExt;
    let (a, b) = (actual.metadata()?, root.metadata()?);
    if a.dev() != b.dev() || a.ino() != b.ino() {
        return Err("download root changed".into());
    }
    let identity = PublishedIdentity {
        name: published.name,
        root_device: published.root_device,
        root_inode: published.root_inode,
        device: published.device,
        inode: published.inode,
    };
    published_cleanup::cleanup(root, &identity)?;
    Ok(())
}

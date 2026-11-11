//! Conservative startup reconciliation of completed journaled renames.
//! Runs before task loading and before workers start; no source is renamed.
use super::{
    torrent_publish_commit,
    torrent_publish_intent::{self, Intent},
    torrent_recovery_content::{self, ContentEvidence},
    torrent_stage_store,
};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use rusqlite::OptionalExtension;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Report {
    pub completed: usize,
    pub cleaned: usize,
    pub unresolved: usize,
}

/// Call only under the singleton lock, before any workers or IPC mutations.
/// Root and private staging must not be modified by another process. All
/// ambiguous, missing, modified or unverified entries are retained unchanged.
pub(super) fn recover(database: &Path, path: &Path) -> Result<Report> {
    let intents = torrent_publish_intent::load(database)?;
    let mut report = Report::default();
    for intent in intents {
        let result = (|| -> Result<bool> {
            let task_root = super::task_directory::resolve(database, &intent.stage.task_id, path)?;
            let (root, area) = open_area(&task_root)?;
            reconcile_one(database, &task_root, &root, &area, &intent)
        })();
        match result {
            Ok(true) => report.completed += 1,
            Ok(false) => {
                report.unresolved += 1;
                tracing::warn!(task_id = %intent.stage.task_id, "torrent publication recovery needs inspection");
            }
            Err(error) => {
                report.unresolved += 1;
                tracing::warn!(task_id = %intent.stage.task_id, %error, "torrent publication recovery failed; evidence retained");
            }
        }
    }
    // A crash after completion commit but before removing the empty owned
    // stage leaves no intent. Only completed tasks and stages without payload
    // qualify; verification-failure stages are never cleaned here.
    for entry in torrent_stage_store::load(database)? {
        if entry.state != torrent_stage_store::State::Retained {
            continue;
        }
        let result = (|| -> Result<bool> {
            let task_root = super::task_directory::resolve(database, &entry.task_id, path)?;
            let (_, area) = open_area(&task_root)?;
            clean_completed(database, &area, &entry)
        })();
        match result {
            Ok(true) => report.cleaned += 1,
            Ok(false) => {}
            Err(error) => {
                report.unresolved += 1;
                tracing::warn!(task_id = %entry.task_id, %error, "completed stage cleanup deferred");
            }
        }
    }
    Ok(report)
}

fn open_area(path: &Path) -> Result<(File, StageArea)> {
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let area = StageArea::open_existing(&root)?;
    Ok((root, area))
}

fn clean_completed(
    database: &Path,
    area: &StageArea,
    entry: &torrent_stage_store::Entry,
) -> Result<bool> {
    let conn = super::persistence::open(database)?;
    let json: Option<String> = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&entry.task_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Ok(false);
    };
    let task: DownloadInfo = serde_json::from_str(&json)?;
    if task.url != entry.task_id
        || task.state != "completed"
        || task.torrent.is_none()
        || torrent_publish_intent::has_intent(database, &entry.task_id)?
    {
        return Ok(false);
    }
    let record = entry
        .identity
        .as_ref()
        .ok_or("retained stage identity missing")?;
    let stage = area.verify(record)?;
    match nix::fcntl::openat(
        &stage,
        "payload",
        nix::fcntl::OFlag::O_PATH | nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_CLOEXEC,
        nix::sys::stat::Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => {}
        Ok(_) => return Ok(false),
        Err(error) => return Err(std::io::Error::from_raw_os_error(error as i32).into()),
    }
    torrent_stage_store::cleanup_recorded(database, area, &entry.task_id, &entry.name, true)?;
    Ok(true)
}

fn reconcile_one(
    database: &Path,
    path: &Path,
    root: &File,
    area: &StageArea,
    intent: &Intent,
) -> Result<bool> {
    let conn = super::persistence::open(database)?;
    let json: Option<String> = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&intent.stage.task_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Ok(false);
    };
    let task: DownloadInfo = serde_json::from_str(&json)?;
    if task.state != "finalizing" {
        return Ok(false);
    }
    if torrent_recovery_content::inspect(database, area, root, intent, &task)?
        != ContentEvidence::DestinationPiecesMatch
    {
        return Ok(false);
    }
    // A crash may have happened between rename and either parent sync. Both
    // syncs are attempted even if one fails; a failure becomes a completion
    // warning, never a reason to retry or remove the published directory.
    let stage = area.verify(&intent.stage)?;
    let source_sync = stage.sync_all();
    let destination_sync = root.sync_all();
    let warning = source_sync
        .and(destination_sync)
        .err()
        .map(|e| format!("publication recovery directory sync failed: {e}"));
    let mut completed = task;
    completed.state = "completed".into();
    completed.speed_bps = 0;
    completed.filename = Some(intent.destination.clone());
    completed.final_path = Some(
        path.join(&intent.destination)
            .to_str()
            .ok_or("non-UTF-8 destination root")?
            .into(),
    );
    completed
        .torrent
        .as_mut()
        .ok_or("torrent identity missing")?
        .publication_warning = warning;
    torrent_publish_commit::commit(database, path, intent, &completed, Some(root))?;
    if let Err(error) = torrent_stage_store::cleanup_recorded(
        database,
        area,
        &intent.stage.task_id,
        &intent.stage.name,
        true,
    ) {
        tracing::warn!(task_id = %intent.stage.task_id, %error, "recovered torrent stage cleanup failed; record retained");
    }
    Ok(true)
}

pub(super) async fn startup(store: &super::persistence::Persistence) {
    let Some(home) = std::env::var_os("HOME") else {
        tracing::warn!("HOME missing; torrent publication recovery skipped");
        return;
    };
    let database = store.database_path();
    let root = std::path::PathBuf::from(home).join("Downloads");
    match tokio::task::spawn_blocking(move || recover(&database, &root)).await {
        Ok(Ok(report)) => tracing::info!(
            completed = report.completed,
            cleaned = report.cleaned,
            unresolved = report.unresolved,
            "torrent publication recovery"
        ),
        Ok(Err(error)) => {
            tracing::warn!(%error, "torrent publication recovery failed; records retained")
        }
        Err(error) => {
            tracing::warn!(%error, "torrent publication recovery task failed; records retained")
        }
    }
}

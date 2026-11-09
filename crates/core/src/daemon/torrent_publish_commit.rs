//! Isolated transactional completion primitive; not yet used by downloads.
use super::{control::Tasks, persistence::Persistence, torrent_publish_intent::{self, Intent, Observation}};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use rusqlite::params;
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl Persistence {
    /// Caller must exclude concurrent cleanup, publication and external writes
    /// for the task and hold that lifecycle guard through this operation. The
    /// queue snapshot gate prevents older saves from overwriting the commit.
    /// The stage record is deliberately retained for later owned cleanup.
    #[allow(dead_code)]
    pub(super) async fn commit_torrent_publication(&self, tasks: &Tasks, root: &Path,
        intent: &Intent, warning: Option<String>) -> Result<()> {
        let _gate = self.gate.lock().await;
        let mut tasks = tasks.lock().await;
        let task = tasks.get(&intent.stage.task_id).ok_or("torrent task missing")?;
        if task.state != "finalizing" || task.torrent.is_none() { return Err("torrent is not finalizing".into()); }
        let mut completed = task.clone();
        let final_path = root.join(&intent.destination);
        completed.final_path = Some(final_path.to_str().ok_or("non-UTF-8 destination root")?.into());
        completed.filename = Some(intent.destination.clone());
        completed.state = "completed".into(); completed.speed_bps = 0;
        completed.torrent.as_mut().ok_or("torrent identity missing")?.publication_warning = warning;
        let database = self.database_path(); let root = root.to_owned(); let intent = intent.clone();
        let record = completed.clone();
        tokio::task::spawn_blocking(move || commit(&database, &root, &intent, &record)).await??;
        tasks.insert(completed.url.clone(), completed);
        Ok(())
    }
}

/// Commit queue completion and clear the exact intent in one SQLite transaction.
/// Stage ownership is retained until a separate, verified cleanup succeeds.
fn commit(database: &Path, root: &Path, intent: &Intent, completed: &DownloadInfo) -> Result<()> {
    let root_file = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)?;
    let area = StageArea::open_existing(&root_file)?;
    if torrent_publish_intent::inspect(&area, &root_file, intent)? != Observation::DestinationMatches {
        return Err("publication destination is not proven to match intent".into());
    }
    // Validate stored intent and initialize tables before beginning transaction.
    if !torrent_publish_intent::load(database)?.iter().any(|stored| stored == intent) { return Err("publication intent missing or changed".into()); }
    super::torrent_publish_proof::ensure_schema(database)?;
    let mut conn = super::persistence::open(database)?;
    let tx = conn.transaction()?;
    let matched: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_publish_intents_v1 i
        JOIN rget_torrent_stages_v1 s ON s.task_id=i.task_id AND s.stage_name=i.stage_name AND s.device=i.stage_device AND s.inode=i.stage_inode
        WHERE i.task_id=?1 AND i.stage_name=?2 AND i.stage_device=?3 AND i.stage_inode=?4 AND i.destination=?5 AND i.payload_device=?6 AND i.payload_inode=?7 AND s.state='retained')",
        params![intent.stage.task_id, intent.stage.name, intent.stage.device.to_string(), intent.stage.inode.to_string(), intent.destination, intent.payload_device.to_string(), intent.payload_inode.to_string()], |row| row.get(0))?;
    if !matched { return Err("staging identity or publication intent changed".into()); }
    let count = tx.execute("UPDATE rget_queue_v1 SET info=?1 WHERE url=?2", params![serde_json::to_string(completed)?, intent.stage.task_id])?;
    if count != 1 { return Err("durable torrent task missing".into()); }
    let removed = tx.execute("DELETE FROM rget_torrent_publish_intents_v1 WHERE task_id=?1", [&intent.stage.task_id])?;
    if removed != 1 { return Err("publication intent changed before commit".into()); }
    tx.execute("DELETE FROM rget_torrent_publish_proofs_v1 WHERE task_id=?1", [&intent.stage.task_id])?;
    tx.commit()?;
    Ok(())
}

//! Persist a validated multi-file manifest before any staging allocation.
//! Not connected to multi-file ingress or downloads yet.
use super::{control::Tasks, persistence::Persistence, torrent_stage_store};
use crate::protocols::torrent::{multi_file_metadata, staging_manifest::ValidatedManifest};
use downloader_ipc::DownloadInfo;
use rusqlite::params;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl Persistence {
    /// Caller must hold the task lifecycle guard to exclude stage creation,
    /// cancel and remove. This method serializes with queue snapshot writes.
    #[allow(dead_code)]
    pub(super) async fn save_torrent_manifest(&self, tasks: &Tasks, task_id: &str, expected_hash: &str,
        manifest: &ValidatedManifest) -> Result<()> {
        let _gate = self.gate.lock().await;
        let mut guard = tasks.lock().await;
        let task = guard.get(task_id).ok_or("torrent task missing")?;
        let identity = task.torrent.as_ref().ok_or("torrent identity missing")?;
        if task.state != "metadata" || identity.info_hash.as_deref() != Some(expected_hash) || identity.stage_name.is_some() {
            return Err("torrent identity or state changed before manifest commit".into());
        }
        let mut updated = task.clone();
        updated.total = Some(manifest.total_bytes());
        updated.suggested_filename = Some(manifest.root_name().to_owned());
        let identity = updated.torrent.as_mut().ok_or("torrent identity missing")?;
        identity.root_name = Some(manifest.root_name().to_owned());
        identity.files = Some(multi_file_metadata::file_records(manifest));
        let database = self.database_path(); let record = updated.clone();
        tokio::task::spawn_blocking(move || commit(&database, &record)).await??;
        guard.insert(task_id.to_owned(), updated);
        Ok(())
    }
}

fn commit(database: &Path, task: &DownloadInfo) -> Result<()> {
    // Create the stages table if needed; a reservation must prevent this write.
    torrent_stage_store::load(database)?;
    let mut conn = super::persistence::open(database)?; let tx = conn.transaction()?;
    // Do not overwrite a newer or unrelated durable task merely because its
    // URL matches the in-memory snapshot. Both sides must still be at the
    // metadata boundary and identify the same source and info-hash.
    let json: String = tx.query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", [&task.url], |row| row.get(0))?;
    let durable: DownloadInfo = serde_json::from_str(&json)?;
    let old = durable.torrent.as_ref().ok_or("durable torrent identity missing")?;
    let new = task.torrent.as_ref().ok_or("torrent identity missing")?;
    if durable.url != task.url || durable.state != "metadata" || old.source != new.source
        || old.info_hash != new.info_hash || old.created_at != new.created_at || old.stage_name.is_some()
        || (old.root_name.is_some() && old.root_name != new.root_name)
        || (old.files.is_some() && old.files != new.files) {
        return Err("durable torrent identity, state or manifest changed".into());
    }
    let rows = tx.execute("UPDATE rget_queue_v1 SET info=?1 WHERE url=?2 AND NOT EXISTS (SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?2)",
        params![serde_json::to_string(task)?, task.url])?;
    if rows != 1 { return Err("durable task missing or staging reservation already exists".into()); }
    tx.commit()?;
    Ok(())
}

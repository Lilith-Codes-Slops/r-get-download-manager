//! Abandon only a stopped publication attempt whose exact payload is still staged.
use super::{torrent_publish_intent::{self, Observation}, torrent_publish_proof, torrent_published_store, torrent_remove_completed};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use rusqlite::{params, OptionalExtension};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Caller holds lifecycle, task, and persistence locks; engine storage must be
/// quiescent and filesystem mutation excluded. A destination match or an
/// ambiguous observation is never permission to discard publication evidence.
pub(super) fn abandon_source(database: &Path, root: &Path, task: &DownloadInfo) -> Result<()> {
    let Some(intent) = torrent_publish_intent::load(database)?.into_iter().find(|entry| entry.stage.task_id == task.url) else { return Ok(()); };
    if task.torrent.as_ref().and_then(|t| t.stage_name.as_deref()) != Some(intent.stage.name.as_str())
        || task.final_path.is_some() || !(task.state == "canceled" || task.state.starts_with("failed:")) {
        return Err("publication intent is not a stopped unpublished task".into());
    }
    let root = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)?;
    let area = StageArea::open_existing(&root)?;
    if torrent_publish_intent::inspect(&area, &root, &intent)? != Observation::SourcePresent {
        return Err("publication source is missing or ambiguous; reconciliation required".into());
    }
    torrent_publish_proof::ensure_schema(database)?; torrent_published_store::ensure_schema(database)?;
    let mut conn = torrent_remove_completed::open(database)?; let tx = conn.transaction()?;
    let json: Option<String> = tx.query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", [&task.url], |row| row.get(0)).optional()?;
    let durable: DownloadInfo = serde_json::from_str(&json.ok_or("durable task missing")?)?;
    if durable != *task { return Err("durable torrent task changed".into()); }
    let removed = tx.execute("DELETE FROM rget_torrent_publish_intents_v1 WHERE task_id=?1 AND stage_name=?2 AND stage_device=?3 AND stage_inode=?4 AND destination=?5 AND payload_device=?6 AND payload_inode=?7
        AND EXISTS (SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?1 AND stage_name=?2 AND device=?3 AND inode=?4 AND state='retained')
        AND NOT EXISTS (SELECT 1 FROM rget_torrent_published_v1 WHERE task_id=?1)
        AND NOT EXISTS (SELECT 1 FROM rget_torrent_remove_intents_v1 WHERE task_id=?1)",
        params![task.url, intent.stage.name, intent.stage.device.to_string(), intent.stage.inode.to_string(), intent.destination,
            intent.payload_device.to_string(), intent.payload_inode.to_string()])?;
    if removed != 1 { return Err("publication intent or ownership changed".into()); }
    tx.execute("DELETE FROM rget_torrent_publish_proofs_v1 WHERE task_id=?1", [&task.url])?;
    tx.commit()?; Ok(())
}

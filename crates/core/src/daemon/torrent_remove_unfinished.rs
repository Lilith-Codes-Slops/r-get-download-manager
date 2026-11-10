//! Delete a stopped unpublished torrent's proven-owned stage and task.
use super::{
    control::Tasks, persistence::Persistence, torrent_publish_intent, torrent_published_store,
    torrent_remove_completed, torrent_stage_store,
};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use rusqlite::OptionalExtension;
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn ensure_task(conn: &rusqlite::Connection, task: &DownloadInfo) -> Result<()> {
    let json: Option<String> = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&task.url],
            |r| r.get(0),
        )
        .optional()?;
    let durable: DownloadInfo = serde_json::from_str(&json.ok_or("durable task missing")?)?;
    if durable != *task
        || task.torrent.is_none()
        || task.final_path.is_some()
        || !(task.state == "canceled" || task.state.starts_with("failed:"))
    {
        return Err("unfinished torrent task changed".into());
    }
    Ok(())
}
fn remove(path: &Path, root: &Path, task: &DownloadInfo) -> Result<()> {
    let conn = torrent_remove_completed::open(path)?;
    ensure_task(&conn, task)?;
    super::torrent_abandon_intent::abandon_source(path, root, task)?;
    if torrent_publish_intent::has_intent(path, &task.url)?
        || torrent_published_store::load(path, &task.url)?.is_some()
    {
        return Err("published or journaled torrent requires reconciliation".into());
    }
    if let Some(stage) = torrent_stage_store::load(path)?
        .into_iter()
        .find(|row| row.task_id == task.url)
    {
        let root_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        let area = StageArea::open_existing(&root_file)?;
        torrent_stage_store::cleanup_recorded(path, &area, &stage.task_id, &stage.name, true)?;
    }
    let mut conn = torrent_remove_completed::open(path)?;
    let tx = conn.transaction()?;
    ensure_task(&tx, task)?;
    let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?1 UNION ALL SELECT 1 FROM rget_torrent_publish_intents_v1 WHERE task_id=?1 UNION ALL SELECT 1 FROM rget_torrent_published_v1 WHERE task_id=?1 UNION ALL SELECT 1 FROM rget_torrent_remove_intents_v1 WHERE task_id=?1)", [&task.url], |r| r.get(0))?;
    if busy {
        return Err("torrent ownership or intent changed during removal".into());
    }
    tx.execute(
        "DELETE FROM rget_torrent_recovery_info_v1 WHERE task_id=?1",
        [&task.url],
    )?;
    tx.execute("DELETE FROM rget_rates_v1 WHERE id=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_checkpoints_v1 WHERE url=?1", [&task.url])?;
    tx.execute(
        "DELETE FROM rget_split_checkpoints_v2 WHERE url=?1",
        [&task.url],
    )?;
    tx.execute("DELETE FROM rget_queue_v1 WHERE url=?1", [&task.url])?;
    tx.commit()?;
    Ok(())
}
impl Persistence {
    /// Caller must first signal and wait for the worker's storage barrier.
    /// Unsafe, reserved or ambiguous stages remain untouched on failure.
    pub(super) async fn remove_unfinished_torrent(
        &self,
        tasks: &Tasks,
        task_id: &str,
        root: &Path,
    ) -> Result<()> {
        let lifecycle = super::torrent_lifecycle::global().lock(task_id).await?;
        let gate = self.gate.clone().lock_owned().await;
        let mut tasks = tasks.clone().lock_owned().await;
        let task = tasks.get(task_id).ok_or("torrent task missing")?.clone();
        let path = self.database_path();
        let root = root.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let (_lifecycle, _gate) = (lifecycle, gate);
            remove(&path, &root, &task)?;
            tasks.remove(&task.url);
            Ok(())
        })
        .await?
    }
}

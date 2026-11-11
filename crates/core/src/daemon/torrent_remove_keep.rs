//! Isolated torrent remove-from-list operation; not connected to IPC.
use super::{control::Tasks, persistence::Persistence, torrent_remove_completed};
use downloader_ipc::DownloadInfo;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn forget(database: &Path, root: Option<&Path>, task: &DownloadInfo) -> Result<()> {
    if (task.state == "canceled" || task.state.starts_with("failed:"))
        && super::torrent_publish_intent::has_intent(database, &task.url)?
    {
        super::torrent_abandon_intent::abandon_source(
            database,
            root.ok_or("HOME is not set; publication intent requires inspection")?,
            task,
        )?;
    }
    let mut conn = torrent_remove_completed::open(database)?;
    let tx = conn.transaction()?;
    let json: String = tx.query_row(
        "SELECT info FROM rget_queue_v1 WHERE url=?1",
        [&task.url],
        |r| r.get(0),
    )?;
    if serde_json::from_str::<DownloadInfo>(&json)? != *task {
        return Err("durable torrent task changed".into());
    }
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_publish_intents_v1 WHERE task_id=?1 UNION ALL SELECT 1 FROM rget_torrent_remove_intents_v1 WHERE task_id=?1)", [&task.url], |r| r.get(0))?;
    if pending {
        return Err("torrent intent requires reconciliation before removal".into());
    }
    // A detached stage must never be classified as interrupted and deleted on
    // startup. Reserved entries remain reserved and are also never adopted.
    tx.execute(
        "UPDATE rget_torrent_stages_v1 SET state='retained' WHERE task_id=?1 AND state='staging'",
        [&task.url],
    )?;
    tx.execute("DELETE FROM rget_rates_v1 WHERE id=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_checkpoints_v1 WHERE url=?1", [&task.url])?;
    tx.execute(
        "DELETE FROM rget_split_checkpoints_v2 WHERE url=?1",
        [&task.url],
    )?;
    if tx.execute("DELETE FROM rget_queue_v1 WHERE url=?1", [&task.url])? != 1 {
        return Err("durable torrent task missing".into());
    }
    #[cfg(feature = "torrent-test-hooks")]
    super::torrent_test_hooks::pause("keep-before-commit")?;
    // Ownership, recovery-info and stage rows intentionally remain: forgetting
    // a task must not authorize deletion of its files or detached stage.
    tx.commit()?;
    #[cfg(feature = "torrent-test-hooks")]
    super::torrent_test_hooks::pause("keep-after-commit")?;
    Ok(())
}

impl Persistence {
    /// Caller must stop/join any worker first; all paths must use the same
    /// lifecycle registry. No filesystem operation is performed.
    #[allow(dead_code)]
    pub(super) async fn forget_torrent_keep_files(
        &self,
        tasks: &Tasks,
        task_id: &str,
        root: Option<&Path>,
    ) -> Result<()> {
        let lifecycle = super::torrent_lifecycle::global().lock(task_id).await?;
        let gate = self.gate.clone().lock_owned().await;
        let mut tasks = tasks.clone().lock_owned().await;
        let task = tasks.get(task_id).ok_or("torrent task missing")?.clone();
        if task.torrent.is_none()
            || !(task.state == "completed"
                || task.state == "canceled"
                || task.state.starts_with("failed:"))
        {
            return Err("torrent must be stopped before removing from list".into());
        }
        let database = self.database_path();
        let root = root.map(|path| path.to_path_buf());
        tokio::task::spawn_blocking(move || -> Result<()> {
            let (_lifecycle, _gate) = (lifecycle, gate);
            forget(&database, root.as_deref(), &task)?;
            tasks.remove(&task.url);
            Ok(())
        })
        .await?
    }
}

//! Journaled removal for completed multi-file tasks, not wired to IPC.
use super::{control::Tasks, persistence::Persistence, torrent_published_remove, torrent_published_store::{self, PublishedRecord},
    torrent_publish_intent, torrent_stage_store};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) fn open(path: &Path) -> Result<Connection> {
    torrent_stage_store::load(path)?; torrent_publish_intent::load(path)?; torrent_published_store::ensure_schema(path)?;
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_remove_intents_v1 (task_id TEXT PRIMARY KEY NOT NULL, fingerprint TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS rget_torrent_recovery_info_v1 (task_id TEXT PRIMARY KEY NOT NULL, info BLOB NOT NULL);")?;
    Ok(conn)
}
fn fingerprint(task: &DownloadInfo, published: &PublishedRecord) -> Result<String> {
    let bytes = serde_json::to_vec(&("rget-remove-v1", task, &published.task_id, &published.name, published.root_device, published.root_inode, published.device, published.inode))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
fn same_record(conn: &Connection, record: &PublishedRecord) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_published_v1 WHERE task_id=?1 AND name=?2 AND root_device=?3 AND root_inode=?4 AND device=?5 AND inode=?6)",
        params![record.task_id, record.name, record.root_device.to_string(), record.root_inode.to_string(), record.device.to_string(), record.inode.to_string()], |r| r.get(0))?)
}
fn durable_task(conn: &Connection, task: &DownloadInfo) -> Result<bool> {
    let json: Option<String> = conn.query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", [&task.url], |r| r.get(0)).optional()?;
    let Some(json) = json else { return Ok(false); };
    Ok(serde_json::from_str::<DownloadInfo>(&json)? == *task)
}
fn begin(path: &Path, task: &DownloadInfo, record: &PublishedRecord, proof: &str) -> Result<()> {
    let mut conn = open(path)?; let tx = conn.transaction()?;
    if !durable_task(&tx, task)? || !same_record(&tx, record)? || task.state != "completed" || task.torrent.is_none() { return Err("completed task or ownership changed".into()); }
    let publishing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_publish_intents_v1 WHERE task_id=?1)", [&task.url], |r| r.get(0))?;
    if publishing { return Err("publication intent requires reconciliation".into()); }
    tx.execute("INSERT INTO rget_torrent_remove_intents_v1(task_id,fingerprint) VALUES(?1,?2)", params![task.url, proof])?;
    tx.commit()?; Ok(())
}
fn finish(path: &Path, task: &DownloadInfo, record: &PublishedRecord, proof: &str) -> Result<()> {
    let mut conn = open(path)?; let tx = conn.transaction()?;
    if !durable_task(&tx, task)? || !same_record(&tx, record)? { return Err("task or ownership changed during deletion".into()); }
    let found: Option<String> = tx.query_row("SELECT fingerprint FROM rget_torrent_remove_intents_v1 WHERE task_id=?1", [&task.url], |r| r.get(0)).optional()?;
    if found.as_deref() != Some(proof) { return Err("removal intent changed".into()); }
    let stages: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?1)", [&task.url], |r| r.get(0))?;
    if stages { return Err("owned stage record remains".into()); }
    tx.execute("DELETE FROM rget_torrent_recovery_info_v1 WHERE task_id=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_torrent_published_v1 WHERE task_id=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_rates_v1 WHERE id=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_checkpoints_v1 WHERE url=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_split_checkpoints_v2 WHERE url=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_queue_v1 WHERE url=?1", [&task.url])?;
    tx.execute("DELETE FROM rget_torrent_remove_intents_v1 WHERE task_id=?1", [&task.url])?;
    tx.commit()?; Ok(())
}

/// Runs with exclusive daemon lifecycle control. Resuming never assumes that a
/// missing directory was deleted by rGet; all such cases remain unresolved.
pub(super) fn execute(path: &Path, root: &Path, task: &DownloadInfo, resume: bool) -> Result<()> {
    let record = torrent_published_store::load(path, &task.url)?.ok_or("published identity missing")?;
    let proof = fingerprint(task, &record)?;
    let root_file = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)?;
    use std::os::unix::fs::MetadataExt;
    let root_meta = root_file.metadata()?; let expected = root.join(&record.name);
    if root_meta.dev() != record.root_device || root_meta.ino() != record.root_inode
        || task.filename.as_deref() != Some(record.name.as_str())
        || task.final_path.as_deref().map(Path::new) != Some(expected.as_path()) {
        return Err("removal root or destination changed".into());
    }
    let identity = crate::protocols::torrent::published_cleanup::PublishedIdentity {
        name: record.name.clone(), root_device: record.root_device, root_inode: record.root_inode,
        device: record.device, inode: record.inode,
    };
    // Check the exact target before deleting any retained stage. A missing or
    // replaced destination cannot justify progressing an interrupted removal.
    crate::protocols::torrent::published_cleanup::open_owned(&root_file, &identity)?;
    if resume {
        let conn = open(path)?;
        let found: Option<String> = conn.query_row("SELECT fingerprint FROM rget_torrent_remove_intents_v1 WHERE task_id=?1", [&task.url], |r| r.get(0)).optional()?;
        if found.as_deref() != Some(proof.as_str()) || !durable_task(&conn, task)? || !same_record(&conn, &record)?
            || torrent_publish_intent::has_intent(path, &task.url)? || task.state != "completed" || task.torrent.is_none() {
            return Err("removal intent or task identity changed".into());
        }
    } else { begin(path, task, &record, &proof)?; }
    if let Some(stage) = torrent_stage_store::load(path)?.into_iter().find(|row| row.task_id == task.url) {
        let area = StageArea::open_existing(&root_file)?;
        torrent_stage_store::cleanup_recorded(path, &area, &stage.task_id, &stage.name, true)?;
    }
    torrent_published_remove::delete_files(path, root, &root_file, task)?;
    finish(path, task, &record, &proof)
}

pub(super) fn pending(path: &Path) -> Result<Vec<String>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare("SELECT task_id FROM rget_torrent_remove_intents_v1 ORDER BY task_id")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<String>, _>>()?)
}

impl Persistence {
    /// Only call after the engine has stopped and all lifecycle paths use the
    /// same registry. Cancellation cannot unlock the blocking operation early.
    #[allow(dead_code)]
    pub(super) async fn remove_completed_torrent(&self, tasks: &Tasks, task_id: &str, root: &Path) -> Result<()> {
        let lifecycle = super::torrent_lifecycle::global().lock(task_id).await?;
        let gate = self.gate.clone().lock_owned().await;
        let mut tasks = tasks.clone().lock_owned().await;
        let task = tasks.get(task_id).ok_or("torrent task missing")?.clone();
        let database = self.database_path(); let root = root.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let (_lifecycle, _gate) = (lifecycle, gate);
            execute(&database, &root, &task, false)?;
            tasks.remove(&task.url);
            Ok(())
        }).await?
    }
}

//! Transactional queue, bandwidth, and single-stream recovery snapshots.
use super::bandwidth::BandwidthLimiter;
use super::checkpoint::RecoveryRecord;
use super::control::Tasks;
use downloader_ipc::DownloadInfo;
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

type StoreResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Default)]
pub(super) struct SavedState {
    pub tasks: BTreeMap<String, DownloadInfo>,
    pub global_kbps: u64,
    pub task_kbps: HashMap<String, u64>,
    pub prior_states: HashMap<String, String>,
    pub checkpoints: HashMap<String, RecoveryRecord>,
    #[cfg(target_os = "linux")]
    pub split_checkpoints: HashMap<String, super::split::checkpoint::SplitRecord>,
}

pub(super) struct Persistence {
    path: PathBuf,
    // Serialize capture AND commit, so an older snapshot cannot commit last.
    pub(super) gate: std::sync::Arc<Mutex<()>>,
}

impl Persistence {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            gate: std::sync::Arc::new(Mutex::new(())),
        }
    }
    #[cfg(target_os = "linux")]
    pub(super) fn database_path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn default_path() -> StoreResult<PathBuf> {
        let state_dir = match std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(
                std::env::var_os("HOME").ok_or("HOME and XDG_STATE_HOME are not set")?,
            )
            .join(".local/state"),
        };
        Ok(state_dir.join("rget").join("queue.sqlite3"))
    }
    pub async fn load(&self) -> StoreResult<SavedState> {
        let _gate = self.gate.lock().await;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || load(&path)).await?
    }

    /// Commit the task snapshot and its validated sidecar metadata together.
    pub async fn save_recovery(&self, tasks: &Tasks, record: &RecoveryRecord) -> StoreResult<()> {
        let _gate = self.gate.lock().await;
        let guard = tasks.lock().await;
        let task = guard
            .get(&record.task_url)
            .filter(|task| task.partial_name.as_deref() == Some(record.partial_name.as_str()))
            .cloned()
            .ok_or("checkpoint no longer belongs to the active task")?;
        drop(guard);
        let path = self.path.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || save_recovery(&path, &task, &record)).await?
    }

    /// Store the split checkpoint and its owning task in one SQLite transaction.
    #[cfg(target_os = "linux")]
    pub async fn save_split_recovery(
        &self,
        tasks: &Tasks,
        record: &super::split::checkpoint::SplitRecord,
    ) -> StoreResult<()> {
        let _gate = self.gate.lock().await;
        let mut task = tasks
            .lock()
            .await
            .get(&record.task_url)
            .filter(|task| task.partial_name.as_deref() == Some(record.partial_name.as_str()))
            .cloned()
            .ok_or("split checkpoint no longer belongs to the task")?;
        task.downloaded = record
            .segments
            .iter()
            .try_fold(0u64, |sum, segment| sum.checked_add(segment.downloaded))
            .ok_or("split checkpoint progress overflow")?;
        task.total = Some(record.total);
        let path = self.path.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || -> StoreResult<()> {
            let mut conn = open(&path)?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO rget_queue_v1 (url, info) VALUES (?1, ?2)",
                params![task.url, serde_json::to_string(&task)?],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO rget_split_checkpoints_v2 (url, info) VALUES (?1, ?2)",
                params![task.url, serde_json::to_string(&record)?],
            )?;
            tx.execute(
                "DELETE FROM rget_checkpoints_v1 WHERE url = ?1",
                [&task.url],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    #[cfg(target_os = "linux")]
    pub async fn load_split_recovery(
        &self,
        id: &str,
    ) -> StoreResult<Option<super::split::checkpoint::SplitRecord>> {
        use rusqlite::OptionalExtension;
        let _gate = self.gate.lock().await;
        let path = self.path.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(
            move || -> StoreResult<Option<super::split::checkpoint::SplitRecord>> {
                let conn = open(&path)?;
                let json: Option<String> = conn
                    .query_row(
                        "SELECT info FROM rget_split_checkpoints_v2 WHERE url = ?1",
                        [&id],
                        |row| row.get(0),
                    )
                    .optional()?;
                match json {
                    Some(value) => Ok(Some(serde_json::from_str(&value)?)),
                    None => Ok(None),
                }
            },
        )
        .await?
    }

    pub async fn save(&self, tasks: &Tasks, limiter: &BandwidthLimiter) -> StoreResult<()> {
        let _gate = self.gate.lock().await;
        let guard = tasks.lock().await;
        let (global_kbps, task_kbps) = limiter.rates().await;
        let state = SavedState {
            tasks: guard.clone(),
            global_kbps,
            task_kbps,
            ..SavedState::default()
        };
        drop(guard);
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || save(&path, &state)).await?
    }
}

pub(super) fn open(path: &Path) -> StoreResult<Connection> {
    let parent = path
        .parent()
        .ok_or("state database has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .mode(0o600)
            .open(path)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS rget_queue_v1 (url TEXT PRIMARY KEY NOT NULL, info TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS rget_rates_v1 (id TEXT PRIMARY KEY NOT NULL, kbps TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS rget_checkpoints_v1 (url TEXT PRIMARY KEY NOT NULL, info TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS rget_split_checkpoints_v2 (url TEXT PRIMARY KEY NOT NULL, info TEXT NOT NULL);")?;
    Ok(conn)
}

fn save(path: &Path, state: &SavedState) -> StoreResult<()> {
    let mut conn = open(path)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM rget_queue_v1", [])?;
    tx.execute("DELETE FROM rget_rates_v1", [])?;
    for (id, info) in &state.tasks {
        tx.execute(
            "INSERT INTO rget_queue_v1(url, info) VALUES (?1, ?2)",
            params![id, serde_json::to_string(info)?],
        )?;
    }
    // Empty id denotes the global rate; valid task ids are non-empty URLs.
    tx.execute(
        "INSERT INTO rget_rates_v1(id, kbps) VALUES (?1, ?2)",
        params!["", state.global_kbps.to_string()],
    )?;
    for (id, kbps) in &state.task_kbps {
        if state.tasks.contains_key(id) {
            tx.execute(
                "INSERT INTO rget_rates_v1(id, kbps) VALUES (?1, ?2)",
                params![id, kbps.to_string()],
            )?;
        }
    }
    tx.execute(
        "DELETE FROM rget_checkpoints_v1 WHERE url NOT IN (SELECT url FROM rget_queue_v1)",
        [],
    )?;
    tx.execute(
        "DELETE FROM rget_split_checkpoints_v2 WHERE url NOT IN (SELECT url FROM rget_queue_v1)",
        [],
    )?;
    for (id, info) in &state.tasks {
        if info.state == "completed"
            || info.state == "canceled"
            || info.state.starts_with("failed:")
        {
            tx.execute("DELETE FROM rget_checkpoints_v1 WHERE url = ?1", [id])?;
            tx.execute("DELETE FROM rget_split_checkpoints_v2 WHERE url = ?1", [id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_recovery(path: &Path, task: &DownloadInfo, record: &RecoveryRecord) -> StoreResult<()> {
    let mut conn = open(path)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO rget_queue_v1 (url, info) VALUES (?1, ?2)",
        params![task.url, serde_json::to_string(task)?],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO rget_checkpoints_v1 (url, info) VALUES (?1, ?2)",
        params![task.url, serde_json::to_string(record)?],
    )?;
    tx.execute(
        "DELETE FROM rget_split_checkpoints_v2 WHERE url = ?1",
        [&task.url],
    )?;
    tx.commit()?;
    Ok(())
}

fn load(path: &Path) -> StoreResult<SavedState> {
    let conn = open(path)?;
    let mut result = SavedState::default();
    let mut statement = conn.prepare("SELECT url, info FROM rget_queue_v1")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (url, json) = row?;
        let mut info: DownloadInfo = serde_json::from_str(&json)?;
        if info.url != url {
            return Err("stored task URL does not match its key".into());
        }
        // Without a durable byte map and validator, never pretend an unfinished
        // file can resume. The user can retry safely from the beginning.
        if info.state == "canceling" {
            // Never restart a transfer that the user had asked to cancel.
            info.state = "canceled".into();
        } else if !matches!(info.state.as_str(), "completed" | "canceled")
            && !info.state.starts_with("failed:")
        {
            // Torrent staging does not use HTTP range checkpoints. Until a
            // torrent recovery path exists, leave interrupted tasks failed.
            #[cfg(target_os = "linux")]
            let recover_http =
                info.torrent.is_none() && !super::torrent_transfer_dispatch::is_torrent(&url);
            #[cfg(not(target_os = "linux"))]
            let recover_http = true;
            if recover_http {
                result.prior_states.insert(url.clone(), info.state.clone());
            }
            info.state =
                "failed: interrupted by daemon restart; retry starts from the beginning".into();
            info.downloaded = 0;
            info.total = None;
            info.final_path = None;
        }
        info.speed_bps = 0;
        if let Some(metrics) = info.torrent.as_mut().and_then(|t| t.metrics.as_mut()) {
            metrics.stalled = false;
            metrics.current_stall_ms = 0;
        }
        // Live worker counts do not survive a daemon restart.
        info.active_connections = info.active_connections.map(|_| 0);
        info.live_connections_supported = false;
        info.resume_supported = false;
        result.tasks.insert(url, info);
    }
    let mut statement = conn.prepare("SELECT url, info FROM rget_checkpoints_v1")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (url, json) = row?;
        // A malformed checkpoint makes that task unrecoverable, not the entire queue.
        if let Ok(record) = serde_json::from_str::<RecoveryRecord>(&json) {
            if record.task_url == url {
                result.checkpoints.insert(url, record);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        result.split_checkpoints = super::split::startup::load_records(&conn)?;
    }
    let mut statement = conn.prepare("SELECT id, kbps FROM rget_rates_v1")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, rate) = row?;
        let kbps: u64 = rate.parse()?;
        if kbps.checked_mul(125).is_none() {
            return Err("stored speed limit is too large".into());
        }
        if id.is_empty() {
            result.global_kbps = kbps;
        } else if result.tasks.contains_key(&id) {
            result.task_kbps.insert(id, kbps);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, SavedState) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("rget-state-{}-{unique}", std::process::id()))
            .join("queue.sqlite3");
        let mut state = SavedState::default();
        for (url, status) in [
            ("https://example.test/done", "completed"),
            ("https://example.test/live", "downloading"),
        ] {
            state.tasks.insert(
                url.into(),
                DownloadInfo {
                    url: url.into(),
                    state: status.into(),
                    filename: Some("test.bin".into()),
                    downloaded: 12,
                    total: Some(100),
                    speed_bps: 5,
                    final_path: None,
                    destination_path: None,
                    suggested_filename: None,
                    torrent: None,
                    resume_supported: true,
                    partial_name: None,
                    requested_connections: None,
                    active_connections: None,
                    segment_count: None,
                    connection_note: None,
                    live_connections_supported: false,
                },
            );
        }
        state.global_kbps = 800;
        state
            .task_kbps
            .insert("https://example.test/live".into(), 120);
        (path, state)
    }

    #[test]
    fn persists_rates_and_marks_unrecoverable_transfers_interrupted() {
        let (path, state) = fixture();
        save(&path, &state).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.global_kbps, 800);
        assert_eq!(
            loaded.task_kbps.get("https://example.test/live"),
            Some(&120)
        );
        assert_eq!(loaded.tasks["https://example.test/done"].state, "completed");
        let active = &loaded.tasks["https://example.test/live"];
        assert!(active.state.starts_with("failed: interrupted"));
        assert_eq!(active.downloaded, 0);
        assert!(!active.resume_supported);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn later_snapshot_removes_deleted_tasks_and_limits() {
        let (path, mut state) = fixture();
        save(&path, &state).unwrap();
        state.tasks.remove("https://example.test/live");
        save(&path, &state).unwrap();
        let loaded = load(&path).unwrap();
        assert!(!loaded.tasks.contains_key("https://example.test/live"));
        assert!(!loaded.task_kbps.contains_key("https://example.test/live"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn persists_checkpoint_record_with_task_state() {
        let (path, mut state) = fixture();
        let id = "https://example.test/live";
        let task = state.tasks.get_mut(id).unwrap();
        task.state = "paused".into();
        task.partial_name = Some(".rget-1-1.part".into());
        let record = RecoveryRecord {
            version: 1,
            sequence: 7,
            task_url: id.into(),
            resolved_url: id.into(),
            filename: "test.bin".into(),
            partial_name: ".rget-1-1.part".into(),
            etag: "\"v1\"".into(),
            total: 100,
            downloaded: 12,
            prefix_blake3: "hash".into(),
        };
        save_recovery(&path, task, &record).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.prior_states.get(id).map(String::as_str),
            Some("paused")
        );
        assert_eq!(loaded.checkpoints.get(id).unwrap().sequence, 7);
        assert_eq!(
            loaded.tasks[id].partial_name.as_deref(),
            Some(".rget-1-1.part")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn interrupted_cancel_does_not_restart_the_transfer() {
        let (path, mut state) = fixture();
        let id = "https://example.test/live";
        state.tasks.get_mut(id).unwrap().state = "canceling".into();
        save(&path, &state).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.tasks[id].state, "canceled");
        assert!(!loaded.prior_states.contains_key(id));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
#[cfg(test)]
#[path = "persistence_identity_tests.rs"]
mod identity_tests;

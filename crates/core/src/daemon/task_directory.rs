//! Durable per-task download-root bindings.
//!
//! Bind queue entries and their destination before workers create files.
use super::persistence::{self, Persistence};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn ensure_schema(conn: &Connection) -> Result<()> {
    super::download_directory::ensure_schema(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_task_directory_v1 (
            task_id TEXT PRIMARY KEY NOT NULL,
            root TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rget_task_directory_migrations_v1 (
            id INTEGER PRIMARY KEY CHECK (id = 1)
        );",
    )?;
    Ok(())
}

fn validate_root(root: &Path) -> Result<()> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || root.to_str().is_none()
    {
        return Err("invalid task download root".into());
    }
    Ok(())
}

/// Migrate legacy records once before journal replay or accepting requests.
/// Older daemon versions always wrote to HOME/Downloads, even if XDG points
/// elsewhere. Using the recorded GUI default for them would redirect recovery.
fn migrate(database: &Path, legacy_root: &Path) -> Result<()> {
    validate_root(legacy_root)?;
    let mut conn = persistence::open(database)?;
    ensure_schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let done: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM rget_task_directory_migrations_v1 WHERE id=1)",
        [],
        |row| row.get(0),
    )?;
    if !done {
        let root = legacy_root.to_str().ok_or("non-UTF-8 legacy root")?;
        tx.execute("INSERT OR IGNORE INTO rget_task_directory_v1(task_id,root) SELECT url, ?1 FROM rget_queue_v1", [root])?;
        for table in [
            "rget_torrent_stages_v1",
            "rget_torrent_publish_intents_v1",
            "rget_torrent_remove_intents_v1",
            "rget_torrent_published_v1",
        ] {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )?;
            if exists {
                tx.execute(&format!("INSERT OR IGNORE INTO rget_task_directory_v1(task_id,root) SELECT task_id, ?1 FROM {table}"), [root])?;
            }
        }
        tx.execute(
            "INSERT INTO rget_task_directory_migrations_v1(id) VALUES (1)",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// A missing binding after migration must never select an unrelated directory.
pub(super) fn resolve(database: &Path, id: &str, legacy_root: &Path) -> Result<PathBuf> {
    if id.is_empty() {
        return Err("empty task identity".into());
    }
    validate_root(legacy_root)?;
    let conn = persistence::open(database)?;
    ensure_schema(&conn)?;
    let root: Option<String> = conn
        .query_row(
            "SELECT root FROM rget_task_directory_v1 WHERE task_id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    let root = match root {
        Some(root) => PathBuf::from(root),
        None => {
            let migrated: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM rget_task_directory_migrations_v1 WHERE id=1)",
                [],
                |row| row.get(0),
            )?;
            if migrated {
                return Err("task download root is missing".into());
            }
            legacy_root.to_path_buf()
        }
    };
    validate_root(&root)?;
    Ok(root)
}

/// Load roots for durable queue entries before HTTP recovery starts.
pub(super) fn load_queue_roots(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, PathBuf>> {
    ensure_schema(conn)?;
    let mut statement = conn.prepare(
        "SELECT d.task_id, d.root FROM rget_task_directory_v1 d INNER JOIN rget_queue_v1 q ON q.url = d.task_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut roots = std::collections::HashMap::new();
    for row in rows {
        let (id, root) = row?;
        let root = PathBuf::from(root);
        validate_root(&root)?;
        roots.insert(id, root);
    }
    let migrated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM rget_task_directory_migrations_v1 WHERE id=1)",
        [],
        |row| row.get(0),
    )?;
    if migrated {
        let missing: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM rget_queue_v1 q LEFT JOIN rget_task_directory_v1 d ON d.task_id=q.url WHERE d.task_id IS NULL)", [], |row| row.get(0))?;
        if missing {
            return Err("task download root is missing".into());
        }
    }
    Ok(roots)
}

/// Persist a task and its root in the same transaction, before starting work.
///
/// Existing queue entries keep their binding on retry. An unbound pre-upgrade
/// task is tied to the legacy root, never to the current selection. A new task
/// replaces any stale binding, including when a completed URL is reused.
/// The caller must serialize this with queue snapshots and hold task admission
/// exclusive until the in-memory queue has been updated.
#[allow(dead_code)]
fn bind_and_store(
    database: &Path,
    id: &str,
    task_json: &str,
    retry: bool,
    legacy_root: &Path,
) -> Result<PathBuf> {
    if id.is_empty() {
        return Err("empty task identity".into());
    }
    let task: serde_json::Value = serde_json::from_str(task_json)?;
    if task.get("url").and_then(|value| value.as_str()) != Some(id) {
        return Err("task identity does not match its queue record".into());
    }
    validate_root(legacy_root)?;
    let mut conn = persistence::open(database)?;
    ensure_schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM rget_queue_v1 WHERE url = ?1)",
        [id],
        |row| row.get(0),
    )?;
    let old_root: Option<String> = tx
        .query_row(
            "SELECT root FROM rget_task_directory_v1 WHERE task_id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    if retry && !existing {
        return Err("cannot retry a missing task".into());
    }
    let root = if retry {
        match old_root {
            Some(root) => PathBuf::from(root),
            None => {
                let migrated: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM rget_task_directory_migrations_v1 WHERE id=1)",
                    [],
                    |row| row.get(0),
                )?;
                if migrated {
                    return Err("task download root is missing".into());
                }
                legacy_root.to_path_buf()
            }
        }
    } else {
        let selected: String = tx.query_row(
            "SELECT COALESCE(override_path, default_path) FROM rget_download_directory_v1 WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        PathBuf::from(selected)
    };
    validate_root(&root)?;
    tx.execute(
        "INSERT INTO rget_task_directory_v1 (task_id, root) VALUES (?1, ?2) ON CONFLICT(task_id) DO UPDATE SET root = excluded.root",
        params![id, root.to_str().ok_or("non-UTF-8 task download root")?],
    )?;
    tx.execute(
        "INSERT INTO rget_queue_v1 (url, info) VALUES (?1, ?2) ON CONFLICT(url) DO UPDATE SET info = excluded.info",
        params![id, task_json],
    )?;
    tx.commit()?;
    Ok(root)
}

#[allow(dead_code)]
impl Persistence {
    pub(super) async fn task_directory(&self, id: &str) -> Result<PathBuf> {
        let legacy =
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
        let database = self.database_path();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || resolve(&database, &id, &legacy)).await?
    }

    pub(super) async fn migrate_task_directories(&self) -> Result<()> {
        let legacy =
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
        let database = self.database_path();
        tokio::task::spawn_blocking(move || migrate(&database, &legacy)).await?
    }

    /// Caller holds the persistence gate and task lock, in that order, until
    /// the committed task has been inserted into the in-memory queue.
    pub(super) async fn bind_task_directory_locked(
        &self,
        task: &downloader_ipc::DownloadInfo,
        retry: bool,
    ) -> Result<PathBuf> {
        let legacy =
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
        let database = self.database_path();
        let id = task.url.clone();
        let json = serde_json::to_string(task)?;
        tokio::task::spawn_blocking(move || bind_and_store(&database, &id, &json, retry, &legacy))
            .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "rget-task-directory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("queue.sqlite3");
        let conn = persistence::open(&db).unwrap();
        ensure_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO rget_download_directory_v1 (id, default_path) VALUES (1, '/downloads-a')",
            [],
        )
        .unwrap();
        (db, root)
    }

    fn select(db: &Path, root: &str) {
        let conn = persistence::open(db).unwrap();
        conn.execute(
            "UPDATE rget_download_directory_v1 SET override_path = ?1 WHERE id = 1",
            [root],
        )
        .unwrap();
    }

    #[test]
    fn new_tasks_follow_selection_and_retries_retain_original_root() {
        let (db, root) = fixture();
        let legacy = Path::new("/legacy/Downloads");
        assert_eq!(
            bind_and_store(&db, "a", r#"{"url":"a"}"#, false, legacy).unwrap(),
            Path::new("/downloads-a")
        );
        select(&db, "/downloads-b");
        assert_eq!(
            bind_and_store(&db, "a", r#"{"url":"a","state":"retry"}"#, true, legacy).unwrap(),
            Path::new("/downloads-a")
        );
        assert_eq!(
            bind_and_store(&db, "b", r#"{"url":"b"}"#, false, legacy).unwrap(),
            Path::new("/downloads-b")
        );
        // A completed task can be replaced by a new task with the same ID.
        assert_eq!(
            bind_and_store(&db, "a", r#"{"url":"a"}"#, false, legacy).unwrap(),
            Path::new("/downloads-b")
        );
        let conn = persistence::open(&db).unwrap();
        conn.execute("DELETE FROM rget_queue_v1 WHERE url='a'", [])
            .unwrap();
        drop(conn);
        assert_eq!(
            bind_and_store(&db, "a", r#"{"url":"a"}"#, false, legacy).unwrap(),
            Path::new("/downloads-b")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolution_uses_binding_or_legacy_and_rejects_corrupt_roots() {
        let (db, root) = fixture();
        let legacy = Path::new("/legacy/Downloads");
        assert_eq!(resolve(&db, "unbound", legacy).unwrap(), legacy);
        bind_and_store(&db, "bound", r#"{"url":"bound"}"#, false, legacy).unwrap();
        select(&db, "/downloads-b");
        assert_eq!(
            resolve(&db, "bound", legacy).unwrap(),
            Path::new("/downloads-a")
        );
        let conn = persistence::open(&db).unwrap();
        assert_eq!(
            load_queue_roots(&conn).unwrap().get("bound"),
            Some(&PathBuf::from("/downloads-a"))
        );
        conn.execute(
            "UPDATE rget_task_directory_v1 SET root='relative' WHERE task_id='bound'",
            [],
        )
        .unwrap();
        assert!(load_queue_roots(&conn).is_err());
        drop(conn);
        assert!(resolve(&db, "bound", legacy).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_binds_legacy_tasks_and_journals_once_and_missing_binding_fails_closed() {
        let (db, root) = fixture();
        let conn = persistence::open(&db).unwrap();
        conn.execute(
            "INSERT INTO rget_queue_v1 (url,info) VALUES ('old', '{\"url\":\"old\"}')",
            [],
        )
        .unwrap();
        conn.execute_batch("CREATE TABLE rget_torrent_stages_v1(task_id TEXT PRIMARY KEY); INSERT INTO rget_torrent_stages_v1 VALUES ('orphan');").unwrap();
        drop(conn);
        let legacy = Path::new("/legacy/Downloads");
        migrate(&db, legacy).unwrap();
        select(&db, "/downloads-b");
        migrate(&db, Path::new("/somewhere-else")).unwrap();
        assert_eq!(resolve(&db, "old", legacy).unwrap(), legacy);
        assert_eq!(resolve(&db, "orphan", legacy).unwrap(), legacy);
        assert!(resolve(&db, "unknown", legacy).is_err());
        let conn = persistence::open(&db).unwrap();
        conn.execute("DELETE FROM rget_task_directory_v1 WHERE task_id='old'", [])
            .unwrap();
        assert!(load_queue_roots(&conn).is_err());
        drop(conn);
        assert!(resolve(&db, "old", legacy).is_err());
        assert!(bind_and_store(&db, "old", r#"{"url":"old"}"#, true, legacy).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_selection_and_admission_bind_one_complete_selection() {
        let (db, root) = fixture();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let other_db = db.clone();
        let other_barrier = barrier.clone();
        let change = std::thread::spawn(move || {
            other_barrier.wait();
            select(&other_db, "/downloads-b");
        });
        barrier.wait();
        let selected = bind_and_store(
            &db,
            "race",
            r#"{"url":"race"}"#,
            false,
            Path::new("/legacy"),
        )
        .unwrap();
        change.join().unwrap();
        assert!(selected == Path::new("/downloads-a") || selected == Path::new("/downloads-b"));
        assert_eq!(
            bind_and_store(&db, "race", r#"{"url":"race"}"#, true, Path::new("/legacy")).unwrap(),
            selected
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_write_failure_rolls_back_binding() {
        let (db, root) = fixture();
        let conn = persistence::open(&db).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_queue BEFORE INSERT ON rget_queue_v1 BEGIN SELECT RAISE(ABORT, 'rejected'); END;").unwrap();
        drop(conn);
        let error =
            bind_and_store(&db, "a", r#"{"url":"a"}"#, false, Path::new("/legacy")).unwrap_err();
        assert!(error.to_string().contains("rejected"));
        let conn = persistence::open(&db).unwrap();
        let count: u32 = conn
            .query_row("SELECT count(*) FROM rget_task_directory_v1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_queue_entries_are_bound_to_legacy_root_and_invalid_tasks_do_not_write() {
        let (db, root) = fixture();
        let conn = persistence::open(&db).unwrap();
        conn.execute(
            "INSERT INTO rget_queue_v1 (url, info) VALUES ('old', '{\"url\":\"old\"}')",
            [],
        )
        .unwrap();
        drop(conn);
        select(&db, "/downloads-b");
        let legacy = Path::new("/legacy/Downloads");
        assert_eq!(
            bind_and_store(&db, "old", r#"{"url":"old"}"#, true, legacy).unwrap(),
            legacy
        );
        assert!(bind_and_store(&db, "bad", r#"{"url":"other"}"#, false, legacy).is_err());
        let conn = persistence::open(&db).unwrap();
        let count: u32 = conn
            .query_row(
                "SELECT count(*) FROM rget_task_directory_v1 WHERE task_id='bad'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}

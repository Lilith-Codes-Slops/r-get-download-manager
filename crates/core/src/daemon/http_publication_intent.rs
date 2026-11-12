//! Durable, fail-closed intent for an exact confirmed HTTP publication.
//!
//! These primitives do not publish, remove files, or mark tasks completed.
//! Callers must serialize the task lifecycle and queue snapshots before using
//! them. A record by itself is not evidence that a link was created by us.
use super::{http_partial_owner, persistence};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{fs::File, os::unix::fs::MetadataExt, path::{Component, Path, PathBuf}};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) const UNCERTAIN: &str = "publication outcome uncertain—inspection required";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Intent {
    pub task_id: String,
    // The cryptographically random partial name identifies this attempt. It
    // must match the durable queue entry and the owner reservation.
    pub attempt: String,
    pub root: PathBuf,
    pub root_device: u64,
    pub root_inode: u64,
    pub partial_name: String,
    pub partial_device: u64,
    pub partial_inode: u64,
    pub destination: String,
}

pub(super) fn schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_http_publication_intents_v1 (
            task_id TEXT PRIMARY KEY NOT NULL,
            attempt TEXT NOT NULL UNIQUE,
            root TEXT NOT NULL,
            root_device TEXT NOT NULL,
            root_inode TEXT NOT NULL,
            partial_name TEXT NOT NULL,
            partial_device TEXT NOT NULL,
            partial_inode TEXT NOT NULL,
            destination TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn valid_component(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains('/')
        && !name.contains('\0')
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
}

fn attempt_from_name(name: &str) -> Result<&str> {
    let token = name.strip_prefix(".rget-")
        .and_then(|name| name.strip_suffix(".part"))
        .ok_or("invalid HTTP partial name")?;
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err("HTTP partial has no valid attempt identity".into());
    }
    Ok(token)
}

/// Commit the intent before a publication operation. The descriptors must be
/// the already-verified root and partial that the caller will use; callers
/// must keep them open and hold the lifecycle/snapshot exclusion throughout.
/// Existing attempts are never overwritten. No filesystem entry is changed.
#[allow(dead_code)]
pub(super) fn begin(
    database: &Path,
    task_id: &str,
    root: &Path,
    root_file: &File,
    partial_name: &str,
    partial_file: &File,
    destination: &str,
) -> Result<Intent> {
    if task_id.is_empty() || !root.is_absolute() || !valid_component(destination) {
        return Err("invalid HTTP publication identity".into());
    }
    let attempt = attempt_from_name(partial_name)?.to_owned();
    let root_meta = root_file.metadata()?;
    let partial_meta = partial_file.metadata()?;
    if !root_meta.is_dir() || !partial_meta.is_file() {
        return Err("invalid HTTP publication descriptors".into());
    }
    // Reject an already-replaced root. A later move still requires the caller
    // to fail closed; recording an intent cannot freeze the pathname.
    use std::os::unix::fs::OpenOptionsExt;
    let current = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?.metadata()?;
    if (current.dev(), current.ino()) != (root_meta.dev(), root_meta.ino())
        || std::fs::canonicalize(root)? != root {
        return Err("HTTP publication root identity changed".into());
    }
    let root_string = root.to_str().ok_or("non-UTF-8 task root")?;
    let mut conn = persistence::open(database)?;
    schema(&conn)?;
    http_partial_owner::schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task_json: String = tx.query_row(
        "SELECT info FROM rget_queue_v1 WHERE url=?1", [task_id], |row| row.get(0),
    )?;
    let task: downloader_ipc::DownloadInfo = serde_json::from_str(&task_json)?;
    if task.url != task_id
        || task.partial_name.as_deref() != Some(partial_name)
        || task.confirmed_filename.as_deref() != Some(destination)
    {
        return Err("HTTP publication task identity changed".into());
    }
    let (bound_root, device, inode): (String, String, String) = tx.query_row(
        "SELECT d.root,i.device,i.inode FROM rget_task_directory_v1 d JOIN rget_task_directory_identity_v1 i ON i.task_id=d.task_id WHERE d.task_id=?1",
        [task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if bound_root != root_string || device.parse::<u64>()? != root_meta.dev()
        || inode.parse::<u64>()? != root_meta.ino() {
        return Err("HTTP publication root identity changed".into());
    }
    let (owner_name, owner_device, owner_inode): (String, String, String) = tx.query_row(
        "SELECT name,device,inode FROM rget_http_partial_owner_v1 WHERE task_id=?1 AND device IS NOT NULL AND inode IS NOT NULL",
        [task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if owner_name != partial_name || owner_device.parse::<u64>()? != partial_meta.dev()
        || owner_inode.parse::<u64>()? != partial_meta.ino() {
        return Err("HTTP publication partial identity changed".into());
    }
    let intent = Intent {
        task_id: task_id.into(), attempt, root: root.to_path_buf(),
        root_device: root_meta.dev(), root_inode: root_meta.ino(),
        partial_name: partial_name.into(), partial_device: partial_meta.dev(),
        partial_inode: partial_meta.ino(), destination: destination.into(),
    };
    tx.execute(
        "INSERT INTO rget_http_publication_intents_v1(task_id,attempt,root,root_device,root_inode,partial_name,partial_device,partial_inode,destination) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![intent.task_id, intent.attempt, root_string, intent.root_device.to_string(),
            intent.root_inode.to_string(), intent.partial_name, intent.partial_device.to_string(),
            intent.partial_inode.to_string(), intent.destination],
    )?;
    tx.commit()?;
    Ok(intent)
}

/// Even a malformed record blocks automatic recovery and destructive actions.
pub(super) fn exists(conn: &Connection, task_id: &str) -> Result<bool> {
    schema(conn)?;
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM rget_http_publication_intents_v1 WHERE task_id=?1)",
        [task_id], |row| row.get(0),
    )?)
}

/// Inspect the durable record only. No recorded intent by itself authorizes
/// completing, relinking, deleting, or adopting anything during recovery.
#[allow(dead_code)]
pub(super) fn load(conn: &Connection, task_id: &str) -> Result<Option<Intent>> {
    schema(conn)?;
    let row: Option<(String, String, String, String, String, String, String, String)> = conn.query_row(
        "SELECT attempt,root,root_device,root_inode,partial_name,partial_device,partial_inode,destination FROM rget_http_publication_intents_v1 WHERE task_id=?1",
        [task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
    ).optional()?;
    let Some((attempt, root, root_device, root_inode, partial_name, partial_device, partial_inode, destination)) = row else {
        return Ok(None);
    };
    if task_id.is_empty() || !Path::new(&root).is_absolute() || !valid_component(&destination)
        || attempt_from_name(&partial_name)? != attempt.as_str() {
        return Err("invalid stored HTTP publication intent".into());
    }
    Ok(Some(Intent {
        task_id: task_id.into(), attempt, root: PathBuf::from(root),
        root_device: root_device.parse()?, root_inode: root_inode.parse()?,
        partial_name, partial_device: partial_device.parse()?,
        partial_inode: partial_inode.parse()?, destination,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    const ID: &str = "https://example.test/file";
    const PARTIAL: &str = ".rget-0123456789abcdef0123456789abcdef.part";

    struct Fixture {
        lab: PathBuf,
        root: PathBuf,
        db: PathBuf,
        root_file: File,
        partial_file: File,
    }

    impl Fixture {
        fn new() -> Self {
            let lab = std::env::temp_dir().join(format!(
                "rget-http-intent-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&lab).unwrap();
            let root = lab.join("download");
            std::fs::create_dir(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let db = lab.join("state.sqlite3");
            std::fs::write(root.join(PARTIAL), b"payload").unwrap();
            let root_file = File::open(&root).unwrap();
            let partial_file = File::open(root.join(PARTIAL)).unwrap();
            let root_meta = root_file.metadata().unwrap();
            let partial_meta = partial_file.metadata().unwrap();
            let task = serde_json::json!({
                "url": ID, "state": "finalizing", "filename": "file.bin", "downloaded": 7,
                "total": 7, "speed_bps": 0, "final_path": null,
                "confirmed_filename": "file.bin", "partial_name": PARTIAL
            }).to_string();
            let conn = persistence::open(&db).unwrap();
            http_partial_owner::schema(&conn).unwrap();
            conn.execute_batch("CREATE TABLE rget_task_directory_v1(task_id TEXT PRIMARY KEY, root TEXT NOT NULL);
                CREATE TABLE rget_task_directory_identity_v1(task_id TEXT PRIMARY KEY, device TEXT NOT NULL, inode TEXT NOT NULL);").unwrap();
            conn.execute("INSERT INTO rget_queue_v1(url,info) VALUES(?1,?2)", params![ID, task]).unwrap();
            conn.execute("INSERT INTO rget_task_directory_v1(task_id,root) VALUES(?1,?2)", params![ID, root.to_str().unwrap()]).unwrap();
            conn.execute("INSERT INTO rget_task_directory_identity_v1(task_id,device,inode) VALUES(?1,?2,?3)", params![ID, root_meta.dev().to_string(), root_meta.ino().to_string()]).unwrap();
            conn.execute("INSERT INTO rget_http_partial_owner_v1(task_id,name,device,inode) VALUES(?1,?2,?3,?4)", params![ID, PARTIAL, partial_meta.dev().to_string(), partial_meta.ino().to_string()]).unwrap();
            drop(conn);
            Self { lab, root, db, root_file, partial_file }
        }

        fn begin(&self) -> Result<Intent> {
            begin(&self.db, ID, &self.root, &self.root_file, PARTIAL, &self.partial_file, "file.bin")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.lab); }
    }

    #[test]
    fn intent_retains_exact_evidence_across_publication_crash_stages() {
        let f = Fixture::new();
        let expected = f.begin().unwrap();
        assert!(f.begin().is_err());
        // Before link, after link, and after directory sync all remain
        // uncertain: the record does not itself prove who made a link.
        for stage in 0..3 {
            if stage == 1 { std::fs::hard_link(f.root.join(PARTIAL), f.root.join("file.bin")).unwrap(); }
            if stage == 2 { f.root_file.sync_all().unwrap(); }
            let conn = persistence::open(&f.db).unwrap();
            assert_eq!(load(&conn, ID).unwrap(), Some(expected.clone()));
            drop(conn);
            assert_eq!(std::fs::read(f.root.join(PARTIAL)).unwrap(), b"payload");
            if stage != 0 { assert_eq!(std::fs::read(f.root.join("file.bin")).unwrap(), b"payload"); }
            assert_eq!(UNCERTAIN, "publication outcome uncertain—inspection required");
        }
    }

    #[test]
    fn replaced_root_and_stale_partial_are_not_recorded() {
        let f = Fixture::new();
        let old = f.lab.join("old-download");
        std::fs::rename(&f.root, &old).unwrap();
        std::fs::create_dir(&f.root).unwrap();
        assert!(f.begin().is_err());
        let conn = persistence::open(&f.db).unwrap();
        assert_eq!(load(&conn, ID).unwrap(), None);
        drop(conn);
        std::fs::remove_dir(&f.root).unwrap();
        std::fs::rename(&old, &f.root).unwrap();
        let other = File::create(f.root.join("other")).unwrap();
        assert!(begin(&f.db, ID, &f.root, &f.root_file, PARTIAL, &other, "file.bin").is_err());
        let conn = persistence::open(&f.db).unwrap();
        assert_eq!(load(&conn, ID).unwrap(), None);
    }

    #[test]
    fn wrong_attempt_or_destination_does_not_overwrite_intent() {
        let f = Fixture::new();
        let expected = f.begin().unwrap();
        assert!(begin(&f.db, ID, &f.root, &f.root_file, ".rget-wrong.part", &f.partial_file, "file.bin").is_err());
        assert!(begin(&f.db, ID, &f.root, &f.root_file, PARTIAL, &f.partial_file, "other.bin").is_err());
        let conn = persistence::open(&f.db).unwrap();
        assert_eq!(load(&conn, ID).unwrap(), Some(expected));
    }
}

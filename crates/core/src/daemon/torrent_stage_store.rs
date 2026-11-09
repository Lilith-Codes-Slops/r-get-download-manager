//! Durable reservations for future multi-file staging; not yet used by the daemon.
//! All mutation functions are synchronous and must run off the async executor.
use crate::protocols::torrent::{multi_file_layout::validate_component, staging_tree::StageRecord};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum State { Reserved, Staging, Retained }
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub task_id: String,
    pub name: String,
    pub created_at: i64,
    pub state: State,
    pub identity: Option<StageRecord>,
}

fn open(path: &Path) -> Result<Connection> {
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_stages_v1 (
        task_id TEXT PRIMARY KEY NOT NULL, stage_name TEXT UNIQUE NOT NULL,
        created_at INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('reserved','staging','retained')),
        device TEXT, inode TEXT,
        CHECK((state = 'reserved' AND device IS NULL AND inode IS NULL) OR
              (state IN ('staging','retained') AND device IS NOT NULL AND inode IS NOT NULL)));")?;
    Ok(conn)
}

fn valid(task_id: &str, name: &str, created_at: i64) -> Result<()> {
    validate_component(name)?;
    if task_id.is_empty() || task_id.len() > 512 || !name.starts_with("task-") || created_at <= 0 {
        return Err("invalid staging reservation".into());
    }
    Ok(())
}

/// Commit the exact task/name before allocating its stage directory. Existing
/// records are never replaced: concurrent or stale attempts must reconcile.
#[allow(dead_code)]
pub(super) fn reserve(path: &Path, task_id: &str, name: &str, created_at: i64) -> Result<()> {
    valid(task_id, name, created_at)?;
    let conn = open(path)?;
    conn.execute("INSERT INTO rget_torrent_stages_v1(task_id,stage_name,created_at,state) VALUES(?1,?2,?3,'reserved')", params![task_id, name, created_at])?;
    Ok(())
}

/// Commit the device/inode obtained from the newly created marker-bearing
/// directory. Payload must not be created until this write succeeds.
#[allow(dead_code)]
pub(super) fn confirm(path: &Path, record: &StageRecord) -> Result<()> {
    let conn = open(path)?;
    let count = conn.execute("UPDATE rget_torrent_stages_v1 SET state='staging', device=?1, inode=?2 WHERE task_id=?3 AND stage_name=?4 AND state='reserved'",
        params![record.device.to_string(), record.inode.to_string(), record.task_id, record.name])?;
    if count != 1 { return Err("missing or changed staging reservation".into()); }
    Ok(())
}

/// Verification failures are retained; do not classify them as interrupted cleanup.
#[allow(dead_code)]
pub(super) fn retain(path: &Path, record: &StageRecord) -> Result<()> {
    let conn = open(path)?;
    let count = conn.execute("UPDATE rget_torrent_stages_v1 SET state='retained' WHERE task_id=?1 AND stage_name=?2 AND state='staging' AND device=?3 AND inode=?4",
        params![record.task_id, record.name, record.device.to_string(), record.inode.to_string()])?;
    if count != 1 { return Err("missing or changed staging identity".into()); }
    Ok(())
}

/// Explicitly identify the exact row being removed. Never call this before a
/// verified cleanup or a separately recorded publication has completed.
#[allow(dead_code)]
pub(super) fn forget(path: &Path, record: &StageRecord) -> Result<()> {
    let conn = open(path)?;
    let count = conn.execute("DELETE FROM rget_torrent_stages_v1 WHERE task_id=?1 AND stage_name=?2 AND device=?3 AND inode=?4",
        params![record.task_id, record.name, record.device.to_string(), record.inode.to_string()])?;
    if count != 1 { return Err("missing or changed staging identity".into()); }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn load(path: &Path) -> Result<Vec<Entry>> {
    let conn = open(path)?;
    let mut statement = conn.prepare("SELECT task_id,stage_name,created_at,state,device,inode FROM rget_torrent_stages_v1 ORDER BY created_at,task_id")?;
    let mut rows = statement.query([])?; let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let task_id: String = row.get(0)?; let name: String = row.get(1)?; let created_at: i64 = row.get(2)?;
        valid(&task_id, &name, created_at)?;
        let state: String = row.get(3)?; let device: Option<String> = row.get(4)?; let inode: Option<String> = row.get(5)?;
        let (state, identity) = match (state.as_str(), device, inode) {
            ("reserved", None, None) => (State::Reserved, None),
            ("staging" | "retained", Some(device), Some(inode)) => {
                let identity = StageRecord { task_id: task_id.clone(), name: name.clone(), device: device.parse()?, inode: inode.parse()? };
                (if state == "staging" { State::Staging } else { State::Retained }, Some(identity))
            }
            _ => return Err("invalid stored staging state".into()),
        };
        result.push(Entry { task_id, name, created_at, state, identity });
    }
    Ok(result)
}

/// Reserved rows have no authenticated inode and may be forgotten only after
/// explicitly proving there is no filesystem entry to clean up.
#[allow(dead_code)]
pub(super) fn has_reservation(path: &Path, task_id: &str, name: &str) -> Result<bool> {
    let conn = open(path)?;
    Ok(conn.query_row("SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?1 AND stage_name=?2 AND state='reserved'", params![task_id, name], |_| Ok(())).optional()?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::torrent::staging_tree::StageArea;
    use std::{fs::{self, File}, sync::atomic::{AtomicU64, Ordering}};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn lab() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("rget-stage-store-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap(); path
    }
    #[test]
    fn reserve_create_confirm_reload_and_retain_without_adopting_existing_names() {
        let root = lab(); let path = root.join("state/queue.sqlite3");
        let area = StageArea::open(&File::open(&root).unwrap()).unwrap();
        let name = StageArea::candidate_name();
        reserve(&path, "task-1", &name, 1).unwrap();
        assert!(has_reservation(&path, "task-1", &name).unwrap());
        assert!(reserve(&path, "task-2", &name, 2).is_err());
        assert!(reserve(&path, "task-1", "task-other", 2).is_err());
        let stage = area.create_reserved(&name, "task-1").unwrap();
        assert!(area.create_reserved(&name, "task-2").is_err());
        confirm(&path, &stage).unwrap();
        assert!(!has_reservation(&path, "task-1", &name).unwrap());
        let records = load(&path).unwrap(); assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, State::Staging); assert_eq!(records[0].identity.as_ref(), Some(&stage));
        assert!(area.verify(records[0].identity.as_ref().unwrap()).is_ok());
        let mut altered = stage.clone(); altered.inode = altered.inode.wrapping_add(1);
        assert!(retain(&path, &altered).is_err()); assert!(forget(&path, &altered).is_err());
        retain(&path, &stage).unwrap(); assert_eq!(load(&path).unwrap()[0].state, State::Retained);
        // Test-only deletion after verifying the stage; integration must perform safe cleanup first.
        assert!(area.verify(&stage).is_ok());
        fs::remove_dir_all(root.join(".rget-staging").join(&name)).unwrap();
        forget(&path, &stage).unwrap(); assert!(load(&path).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reservation_cannot_be_confirmed_by_another_task_or_path() {
        let root = lab(); let path = root.join("state/queue.sqlite3");
        reserve(&path, "first", "task-first", 1).unwrap();
        let wrong = StageRecord { task_id: "other".into(), name: "task-first".into(), device: 1, inode: 2 };
        assert!(confirm(&path, &wrong).is_err());
        let wrong = StageRecord { task_id: "first".into(), name: "task-other".into(), device: 1, inode: 2 };
        assert!(confirm(&path, &wrong).is_err());
        let entries = load(&path).unwrap(); assert_eq!(entries[0].state, State::Reserved);
        assert!(entries[0].identity.is_none());
        assert!(reserve(&path, "bad", "../bad", 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

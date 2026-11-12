//! Durable records for publication names awaiting explicit approval.
//!
//! This module is not connected to transfers yet. Callers must hold the task's
//! lifecycle lock, verify its durable queue and staging identity, and ensure
//! that `attempt` changes when an identity is reused. These records alone do
//! not prove ownership of any file or authorize a filesystem operation.
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::path::{Component, Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Confirmation {
    pub task_id: String,
    pub attempt: String,
    pub name: String,
    pub approved: bool,
}

fn valid_component(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
}

pub(super) fn schema(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_publication_confirmation_v1 (
            task_id TEXT PRIMARY KEY NOT NULL,
            attempt TEXT NOT NULL,
            name TEXT NOT NULL,
            approved INTEGER NOT NULL CHECK(approved IN (0, 1))
        );",
    )?;
    Ok(())
}

/// During startup, preserve a task's staged data while any confirmation
/// record remains. Presence alone never authorizes publication.
pub(super) fn exists(conn: &rusqlite::Connection, task_id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM rget_publication_confirmation_v1 WHERE task_id=?1)",
        [task_id],
        |row| row.get(0),
    )?)
}

/// Insert a pending candidate. Never replace an existing candidate or approval.
#[allow(dead_code)]
pub(super) fn insert(database: &Path, task_id: &str, attempt: &str, name: &str) -> Result<()> {
    if task_id.is_empty() || attempt.is_empty() || !valid_component(name) {
        return Err("invalid publication confirmation identity or filename".into());
    }
    let conn = super::persistence::open(database)?;
    schema(&conn)?;
    conn.execute(
        "INSERT INTO rget_publication_confirmation_v1(task_id, attempt, name, approved) VALUES (?1, ?2, ?3, 0)",
        params![task_id, attempt, name],
    )?;
    Ok(())
}

/// A durable approval must match the exact task, attempt, and candidate shown.
#[allow(dead_code)]
pub(super) fn approve(database: &Path, task_id: &str, attempt: &str, name: &str) -> Result<()> {
    let conn = super::persistence::open(database)?;
    schema(&conn)?;
    let changed = conn.execute(
        "UPDATE rget_publication_confirmation_v1 SET approved=1 WHERE task_id=?1 AND attempt=?2 AND name=?3 AND approved=0",
        params![task_id, attempt, name],
    )?;
    if changed != 1 {
        return Err("publication confirmation no longer matches the pending candidate".into());
    }
    Ok(())
}

/// After an approved name also collides, record a new, unapproved candidate.
#[allow(dead_code)]
pub(super) fn replace_after_collision(
    database: &Path,
    task_id: &str,
    attempt: &str,
    approved_name: &str,
    new_name: &str,
) -> Result<()> {
    if !valid_component(new_name) || new_name == approved_name {
        return Err("invalid replacement publication filename".into());
    }
    let conn = super::persistence::open(database)?;
    schema(&conn)?;
    let changed = conn.execute(
        "UPDATE rget_publication_confirmation_v1 SET name=?1, approved=0 WHERE task_id=?2 AND attempt=?3 AND name=?4 AND approved=1",
        params![new_name, task_id, attempt, approved_name],
    )?;
    if changed != 1 {
        return Err("approved publication candidate changed".into());
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn load(database: &Path, task_id: &str) -> Result<Option<Confirmation>> {
    let conn = super::persistence::open(database)?;
    schema(&conn)?;
    let record = conn.query_row(
        "SELECT task_id, attempt, name, approved FROM rget_publication_confirmation_v1 WHERE task_id=?1",
        [task_id],
        |row| Ok(Confirmation { task_id: row.get(0)?, attempt: row.get(1)?, name: row.get(2)?, approved: row.get(3)? }),
    ).optional()?;
    if let Some(record) = &record {
        if record.task_id.is_empty() || record.attempt.is_empty() || !valid_component(&record.name)
        {
            return Err("invalid stored publication confirmation".into());
        }
    }
    Ok(record)
}

/// Remove only the exact approved record after durable completion or explicit
/// cancellation has been reconciled. This does not remove any staged data.
#[allow(dead_code)]
pub(super) fn remove(database: &Path, expected: &Confirmation) -> Result<()> {
    let mut conn = super::persistence::open(database)?;
    schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "DELETE FROM rget_publication_confirmation_v1 WHERE task_id=?1 AND attempt=?2 AND name=?3 AND approved=?4",
        params![expected.task_id, expected.attempt, expected.name, expected.approved],
    )?;
    if changed != 1 {
        return Err("publication confirmation changed before removal".into());
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn concurrent_approval_accepts_only_one_exact_candidate() {
        let root = std::env::temp_dir().join(format!(
            "rget-confirmation-race-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("queue.sqlite3");
        insert(&db, "task", "attempt", "file (1).txt").unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let db = db.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                approve(&db, "task", "attempt", "file (1).txt").is_ok()
            }));
        }
        barrier.wait();
        let successes = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        assert!(load(&db, "task").unwrap().unwrap().approved);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_and_approval_survive_reopen_and_reject_stale_confirmation() {
        let root = std::env::temp_dir().join(format!(
            "rget-confirmation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("queue.sqlite3");
        for name in [
            "",
            ".",
            "..",
            "file/",
            "file/.",
            "dir/file",
            "file\\name",
            "bad\0name",
        ] {
            assert!(insert(&db, "invalid", "attempt", name).is_err(), "{name:?}");
        }
        insert(&db, "task", "attempt-1", "file (1).txt").unwrap();
        assert!(!load(&db, "task").unwrap().unwrap().approved);
        assert!(approve(&db, "task", "old-attempt", "file (1).txt").is_err());
        assert!(approve(&db, "task", "attempt-1", "file (2).txt").is_err());
        approve(&db, "task", "attempt-1", "file (1).txt").unwrap();
        let approved = load(&db, "task").unwrap().unwrap();
        assert!(approved.approved);
        replace_after_collision(&db, "task", "attempt-1", "file (1).txt", "file (2).txt").unwrap();
        assert!(remove(&db, &approved).is_err());
        let next = load(&db, "task").unwrap().unwrap();
        assert_eq!(next.name, "file (2).txt");
        assert!(!next.approved);
        assert!(approve(&db, "task", "attempt-1", "file (1).txt").is_err());
        remove(&db, &next).unwrap();
        assert_eq!(load(&db, "task").unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}

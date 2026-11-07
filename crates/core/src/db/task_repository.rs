use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TaskRepoError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct TaskRepository<'a> {
    conn: &'a Connection,
}

impl<'a> TaskRepository<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(
        &self,
        task_id: &str,
        url: &str,
        state: &str,
        priority: i32,
        connections: u8,
        created_at: i64,
        not_before: Option<i64>,
    ) -> Result<(), TaskRepoError> {
        self.conn.execute(
            "INSERT INTO tasks (task_id, url, state, priority, connections, created_at, not_before)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, url, state, priority, connections as i32, created_at, not_before],
        )?;
        Ok(())
    }

    pub fn update_state(&self, task_id: &str, state: &str) -> Result<(), TaskRepoError> {
        self.conn.execute(
            "UPDATE tasks SET state = ?1, monotonic_seq_no = monotonic_seq_no + 1 WHERE task_id = ?2",
            params![state, task_id],
        )?;
        Ok(())
    }

    pub fn bump_seq(&self, task_id: &str) -> Result<u64, TaskRepoError> {
        self.conn.execute(
            "UPDATE tasks SET monotonic_seq_no = monotonic_seq_no + 1 WHERE task_id = ?1",
            params![task_id],
        )?;
        let seq: i64 = self.conn.query_row(
            "SELECT monotonic_seq_no FROM tasks WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(seq as u64)
    }
}
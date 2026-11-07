use rusqlite::Connection;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MigrationError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct SchemaMigrations;

impl SchemaMigrations {
    pub fn apply(conn: &Connection) -> Result<(), MigrationError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                task_id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                state TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                connections INTEGER NOT NULL DEFAULT 8,
                created_at INTEGER NOT NULL,
                not_before INTEGER,
                monotonic_seq_no INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS files (
                file_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                filename TEXT NOT NULL,
                target_path TEXT NOT NULL,
                total_size INTEGER,
                downloaded INTEGER NOT NULL DEFAULT 0,
                etag TEXT,
                last_modified TEXT,
                FOREIGN KEY(task_id) REFERENCES tasks(task_id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS segments (
                segment_id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                start_byte INTEGER NOT NULL,
                end_byte INTEGER NOT NULL,
                downloaded INTEGER NOT NULL DEFAULT 0,
                mirror_id TEXT,
                state TEXT NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(file_id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS mirrors (
                mirror_id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                url TEXT NOT NULL,
                ttfb_ms INTEGER,
                goodput_bps INTEGER,
                error_rate REAL,
                bytes_served INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(file_id) REFERENCES files(file_id) ON DELETE CASCADE
            );",
        )?;
        Ok(())
    }
}

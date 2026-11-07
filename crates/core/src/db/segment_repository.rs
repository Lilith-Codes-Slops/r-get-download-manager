use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SegmentRepoError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct SegmentRepository<'a> {
    conn: &'a Connection,
}

impl<'a> SegmentRepository<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn upsert(
        &self,
        segment_id: &str,
        file_id: &str,
        start_byte: u64,
        end_byte: u64,
        downloaded: u64,
        mirror_id: Option<&str>,
        state: &str,
    ) -> Result<(), SegmentRepoError> {
        self.conn.execute(
            "INSERT INTO segments (segment_id, file_id, start_byte, end_byte, downloaded, mirror_id, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(segment_id) DO UPDATE SET
                downloaded = excluded.downloaded,
                end_byte = excluded.end_byte,
                mirror_id = excluded.mirror_id,
                state = excluded.state",
            params![segment_id, file_id, start_byte as i64, end_byte as i64, downloaded as i64, mirror_id, state],
        )?;
        Ok(())
    }

    pub fn batch_update_downloaded(
        &self,
        updates: &[(String, u64)],
    ) -> Result<(), SegmentRepoError> {
        let tx = self.conn.unchecked_transaction()?;
        for (id, downloaded) in updates {
            tx.execute(
                "UPDATE segments SET downloaded = ?1 WHERE segment_id = ?2",
                params![*downloaded as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

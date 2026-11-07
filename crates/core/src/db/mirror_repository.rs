use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MirrorRepoError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct MirrorRepository<'a> {
    conn: &'a Connection,
}

impl<'a> MirrorRepository<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn upsert(
        &self,
        mirror_id: &str,
        file_id: &str,
        url: &str,
        ttfb_ms: Option<u64>,
        goodput_bps: u64,
        error_rate: f32,
        bytes_served: u64,
    ) -> Result<(), MirrorRepoError> {
        self.conn.execute(
            "INSERT INTO mirrors (mirror_id, file_id, url, ttfb_ms, goodput_bps, error_rate, bytes_served)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(mirror_id) DO UPDATE SET
                ttfb_ms = excluded.ttfb_ms,
                goodput_bps = excluded.goodput_bps,
                error_rate = excluded.error_rate,
                bytes_served = excluded.bytes_served",
            params![mirror_id, file_id, url, ttfb_ms.map(|v| v as i64), goodput_bps as i64, error_rate, bytes_served as i64],
        )?;
        Ok(())
    }
}

//! Separate settings row; queue snapshot writes never overwrite it.
use super::persistence::Persistence;
use crate::protocols::torrent::settings::TorrentSettings;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn load(path: &Path) -> Result<TorrentSettings> {
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_settings_v1 (id INTEGER PRIMARY KEY CHECK (id = 1), info TEXT NOT NULL);")?;
    let json: Option<String> = conn.query_row("SELECT info FROM rget_torrent_settings_v1 WHERE id = 1", [], |row| row.get(0)).optional()?;
    let settings: TorrentSettings = match json { Some(json) => serde_json::from_str(&json)?, None => TorrentSettings::default() };
    settings.validate()?;
    Ok(settings)
}

// Kept separate from queue snapshot writes so later settings controls can update it atomically.
#[allow(dead_code)]
fn save(path: &Path, settings: &TorrentSettings) -> Result<()> {
    settings.validate()?;
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_settings_v1 (id INTEGER PRIMARY KEY CHECK (id = 1), info TEXT NOT NULL);")?;
    conn.execute("INSERT INTO rget_torrent_settings_v1(id, info) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET info = excluded.info", params![serde_json::to_string(settings)?])?;
    Ok(())
}

pub(super) async fn log_config(store: &Persistence) {
    let path = store.database_path();
    let settings = match tokio::task::spawn_blocking(move || load(&path)).await {
        Ok(Ok(settings)) => settings,
        Ok(Err(error)) => { tracing::warn!(%error, "torrent settings unavailable; using defaults for planned settings"); TorrentSettings::default() }
        Err(error) => { tracing::warn!(%error, "torrent settings load failed; using defaults for planned settings"); TorrentSettings::default() }
    };
    tracing::info!(upload_while_downloading = settings.upload_while_downloading,
        download_time_upload_cap_bps = settings.download_time_upload_cap_bps, seed_upload_cap_bps = settings.seed_upload_cap_bps,
        max_active_downloads = settings.max_active_downloads, max_active_checks = settings.max_active_checks,
        max_active_seeds = settings.max_active_seeds, "stored torrent policy (not applied; upload and seeding disabled)");
    tracing::info!("global torrent peer ceiling: not implemented");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rget-settings-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))).join("state.sqlite3")
    }
    #[test]
    fn missing_row_defaults_and_roundtrip() {
        let path = path();
        assert_eq!(load(&path).unwrap(), TorrentSettings::default());
        let mut settings = TorrentSettings::default(); settings.max_active_seeds = 0;
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let mut invalid = settings.clone(); invalid.max_active_downloads = 0;
        assert!(save(&path, &invalid).is_err());
        assert_eq!(load(&path).unwrap(), settings);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn invalid_row_is_reported() {
        let path = path(); load(&path).unwrap();
        let conn = super::super::persistence::open(&path).unwrap();
        conn.execute("INSERT INTO rget_torrent_settings_v1 VALUES (1, ?1)", ["not json"]).unwrap();
        drop(conn); assert!(load(&path).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

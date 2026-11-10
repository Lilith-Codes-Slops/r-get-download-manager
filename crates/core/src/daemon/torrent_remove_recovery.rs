//! Replay only still-provable journaled removals, before loading tasks/workers.
use super::torrent_remove_completed;
use downloader_ipc::DownloadInfo;
use rusqlite::OptionalExtension;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Report {
    pub removed: usize,
    pub unresolved: usize,
}

/// Requires the daemon singleton and no concurrent external filesystem writers.
/// Missing, changed, or partially cleaned paths always remain unresolved.
pub(super) fn recover(database: &Path, root: &Path) -> Result<Report> {
    let mut report = Report::default();
    for id in torrent_remove_completed::pending(database)? {
        let result = (|| -> Result<()> {
            let conn = super::persistence::open(database)?;
            let json: Option<String> = conn
                .query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", [&id], |r| {
                    r.get(0)
                })
                .optional()?;
            let task: DownloadInfo = serde_json::from_str(&json.ok_or("durable task missing")?)?;
            if task.url != id {
                return Err("task identity mismatch".into());
            }
            torrent_remove_completed::execute(database, root, &task, true)
        })();
        match result {
            Ok(()) => report.removed += 1,
            Err(error) => {
                report.unresolved += 1;
                tracing::warn!(task_id = %id, %error, "torrent removal recovery requires inspection");
            }
        }
    }
    Ok(report)
}
pub(super) async fn startup(store: &super::persistence::Persistence) {
    let Some(home) = std::env::var_os("HOME") else {
        tracing::warn!("HOME missing; torrent removal recovery skipped");
        return;
    };
    let database = store.database_path();
    let root = std::path::PathBuf::from(home).join("Downloads");
    match tokio::task::spawn_blocking(move || recover(&database, &root)).await {
        Ok(Ok(report)) => tracing::info!(
            removed = report.removed,
            unresolved = report.unresolved,
            "torrent removal recovery"
        ),
        Ok(Err(error)) => {
            tracing::warn!(%error, "torrent removal recovery failed; intents retained")
        }
        Err(error) => {
            tracing::warn!(%error, "torrent removal recovery task failed; intents retained")
        }
    }
}

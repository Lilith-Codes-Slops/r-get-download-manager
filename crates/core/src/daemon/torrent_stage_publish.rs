//! Guarded, exact-name publication sequence; not used by downloads yet.
use super::{torrent_publish_intent::{self, Intent, Observation}, torrent_publish_proof,
    torrent_stage_allocate::AllocatedStage, torrent_stage_store};
use crate::protocols::torrent::directory_publish;
use downloader_ipc::DownloadInfo;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Rename already succeeded. The caller must commit completion and later clean
/// the stage while holding this guard. Do not retry a successful rename even if
/// the sync warning is set or the completion commit fails.
#[allow(dead_code)]
pub(super) struct PublishedStage {
    pub stage: AllocatedStage,
    pub intent: Intent,
    pub sync_warning: Option<String>,
}

impl AllocatedStage {
    /// Caller must stop and join a successfully finished engine first, persist
    /// finalizing state, and exclude all external writers. The verification
    /// checks layout and lengths, not torrent piece hashes. This operation does
    /// not mark tasks complete or clean stages. Failure after retention leaves
    /// the stage and any intent in place. In particular, EEXIST and unsupported
    /// no-replace do not trigger suffix retries or deletion.
    #[allow(dead_code)]
    pub(super) async fn publish_exact(self, task: DownloadInfo, destination: String) -> Result<PublishedStage> {
        // Moving the entire guarded stage into the blocking task prevents an
        // aborted waiter from releasing the guard before rename finishes.
        tokio::task::spawn_blocking(move || publish(self, &task, &destination)).await?
    }
}

fn publish(stage: AllocatedStage, task: &DownloadInfo, destination: &str) -> Result<PublishedStage> {
    if task.url != stage.record.task_id || task.state != "finalizing" { return Err("torrent is not finalizing".into()); }
    let db = &stage.reservation.database;
    // Persist retention before verification, so restart cleanup cannot treat a
    // partially verified tree as an ordinary interrupted download.
    torrent_stage_store::retain(db, &stage.record)?;
    let intent = torrent_publish_proof::begin_verified(db, &stage.area, &stage.record, destination, task)?;
    if !torrent_publish_proof::is_verified(db, &intent, task)? { return Err("publication evidence missing or changed".into()); }
    if torrent_publish_intent::inspect(&stage.area, &stage.root, &intent)? != Observation::SourcePresent {
        return Err("staged payload no longer matches publication intent".into());
    }
    let parent = stage.area.verify(&stage.record)?;
    let published = directory_publish::publish_exact(&parent, "payload", &stage.root, &intent.destination)?;
    Ok(PublishedStage { stage, intent, sync_warning: published.sync_error.map(|e| e.to_string()) })
}

//! Persist a validated multi-file manifest before any staging allocation.
//! Not connected to multi-file ingress or downloads yet.
use super::{control::Tasks, persistence::Persistence, torrent_stage_store};
use crate::protocols::torrent::{multi_file_metadata, staging_manifest::ValidatedManifest};
use downloader_ipc::DownloadInfo;
use rusqlite::params;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The caller must keep this guard until stage allocation, confirmation and
/// payload setup finish, and must explicitly handle every failure. All other
/// lifecycle paths must eventually acquire the same registry lock.
pub(super) struct ManifestReservation {
    pub name: String,
    pub(super) task_id: String,
    pub(super) database: std::path::PathBuf,
    pub(super) _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl Persistence {
    /// Caller must hold the task lifecycle guard to exclude stage creation,
    /// cancel and remove. This method serializes with queue snapshot writes.
    #[allow(dead_code)]
    pub(super) async fn save_torrent_manifest(
        &self,
        tasks: &Tasks,
        task_id: &str,
        expected_hash: &str,
        manifest: &ValidatedManifest,
    ) -> Result<()> {
        self.save_manifest_inner(tasks, task_id, expected_hash, manifest, None, None)
            .await
            .map(|_| ())
    }

    /// Persist manifest and exact-name reservation in one transaction, before
    /// touching the filesystem. The returned guard protects the task until the
    /// caller has completed (or safely handled failure of) stage creation.
    /// This is not yet used by download ingress.
    #[allow(dead_code)]
    pub(super) async fn save_manifest_and_reserve(
        &self,
        tasks: &Tasks,
        task_id: &str,
        expected_hash: &str,
        manifest: &ValidatedManifest,
    ) -> Result<ManifestReservation> {
        let lifecycle = super::torrent_lifecycle::global().lock(task_id).await?;
        let name = crate::protocols::torrent::staging_tree::StageArea::candidate_name();
        self.save_manifest_inner(
            tasks,
            task_id,
            expected_hash,
            manifest,
            Some((name, lifecycle)),
            None,
        )
        .await?
        .ok_or_else(|| "missing lifecycle reservation".into())
    }

    /// Also persist the exact validated v1 info dictionary for future piece
    /// verification. Top-level trackers and webseeds are not stored here.
    #[allow(dead_code)]
    pub(super) async fn save_manifest_info_and_reserve(
        &self,
        tasks: &Tasks,
        task_id: &str,
        expected_hash: &str,
        manifest: &ValidatedManifest,
        metainfo: &[u8],
    ) -> Result<ManifestReservation> {
        let info = crate::protocols::torrent::recovery_metadata::from_metainfo(
            metainfo,
            expected_hash,
            manifest,
        )?;
        let lifecycle = super::torrent_lifecycle::global().lock(task_id).await?;
        let name = crate::protocols::torrent::staging_tree::StageArea::candidate_name();
        self.save_manifest_inner(
            tasks,
            task_id,
            expected_hash,
            manifest,
            Some((name, lifecycle)),
            Some(info),
        )
        .await?
        .ok_or_else(|| "missing lifecycle reservation".into())
    }

    async fn save_manifest_inner(
        &self,
        tasks: &Tasks,
        task_id: &str,
        expected_hash: &str,
        manifest: &ValidatedManifest,
        reservation: Option<(String, tokio::sync::OwnedMutexGuard<()>)>,
        info: Option<Vec<u8>>,
    ) -> Result<Option<ManifestReservation>> {
        // Move both guards into the blocking operation: dropping the async
        // caller must not allow stale snapshots to race a still-running commit.
        let gate = self.gate.clone().lock_owned().await;
        let mut guard = tasks.clone().lock_owned().await;
        let task = guard.get(task_id).ok_or("torrent task missing")?;
        let identity = task.torrent.as_ref().ok_or("torrent identity missing")?;
        if task.state != "metadata"
            || identity.info_hash.as_deref() != Some(expected_hash)
            || identity.stage_name.is_some()
        {
            return Err("torrent identity or state changed before manifest commit".into());
        }
        let mut updated = task.clone();
        updated.total = Some(manifest.total_bytes());
        updated.suggested_filename = Some(manifest.root_name().to_owned());
        let identity = updated.torrent.as_mut().ok_or("torrent identity missing")?;
        identity.root_name = Some(manifest.root_name().to_owned());
        identity.files = Some(multi_file_metadata::file_records(manifest));
        let database = self.database_path();
        let task_id = task_id.to_owned();
        tokio::task::spawn_blocking(move || -> Result<Option<ManifestReservation>> {
            let _gate = gate;
            commit(
                &database,
                &updated,
                reservation.as_ref().map(|(name, _)| name.as_str()),
                info.as_deref(),
            )?;
            guard.insert(task_id.clone(), updated);
            Ok(reservation.map(|(name, lifecycle)| ManifestReservation {
                name,
                task_id,
                database,
                _guard: lifecycle,
            }))
        })
        .await?
    }
}

fn commit(
    database: &Path,
    task: &DownloadInfo,
    reservation: Option<&str>,
    info: Option<&[u8]>,
) -> Result<()> {
    // Create the stages table if needed; a reservation must prevent this write.
    torrent_stage_store::load(database)?;
    let mut conn = super::persistence::open(database)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_recovery_info_v1 (task_id TEXT PRIMARY KEY NOT NULL, info BLOB NOT NULL)")?;
    let tx = conn.transaction()?;
    // Do not overwrite a newer or unrelated durable task merely because its
    // URL matches the in-memory snapshot. Both sides must still be at the
    // metadata boundary and identify the same source and info-hash.
    let json: String = tx.query_row(
        "SELECT info FROM rget_queue_v1 WHERE url=?1",
        [&task.url],
        |row| row.get(0),
    )?;
    let durable: DownloadInfo = serde_json::from_str(&json)?;
    let old = durable
        .torrent
        .as_ref()
        .ok_or("durable torrent identity missing")?;
    let new = task.torrent.as_ref().ok_or("torrent identity missing")?;
    if durable.url != task.url
        || durable.state != "metadata"
        || old.source != new.source
        || old.info_hash != new.info_hash
        || old.created_at != new.created_at
        || old.stage_name.is_some()
        || (old.root_name.is_some() && old.root_name != new.root_name)
        || (old.files.is_some() && old.files != new.files)
    {
        return Err("durable torrent identity, state or manifest changed".into());
    }
    let rows = tx.execute("UPDATE rget_queue_v1 SET info=?1 WHERE url=?2 AND NOT EXISTS (SELECT 1 FROM rget_torrent_stages_v1 WHERE task_id=?2)",
        params![serde_json::to_string(task)?, task.url])?;
    if rows != 1 {
        return Err("durable task missing or staging reservation already exists".into());
    }
    if let Some(name) = reservation {
        torrent_stage_store::valid(&task.url, name, new.created_at)?;
        tx.execute("INSERT INTO rget_torrent_stages_v1(task_id,stage_name,created_at,state) VALUES(?1,?2,?3,'reserved')",
            params![task.url, name, new.created_at])?;
    }
    if let Some(info) = info {
        if reservation.is_none() {
            return Err("recovery metadata requires a staging reservation".into());
        }
        tx.execute(
            "INSERT INTO rget_torrent_recovery_info_v1(task_id,info) VALUES(?1,?2)",
            params![task.url, info],
        )?;
    }
    tx.commit()?;
    Ok(())
}

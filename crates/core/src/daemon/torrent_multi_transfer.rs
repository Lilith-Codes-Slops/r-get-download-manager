//! Local multi-file torrent path; magnets remain disabled for multi-file.
use super::{bandwidth::BandwidthLimiter, control::{Tasks, TransferControl}, persistence::Persistence,
    torrent_stage_allocate::AllocatedStage, torrent_stage_store};
use crate::protocols::torrent::{gated_storage::GatedFilesystemFactory, metrics::MetricsTracker,
    multi_file_metadata, session, storage_gate::StorageGate, task_identity::{Prepared, PreparedSource}};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, storage::StorageFactoryExt};
use std::{path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::sync::watch;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Rename succeeded but durable completion did not. The caller must not mark
/// the task failed or retry its download; startup reconciliation needs finalizing.
#[derive(Debug)]
pub(super) struct PendingPublication(pub String);
impl std::fmt::Display for PendingPublication { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "published directory needs reconciliation: {}", self.0) } }
impl std::error::Error for PendingPublication {}

pub(super) async fn run(id: &str, connections: u8, tasks: &Tasks, store: &Persistence,
    limiter: &Arc<BandwidthLimiter>, mut control: watch::Receiver<TransferControl>, prepared: Prepared) -> Result<()> {
    let PreparedSource::Metainfo(bytes) = prepared.source else { return Err("multi-file magnets are deferred".into()); };
    let manifest = multi_file_metadata::preflight_local(&bytes)?.ok_or("multi-file manifest missing")?;
    if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled".into()); }
    // The metadata state and task identity must be durable before reservation.
    store.save(tasks, limiter).await?;
    let reservation = store.save_manifest_info_and_reserve(tasks, id, &prepared.info_hash, &manifest, &bytes).await?;
    let root = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
    tokio::fs::create_dir_all(&root).await?;
    let opened = open_root(&root).await?;
    let stage = reservation.allocate(opened).await?;
    let stage_path = root.join(".rget-staging").join(&stage.record.name);
    {
        let mut guard = tasks.lock().await;
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        task.torrent.as_mut().ok_or("torrent identity missing")?.stage_name = Some(stage.record.name.clone());
    }
    if let Err(error) = store.save(tasks, limiter).await {
        return Err(cleanup(stage, &stage_path, format!("could not persist stage: {error}")).await.into());
    }
    let canceled = *control.borrow() == TransferControl::Canceled;
    if canceled { return Err(cleanup(stage, &stage_path, "torrent canceled".into()).await.into()); }
    let payload_path = stage_path.join("payload");
    let session = match Session::new_with_opts(payload_path.clone(), session::download_only_options()).await {
        Ok(session) => session,
        Err(error) => return Err(cleanup(stage, &stage_path, format!("torrent session: {error:#}")).await.into()),
    };
    let gate = StorageGate::new();
    let outcome = run_engine(&session, &gate, &payload_path, id, connections, tasks, &mut control, bytes).await;
    // librqbit stop does not join all tasks. The approved delegated-storage
    // barrier drains admitted calls and permanently rejects future storage I/O.
    session.stop().await;
    let closer = gate.clone();
    if let Err(error) = tokio::task::spawn_blocking(move || closer.close_and_wait()).await? {
        return Err(format!("storage quiescence failed: {error}; stage retained at {}", stage_path.display()).into());
    }
    #[cfg(feature = "torrent-test-hooks")]
    {
        let rejected = crate::protocols::torrent::gated_storage::probe_late_write(gate.clone());
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            super::torrent_test_hooks::late_write(rejected)?;
            super::torrent_test_hooks::pause("after-barrier")
        }).await??;
    }
    if let Err(error) = outcome { return Err(cleanup(stage, &stage_path, error.to_string()).await.into()); }
    let canceled = *control.borrow() == TransferControl::Canceled;
    if canceled { return Err(cleanup(stage, &stage_path, "torrent canceled".into()).await.into()); }
    {
        let mut guard = tasks.lock().await;
        let canceled = *control.borrow() == TransferControl::Canceled;
        if canceled { drop(guard); return Err(cleanup(stage, &stage_path, "torrent canceled".into()).await.into()); }
        let task = guard.get_mut(id).ok_or("torrent task removed")?; task.state = "finalizing".into(); task.speed_bps = 0;
    }
    if let Err(error) = store.save(tasks, limiter).await { return Err(cleanup(stage, &stage_path, format!("could not persist finalizing state: {error}")).await.into()); }
    // Refuse publication into a detached directory if Downloads was replaced
    // after allocation. Like the tree checks, this assumes exclusive control
    // of the root while publication runs.
    use std::os::unix::fs::MetadataExt;
    let reopened = open_root(&root).await?; let current = reopened.metadata()?; let original = stage.root.metadata()?;
    if current.dev() != original.dev() || current.ino() != original.ino() {
        return Err(format!("download root changed; stage retained at {}", stage_path.display()).into());
    }
    let task = tasks.lock().await.get(id).cloned().ok_or("torrent task removed")?;
    let published = stage.publish_with_piece_verification(task, manifest.root_name().to_owned()).await
        .map_err(|e| format!("publication failed: {e}; stage retained at {}", stage_path.display()))?;
    published.complete(store, tasks, &root).await.map_err(|error| PendingPublication(error.to_string()))?;
    Ok(())
}

async fn open_root(root: &Path) -> Result<std::fs::File> {
    let root = root.to_owned();
    Ok(tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)
    }).await??)
}

async fn cleanup(stage: AllocatedStage, path: &Path, reason: String) -> String {
    let path = path.to_owned();
    match tokio::task::spawn_blocking(move || {
        let db = stage.reservation.database.clone();
        torrent_stage_store::cleanup_recorded(&db, &stage.area, &stage.record.task_id, &stage.record.name, false)
    }).await {
        Ok(Ok(())) => reason,
        Ok(Err(error)) => format!("{reason}; stage retained at {}: {error}", path.display()),
        Err(error) => format!("{reason}; stage cleanup failed at {}: {error}", path.display()),
    }
}

async fn run_engine(session: &Arc<Session>, gate: &StorageGate, payload: &Path, id: &str, connections: u8,
    tasks: &Tasks, control: &mut watch::Receiver<TransferControl>, bytes: Vec<u8>) -> Result<()> {
    if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled".into()); }
    let path = payload.to_str().ok_or("non-UTF-8 staging path")?.to_owned();
    let opts = AddTorrentOptions { output_folder: Some(path), peer_limit: Some(connections as usize),
        storage_factory: Some(GatedFilesystemFactory { gate: gate.clone() }.boxed()), ..Default::default() };
    let started = Instant::now();
    let added = tokio::select! {
        result = session.add_torrent(AddTorrent::from_bytes(bytes), Some(opts)) => result.map_err(|e| format!("torrent start: {e:#}"))?,
        _ = control.changed() => return Err("torrent canceled".into()),
    };
    let handle = match added { AddTorrentResponse::Added(_, handle) => handle, _ => return Err("torrent already managed".into()) };
    let now = Instant::now(); let mut metrics = MetricsTracker::new(now.duration_since(started), now, Duration::from_secs(30));
    super::torrent_transfer::monitor(&handle, id, tasks, control, &mut metrics).await
}

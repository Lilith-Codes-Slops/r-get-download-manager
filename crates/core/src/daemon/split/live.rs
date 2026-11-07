//! Live split transfers. User-directed resizing remains disabled.
use super::{checkpoint::{SplitCheckpoint, SplitRecord}, recovery::PreparedSplit, runner, segments::SegmentMap};
use crate::daemon::{bandwidth::BandwidthLimiter, control::{Controls, Tasks, TransferControl}, persistence::Persistence, range::ResumeInfo};
use std::{path::{Path, PathBuf}, sync::Arc};
use tokio::sync::{watch, Mutex};

pub(in crate::daemon) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Probe before allocating/writing a split partial. 200/416 mean fallback;
/// malformed 206 and changed validators fail rather than mix representations.
pub(in crate::daemon) async fn probe(client: &reqwest::Client, url: &str, info: &ResumeInfo, control: &mut watch::Receiver<TransferControl>) -> Result<bool> {
    let request = client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, "bytes=0-0").header(reqwest::header::IF_RANGE, info.etag.as_str());
    let response = tokio::select! {
        response = request.send() => response?,
        _ = control.changed() => return Err("download canceled".into()),
    };
    if matches!(response.status(), reqwest::StatusCode::OK | reqwest::StatusCode::RANGE_NOT_SATISFIABLE) { return Ok(false); }
    crate::daemon::range::validate_bounded(&response, info, 0, 1)?;
    Ok(true)
}

pub(in crate::daemon) fn create(partial: &Path, file: std::fs::File, url: &str, filename: &str, info: &ResumeInfo, count: u8) -> Result<PreparedSplit> {
    let map = SegmentMap::new(info.total, count)?;
    let partial_name = partial.file_name().and_then(|s| s.to_str()).ok_or("invalid partial filename")?;
    let record = SplitRecord { version: 2, sequence: 0, task_url: url.into(), resolved_url: info.url.to_string(),
        filename: filename.into(), partial_name: partial_name.into(), etag: info.etag.clone(), total: info.total,
        requested_connections: count, segments: map.records() };
    Ok(PreparedSplit { record, file: Arc::new(file), map: Arc::new(Mutex::new(map)) })
}

pub(in crate::daemon) async fn run(prepared: PreparedSplit, partial: PathBuf, client: reqwest::Client, tasks: Tasks,
    limiter: Arc<BandwidthLimiter>, store: Arc<Persistence>, mut control: watch::Receiver<TransferControl>) -> Result<PathBuf> {
    let id = prepared.record.task_url.clone();
    let dest = partial.parent().ok_or("partial has no parent")?.join(&prepared.record.filename);
    let mut side = partial.as_os_str().to_os_string(); side.push(".meta");
    let sidecar = PathBuf::from(side);
    let checkpoint = SplitCheckpoint { sidecar: sidecar.clone(), record: prepared.record.clone() };
    let result = runner::run_checkpointed(prepared, checkpoint, client, limiter, store, tasks.clone(), Some(control.clone())).await;
    if let Err(error) = result {
        let canceled = *control.borrow() == TransferControl::Canceled;
        if canceled {
            let _ = tokio::fs::remove_file(&partial).await;
            let _ = tokio::fs::remove_file(&sidecar).await;
        }
        return Err(error.into());
    }
    // Serialize the last pause/cancel check with publication and task removal.
    let mut guard = loop {
        if !crate::daemon::control::wait_running(&tasks, &id, &mut control).await {
            let _ = tokio::fs::remove_file(&partial).await;
            let _ = tokio::fs::remove_file(&sidecar).await;
            return Err("download canceled".into());
        }
        let guard = tasks.lock().await;
        let state = *control.borrow();
        if state == TransferControl::Canceled || !guard.contains_key(&id) {
            drop(guard);
            let _ = tokio::fs::remove_file(&partial).await;
            let _ = tokio::fs::remove_file(&sidecar).await;
            return Err("download canceled".into());
        }
        if state == TransferControl::Paused { drop(guard); continue; }
        break guard;
    };
    if let Some(task) = guard.get_mut(&id) { task.state = "finalizing".into(); task.downloaded = task.total.unwrap_or(task.downloaded); }
    crate::daemon::transfer::publish_no_clobber(&partial, &dest).await?;
    // Persist the published directory entry before discarding recovery files.
    let parent = dest.parent().ok_or("destination has no directory")?.to_owned();
    tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all()).await??;
    drop(guard);
    let _ = tokio::fs::remove_file(&partial).await;
    let _ = tokio::fs::remove_file(&sidecar).await;
    Ok(dest)
}

pub(in crate::daemon) async fn spawn(prepared: PreparedSplit, root: &Path, tasks: Tasks, controls: Controls,
    limiter: Arc<BandwidthLimiter>, store: Arc<Persistence>) {
    let id = prepared.record.task_url.clone();
    let partial = root.join(&prepared.record.partial_name);
    let paused = tasks.lock().await.get(&id).is_some_and(|task| task.state == "paused");
    let (sender, receiver) = watch::channel(if paused { TransferControl::Paused } else { TransferControl::Running });
    controls.lock().await.insert(id.clone(), sender);
    tokio::spawn(async move {
        let client = match reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(60)).redirect(reqwest::redirect::Policy::limited(10)).build() {
            Ok(client) => client, Err(error) => { if let Some(task) = tasks.lock().await.get_mut(&id) { task.state = format!("failed: {error}"); } return; }
        };
        let result = run(prepared, partial, client, tasks.clone(), limiter.clone(), store.clone(), receiver.clone()).await;
        let mut guard = tasks.lock().await;
        if let Some(task) = guard.get_mut(&id) {
            task.speed_bps = 0; task.resume_supported = false;
            if *receiver.borrow() == TransferControl::Canceled { task.state = "canceled".into(); }
            else { match result {
                Ok(path) => { task.state = "completed".into(); task.final_path = Some(path.to_string_lossy().into_owned()); task.partial_name = None; }
                Err(error) => task.state = format!("failed: {error}"),
            }}
        }
        drop(guard);
        if let Err(error) = store.save(&tasks, &limiter).await { eprintln!("queue checkpoint failed: {error}"); }
    });
}

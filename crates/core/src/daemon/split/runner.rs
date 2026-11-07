//! Checkpoint a split prototype at a stable write barrier while it runs.
use super::{checkpoint::SplitCheckpoint, recovery::PreparedSplit, run_map};
use crate::daemon::{bandwidth::BandwidthLimiter, control::{Tasks, TransferControl}, persistence::Persistence, range::ResumeInfo};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::watch;

// If the owner future is dropped, request a stop. The independent supervisor
// keeps running until all worker tasks (and their blocking writes) have joined.
struct StopOnDrop(watch::Sender<bool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) { self.0.send_replace(true); }
}

/// Accepts either a newly validated empty map or a prepared recovery. Coordinates
/// checkpoints and waits for all workers to stop before returning on cancellation.
pub(super) async fn run_checkpointed(
    prepared: PreparedSplit, mut checkpoint: SplitCheckpoint,
    client: reqwest::Client, limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>, tasks: Tasks, mut control: Option<watch::Receiver<TransferControl>>,
) -> Result<(), String> {
    if control.as_ref().is_some_and(|receiver| *receiver.borrow() == TransferControl::Canceled) { return Err("download canceled".into()); }
    if prepared.record != checkpoint.record { return Err("checkpoint does not match prepared transfer".into()); }
    let info = ResumeInfo {
        etag: prepared.record.etag.clone(), total: prepared.record.total,
        url: reqwest::Url::parse(&prepared.record.resolved_url).map_err(|e| e.to_string())?,
    };
    let url = prepared.record.task_url.clone();
    let task_url = url.clone();
    let partial_name = prepared.record.partial_name.clone();
    let map = prepared.map;
    let file = prepared.file;
    // Commit task ownership and the recovered snapshot before workers run.
    checkpoint.save(&map, &file, &store, &tasks).await?;
    let (stop, _) = watch::channel(false);
    let _stop_on_drop = StopOnDrop(stop.clone());
    let mut transfer = tokio::spawn(run_map(client, url, info, file.clone(), map.clone(), limiter, stop.clone()));
    let mut timer = tokio::time::interval(Duration::from_millis(250));
    let mut last_time = Instant::now();
    let mut last_bytes: u64 = checkpoint.record.segments.iter().map(|s| s.downloaded).sum();
    let initial_bytes = last_bytes;
    let started = Instant::now();
    let outcome = loop {
        tokio::select! {
            result = &mut transfer => break result.map_err(|e| format!("split supervisor failed: {e}"))?,
            _ = async {
                match &mut control { Some(receiver) => { let _ = receiver.changed().await; }, None => std::future::pending::<()>().await }
            } => {
                if control.as_ref().is_some_and(|receiver| *receiver.borrow() == TransferControl::Canceled) {
                    stop.send_replace(true);
                    let _ = transfer.await;
                    return Err("download canceled".into());
                }
            }
            _ = timer.tick() => {
                let bytes: u64 = map.lock().await.records().iter().map(|s| s.downloaded).sum();
                if let Some(task) = tasks.lock().await.get_mut(&task_url) {
                    if task.partial_name.as_deref() == Some(partial_name.as_str()) && task.state == "downloading" {
                        task.downloaded = bytes;
                        task.speed_bps = (bytes.saturating_sub(initial_bytes) as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
                    }
                }
                if bytes.saturating_sub(last_bytes) >= 8 * 1024 * 1024 || last_time.elapsed() >= Duration::from_secs(5) {
                    if let Err(error) = checkpoint.save(&map, &file, &store, &tasks).await {
                        stop.send_replace(true);
                        let _ = transfer.await;
                        return Err(error);
                    }
                    last_bytes = checkpoint.record.segments.iter().map(|s| s.downloaded).sum();
                    last_time = Instant::now();
                }
            }
        }
    };
    outcome?;
    checkpoint.save(&map, &file, &store, &tasks).await?;
    Ok(())
}

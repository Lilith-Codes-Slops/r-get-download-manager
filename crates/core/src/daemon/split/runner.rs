//! Checkpoint a split prototype at a stable write barrier while it runs.
use super::{checkpoint::SplitCheckpoint, recovery::PreparedSplit, run_map};
use crate::daemon::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence, range::ResumeInfo};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::watch;

/// Accepts either a newly validated empty map or a prepared recovery. This is an
/// internal prototype: the live daemon does not yet discover or invoke it.
pub(super) async fn run_checkpointed(
    prepared: PreparedSplit, mut checkpoint: SplitCheckpoint,
    client: reqwest::Client, limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>, tasks: Tasks,
) -> Result<(), String> {
    if prepared.record != checkpoint.record { return Err("checkpoint does not match prepared transfer".into()); }
    let info = ResumeInfo {
        etag: prepared.record.etag.clone(), total: prepared.record.total,
        url: reqwest::Url::parse(&prepared.record.resolved_url).map_err(|e| e.to_string())?,
    };
    let url = prepared.record.task_url.clone();
    let map = prepared.map;
    let file = prepared.file;
    // Commit task ownership and the recovered snapshot before workers run.
    checkpoint.save(&map, &file, &store, &tasks).await?;
    let (stop, _) = watch::channel(false);
    let mut transfer = Box::pin(run_map(client, url, info, file.clone(), map.clone(), limiter, stop.clone()));
    let mut timer = tokio::time::interval(Duration::from_millis(250));
    let mut last_time = Instant::now();
    let mut last_bytes: u64 = checkpoint.record.segments.iter().map(|s| s.downloaded).sum();
    let outcome = loop {
        tokio::select! {
            result = &mut transfer => break result,
            _ = timer.tick() => {
                let bytes: u64 = map.lock().await.records().iter().map(|s| s.downloaded).sum();
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

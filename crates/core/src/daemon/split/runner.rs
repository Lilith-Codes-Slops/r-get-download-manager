//! Coordinate split checkpoints and worker barriers across pause and cancellation.
use super::{checkpoint::SplitCheckpoint, recovery::PreparedSplit, run_map};
use crate::daemon::{bandwidth::BandwidthLimiter, control::{self, Tasks, TransferControl}, persistence::Persistence, range::ResumeInfo};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::watch;

// If the owner future is dropped, request a stop. The independent supervisor
// keeps running until all worker tasks (and their blocking writes) have joined.
struct StopOnDrop(watch::Sender<bool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) { self.0.send_replace(true); }
}

/// Workers are stopped and joined before a pause is checkpointed or acknowledged.
pub(super) async fn run_checkpointed(
    prepared: PreparedSplit, mut checkpoint: SplitCheckpoint,
    client: reqwest::Client, limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>, tasks: Tasks, mut control: Option<watch::Receiver<TransferControl>>,
) -> Result<(), String> {
    if control.as_ref().is_some_and(|rx| *rx.borrow() == TransferControl::Canceled) { return Err("download canceled".into()); }
    if prepared.record != checkpoint.record { return Err("checkpoint does not match prepared transfer".into()); }
    let info = ResumeInfo { etag: prepared.record.etag.clone(), total: prepared.record.total,
        url: reqwest::Url::parse(&prepared.record.resolved_url).map_err(|e| e.to_string())? };
    let workers = prepared.record.requested_connections as usize;
    let url = prepared.record.task_url.clone();
    let partial_name = prepared.record.partial_name.clone();
    let map = prepared.map;
    let file = prepared.file;
    checkpoint.save(&map, &file, &store, &tasks).await?;
    let mut last_time = Instant::now();
    let mut last_bytes: u64 = checkpoint.record.segments.iter().map(|s| s.downloaded).sum();
    let progress = crate::daemon::progress::ProgressMeter::new(last_bytes);
    let mut complete = false;
    loop {
        if let Some(rx) = control.as_mut() {
            if *rx.borrow() == TransferControl::Canceled { return Err("download canceled".into()); }
            if *rx.borrow() == TransferControl::Paused {
                // The previous supervisor, if any, has joined every worker. Sync
                // and persist the verified map before exposing the paused state.
                checkpoint.save(&map, &file, &store, &tasks).await?;
                last_bytes = checkpoint.record.segments.iter().map(|s| s.downloaded).sum();
                last_time = Instant::now();
                {
                    let mut guard = tasks.lock().await;
                    if *rx.borrow() == TransferControl::Paused {
                        let task = guard.get_mut(&url).ok_or("split task disappeared")?;
                        task.downloaded = last_bytes; task.speed_bps = 0; task.state = "paused".into();
                        task.active_connections = Some(0);
                    }
                }
                store.save(&tasks, &limiter).await.map_err(|e| e.to_string())?;
                if !control::wait_running(&tasks, &url, rx).await { return Err("download canceled".into()); }
                continue;
            }
            // A resume is acknowledged only while the control is still running.
            let mut guard = tasks.lock().await;
            if *rx.borrow() != TransferControl::Running { drop(guard); continue; }
            if let Some(task) = guard.get_mut(&url) {
                if task.state == "resuming" { task.state = "downloading".into(); }
            } else { return Err("split task disappeared".into()); }
        }
        if complete {
            checkpoint.save(&map, &file, &store, &tasks).await?;
            return Ok(());
        }
        let requested = tasks.lock().await.get(&url).and_then(|task| task.requested_connections).unwrap_or(workers as u8).clamp(1, 32);
        let (desired, desired_rx) = watch::channel(requested);
        let (stop, _) = watch::channel(false);
        let _stop_on_drop = StopOnDrop(stop.clone());
        let mut transfer = tokio::spawn(run_map(client.clone(), url.clone(), info.clone(), file.clone(), map.clone(), limiter.clone(), stop.clone(), requested as usize, Some(desired_rx)));
        let mut timer = tokio::time::interval(Duration::from_millis(250));
        let outcome = loop {
            tokio::select! {
                result = &mut transfer => break result.map_err(|e| format!("split supervisor failed: {e}"))?,
                _ = async {
                    match &mut control { Some(rx) => { let _ = rx.changed().await; }, None => std::future::pending::<()>().await }
                } => {
                    if control.as_ref().is_some_and(|rx| *rx.borrow() != TransferControl::Running) {
                        stop.send_replace(true);
                        break transfer.await.map_err(|e| format!("split supervisor failed: {e}"))?;
                    }
                }
                _ = timer.tick() => {
                    let (bytes, active, segments, rejected) = {
                        let map = map.lock().await;
                        (map.records().iter().map(|s| s.downloaded).sum(), map.active_count(), map.segment_count(), map.was_rejected())
                    };
                    if let Some(task) = tasks.lock().await.get_mut(&url) {
                        if task.partial_name.as_deref() == Some(partial_name.as_str()) && task.state == "downloading" {
                            progress.update(task, bytes);
                            if let Some(count) = task.requested_connections.filter(|count| (1..=32).contains(count)) {
                                if *desired.borrow() != count { desired.send_replace(count); }
                            }
                            task.active_connections = Some(active); task.segment_count = Some(segments);
                            if rejected { task.connection_note = Some("Server rejected a range request (HTTP 429/503); concurrency may have been reduced.".into()); }
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
        let (rejected, segments) = {
            let map = map.lock().await;
            (map.was_rejected(), map.segment_count())
        };
        if let Some(task) = tasks.lock().await.get_mut(&url) {
            task.segment_count = Some(segments);
            if rejected { task.connection_note = Some("Server rejected a range request (HTTP 429/503); concurrency may have been reduced.".into()); }
        }
        if control.as_ref().is_some_and(|rx| *rx.borrow() == TransferControl::Canceled) { return Err("download canceled".into()); }
        match outcome {
            Ok(()) => {
                complete = true;
                if let Some(task) = tasks.lock().await.get_mut(&url) { task.active_connections = Some(0); }
            },
            Err(error) if error == "split stopped" && control.as_ref().is_some_and(|rx| *rx.borrow() == TransferControl::Paused) => {},
            Err(error) => return Err(error),
        }
    }
}

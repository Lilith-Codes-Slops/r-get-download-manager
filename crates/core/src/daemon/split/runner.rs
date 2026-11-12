//! Coordinate split checkpoints and worker barriers across pause and cancellation.
use super::{checkpoint::SplitCheckpoint, recovery::PreparedSplit, run_map};
use crate::daemon::{
    bandwidth::BandwidthLimiter,
    control::{self, Tasks, TransferControl},
    persistence::Persistence,
    range::ResumeInfo,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

// If the owner future is dropped, request a stop. The independent supervisor
// keeps running until all worker tasks (and their blocking writes) have joined.
struct StopOnDrop(watch::Sender<bool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

/// Workers are stopped and joined before a pause is checkpointed or acknowledged.
pub(super) async fn run_checkpointed(
    prepared: PreparedSplit,
    mut checkpoint: SplitCheckpoint,
    client: reqwest::Client,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
    tasks: Tasks,
    mut control: Option<watch::Receiver<TransferControl>>,
) -> Result<(), String> {
    if control
        .as_ref()
        .is_some_and(|rx| *rx.borrow() == TransferControl::Canceled)
    {
        return Err("download canceled".into());
    }
    if prepared.record != checkpoint.record {
        return Err("checkpoint does not match prepared transfer".into());
    }
    let info = ResumeInfo {
        etag: prepared.record.etag.clone(),
        total: prepared.record.total,
        url: reqwest::Url::parse(&prepared.record.resolved_url).map_err(|e| e.to_string())?,
    };
    let workers = prepared.record.requested_connections as usize;
    let url = prepared.record.task_url.clone();
    let preserve_partial = tasks
        .lock()
        .await
        .get(&url)
        .map_or(true, |task| task.confirmed_filename.is_some());
    let partial_name = prepared.record.partial_name.clone();
    let map = prepared.map;
    let file = prepared.file;
    let origin = super::host_memory::origin(&info.url);
    let path = store.database_path();
    if let Some(key) = &origin {
        if let Some(memory) = super::host_memory::load(path.clone(), key.clone()).await {
            map.lock().await.seed_policy(memory);
        }
    }
    let mut memory_writer = super::host_memory::Writer::new(path, origin);
    checkpoint.save(&map, &file, &store, &tasks).await?;
    let publisher = Arc::new(crate::daemon::segment_events::Publisher::new(
        url.clone(),
        partial_name.clone(),
        &map,
    ));
    let changes = {
        let mut guard = map.lock().await;
        guard.attach_policy_publisher(Arc::downgrade(&publisher));
        guard.changes()
    };
    publisher.publish(map.lock().await.progress());
    publisher.publish_policy(map.lock().await.policy(), true);
    let mut policy_refresh = Instant::now();
    let mut last_time = Instant::now();
    let mut last_bytes: u64 = checkpoint
        .record
        .segments
        .iter()
        .map(|s| s.downloaded)
        .sum();
    let progress = crate::daemon::progress::ProgressMeter::new(last_bytes);
    let mut complete = false;
    loop {
        if let Some(rx) = control.as_mut() {
            if *rx.borrow() == TransferControl::Canceled {
                return Err("download canceled".into());
            }
            if *rx.borrow() == TransferControl::Paused {
                // The previous supervisor, if any, has joined every worker. Sync
                // and persist the verified map before exposing the paused state.
                map.lock().await.reset_stability();
                checkpoint.save(&map, &file, &store, &tasks).await?;
                publisher.publish(map.lock().await.progress());
                publisher.publish_policy(map.lock().await.policy(), false);
                last_bytes = checkpoint
                    .record
                    .segments
                    .iter()
                    .map(|s| s.downloaded)
                    .sum();
                last_time = Instant::now();
                {
                    let mut guard = tasks.lock().await;
                    if *rx.borrow() == TransferControl::Paused {
                        let task = guard.get_mut(&url).ok_or("split task disappeared")?;
                        task.downloaded = last_bytes;
                        task.speed_bps = 0;
                        task.state = "paused".into();
                        task.active_connections = Some(0);
                    }
                }
                store
                    .save(&tasks, &limiter)
                    .await
                    .map_err(|e| e.to_string())?;
                if !control::wait_running(&tasks, &url, rx).await {
                    return Err("download canceled".into());
                }
                continue;
            }
            // A resume is acknowledged only while the control is still running.
            let mut guard = tasks.lock().await;
            if *rx.borrow() != TransferControl::Running {
                drop(guard);
                continue;
            }
            if let Some(task) = guard.get_mut(&url) {
                if task.state == "resuming" {
                    task.state = "downloading".into();
                }
            } else {
                return Err("split task disappeared".into());
            }
        }
        if complete {
            checkpoint.save(&map, &file, &store, &tasks).await?;
            let now = Instant::now();
            memory_writer.consider(map.lock().await.good_memory(now), now);
            memory_writer.finish().await;
            return Ok(());
        }
        let requested = tasks
            .lock()
            .await
            .get(&url)
            .and_then(|task| task.requested_connections)
            .unwrap_or(workers as u8)
            .clamp(1, 32);
        let (desired, desired_rx) = watch::channel(requested);
        let (stop, _) = watch::channel(false);
        let _stop_on_drop = StopOnDrop(stop.clone());
        let mut transfer = tokio::spawn(run_map(
            client.clone(),
            url.clone(),
            info.clone(),
            file.clone(),
            map.clone(),
            limiter.clone(),
            stop.clone(),
            requested as usize,
            Some(desired_rx),
        ));
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
                _ = changes.notified() => {
                    let (active, segments, rejected) = {
                        let map = map.lock().await;
                        (map.active_count(), map.progress(), map.rejection_status())
                    };
                    publisher.publish(segments);
                    if let Some(task) = tasks.lock().await.get_mut(&url) {
                        if task.partial_name.as_deref() == Some(partial_name.as_str()) && matches!(task.state.as_str(), "downloading" | "pausing" | "resuming" | "canceling") {
                            task.active_connections = Some(active);
                            if let Some(status) = rejected { task.connection_note = Some(rejection_note(status)); }
                        }
                    }
                }
                _ = timer.tick() => {
                    let refresh = policy_refresh.elapsed() >= Duration::from_secs(5);
                    let (bytes, active, segments, rejected, progress_snapshot) = {
                        let map = map.lock().await;
                        // Publish under the map lock so a later worker update cannot be overwritten by this snapshot.
                        publisher.publish_policy(map.policy(), refresh);
                        let progress = map.progress();
                        (progress.iter().map(|s| s.downloaded).sum(), map.active_count(), map.segment_count(), map.rejection_status(), progress)
                    };
                    if refresh { policy_refresh = Instant::now(); }
                    let now = Instant::now(); memory_writer.consider(map.lock().await.good_memory(now), now);
                    publisher.publish(progress_snapshot);
                    if let Some(task) = tasks.lock().await.get_mut(&url) {
                        if task.partial_name.as_deref() == Some(partial_name.as_str()) && task.state == "downloading" {
                            progress.update(task, bytes);
                            if let Some(count) = task.requested_connections.filter(|count| (1..=32).contains(count)) {
                                if *desired.borrow() != count { desired.send_replace(count); }
                            }
                            task.active_connections = Some(active); task.segment_count = Some(segments);
                            if let Some(status) = rejected { task.connection_note = Some(rejection_note(status)); }
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
        publisher.publish(map.lock().await.progress());
        publisher.publish_policy(map.lock().await.policy(), false);
        let (rejected, segments) = {
            let map = map.lock().await;
            (map.rejection_status(), map.segment_count())
        };
        if let Some(task) = tasks.lock().await.get_mut(&url) {
            task.segment_count = Some(segments);
            if let Some(status) = rejected {
                task.connection_note = Some(rejection_note(status));
            }
        }
        if control
            .as_ref()
            .is_some_and(|rx| *rx.borrow() == TransferControl::Canceled)
        {
            if preserve_partial {
                if let Err(error) = checkpoint.save(&map, &file, &store, &tasks).await {
                    eprintln!("could not save retained split checkpoint: {error}");
                }
            }
            return Err("download canceled".into());
        }
        if preserve_partial && outcome.is_err() {
            if let Err(error) = checkpoint.save(&map, &file, &store, &tasks).await {
                eprintln!("could not save retained split checkpoint: {error}");
            }
        }
        match outcome {
            Ok(()) => {
                complete = true;
                if let Some(task) = tasks.lock().await.get_mut(&url) {
                    task.active_connections = Some(0);
                }
            }
            Err(error)
                if error == "split stopped"
                    && control
                        .as_ref()
                        .is_some_and(|rx| *rx.borrow() == TransferControl::Paused) => {}
            Err(error) => return Err(error),
        }
    }
}

fn rejection_note(status: u16) -> String {
    format!("Server rejected a range request (HTTP {status}); the engine reduces concurrency or retries. See active workers for the current count.")
}

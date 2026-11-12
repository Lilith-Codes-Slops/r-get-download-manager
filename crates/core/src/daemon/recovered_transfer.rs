//! Complete a previously validated single-stream partial without replacing files.
use super::bandwidth::BandwidthLimiter;
use super::control::{wait_running, Controls, Tasks, TransferControl};
use super::recovery::PreparedRecovery;
use super::{checkpoint::Checkpoint, persistence::Persistence, range::ResumeInfo, transfer_body};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn run(
    item: PreparedRecovery,
    tasks: Tasks,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
    mut control: watch::Receiver<TransferControl>,
) -> Result<PathBuf> {
    let url = item.record.task_url.clone();
    // If the task disappeared, retain rather than guessing ownership.
    let preserve_partial = tasks
        .lock()
        .await
        .get(&url)
        .map_or(true, |task| task.confirmed_filename.is_some());
    let final_path = item
        .partial
        .parent()
        .ok_or("partial file has no parent")?
        .join(&item.record.filename);
    let info = ResumeInfo {
        etag: item.record.etag.clone(),
        total: item.record.total,
        url: reqwest::Url::parse(&item.record.resolved_url)?,
    };
    let offset = item.record.downloaded;
    let mut checkpoint = Checkpoint::restore(&item.partial, item.record, item.hash);
    checkpoint.attach_persistence(store, tasks.clone());
    let mut file = tokio::fs::File::from_std(item.file);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    if offset == info.total && offset > 0 {
        loop {
            if !wait_running(&tasks, &url, &mut control).await {
                drop(file);
                if !preserve_partial {
                    let _ = tokio::fs::remove_file(&item.partial).await;
                    let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
                }
                return Err("download canceled".into());
            }
            let request = client
                .get(&url)
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .header(reqwest::header::RANGE, "bytes=0-0")
                .header(reqwest::header::IF_RANGE, info.etag.as_str());
            let response = tokio::select! {
                response = request.send() => Some(response?),
                changed = control.changed() => { if changed.is_err() { return Err("download canceled".into()); } None },
            };
            let Some(response) = response else { continue };
            super::range::validate_probe(&response, &info).map_err(|error| {
                format!("cannot resume automatically: {error}; partial retained")
            })?;
            break;
        }
    }
    let result = transfer_body::receive(
        &client,
        None,
        &url,
        &mut file,
        &tasks,
        &limiter,
        &mut control,
        Some(&info),
        Some(info.total),
        Some(&mut checkpoint),
        offset,
    )
    .await;
    if let Err(error) = result {
        if preserve_partial {
            if let Err(checkpoint_error) = checkpoint.save(&file, true).await {
                eprintln!("could not save retained transfer checkpoint: {checkpoint_error}");
            }
        }
        drop(file);
        let canceled = *control.borrow() == TransferControl::Canceled;
        if canceled && !preserve_partial {
            let _ = tokio::fs::remove_file(&item.partial).await;
            let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
        }
        return Err(error);
    }
    if let Err(error) = checkpoint.save(&file, true).await {
        drop(file);
        let canceled = *control.borrow() == TransferControl::Canceled;
        if canceled && !preserve_partial {
            let _ = tokio::fs::remove_file(&item.partial).await;
            let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
        }
        return Err(error);
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let canceled = *control.borrow() == TransferControl::Canceled;
        if canceled && !preserve_partial {
            let _ = tokio::fs::remove_file(&item.partial).await;
            let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
        }
        return Err(error.into());
    }
    drop(file);
    let mut guard = loop {
        if !wait_running(&tasks, &url, &mut control).await {
            if !preserve_partial {
                let _ = tokio::fs::remove_file(&item.partial).await;
                let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
            }
            return Err("download canceled".into());
        }
        let mut guard = tasks.lock().await;
        let state = *control.borrow();
        if state == TransferControl::Canceled || !guard.contains_key(&url) {
            drop(guard);
            if !preserve_partial {
                let _ = tokio::fs::remove_file(&item.partial).await;
                let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
            }
            return Err("download canceled".into());
        }
        if state == TransferControl::Paused {
            drop(guard);
            continue;
        }
        if let Some(task) = guard.get_mut(&url) {
            task.state = "finalizing".into();
        }
        break guard;
    };
    let confirmed = guard
        .get(&url)
        .and_then(|task| task.confirmed_filename.as_deref());
    if confirmed.is_some_and(|name| final_path.file_name().and_then(|s| s.to_str()) != Some(name)) {
        return Err(
            "checkpoint filename differs from the confirmed filename; partial retained".into(),
        );
    }
    let final_path =
        super::collision::publish_for_task(&item.partial, &final_path, confirmed.is_some()).await?;
    if let Some(task) = guard.get_mut(&url) {
        task.filename = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned);
        task.destination_path = Some(final_path.to_string_lossy().into_owned());
    }
    drop(guard);
    let _ = tokio::fs::remove_file(&item.partial).await;
    let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
    Ok(final_path)
}

/// Install the control channel before accepting IPC commands, then run recovery.
pub(super) async fn spawn(
    item: PreparedRecovery,
    tasks: Tasks,
    controls: Controls,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
) {
    let task_id = item.record.task_url.clone();
    let initial = if item.paused {
        TransferControl::Paused
    } else {
        TransferControl::Running
    };
    let (sender, receiver) = watch::channel(initial);
    controls.lock().await.insert(task_id.clone(), sender);
    tokio::spawn(async move {
        let result = run(
            item,
            tasks.clone(),
            limiter.clone(),
            store.clone(),
            receiver.clone(),
        )
        .await;
        let mut guard = tasks.lock().await;
        if let Some(task) = guard.get_mut(&task_id) {
            task.speed_bps = 0;
            task.active_connections = Some(0);
            if *receiver.borrow() == TransferControl::Canceled {
                task.state = "canceled".into();
                task.resume_supported = false;
            } else {
                match result {
                    Ok(path) => {
                        task.state = "completed".into();
                        task.final_path = Some(path.to_string_lossy().into_owned());
                        task.partial_name = None;
                    }
                    Err(e) => {
                        task.state = format!("failed: {e}");
                        task.resume_supported = false;
                    }
                }
            }
        }
        drop(guard);
        #[cfg(target_os = "linux")]
        super::progress_events::notify();
        if let Err(e) = store.save(&tasks, &limiter).await {
            eprintln!("queue checkpoint failed: {e}");
        }
    });
}

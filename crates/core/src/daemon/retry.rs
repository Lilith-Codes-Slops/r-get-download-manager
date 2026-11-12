//! Decide whether a stopped, task-owned partial is safe to resume.
use super::{
    bandwidth::BandwidthLimiter,
    control::{Tasks, TransferControl},
    persistence::{Persistence, SavedState},
};
use downloader_ipc::DownloadInfo;
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

enum Decision {
    Single(Box<super::recovery::PreparedRecovery>),
    Split(Box<super::split::recovery::PreparedSplit>),
    Fresh,
}

fn decide(saved: &mut SavedState, root: &Path, prior: &DownloadInfo) -> Decision {
    let Some(name) = prior.partial_name.as_deref() else {
        return Decision::Fresh;
    };
    let mut parts = Path::new(name).components();
    if name.contains('\\')
        || !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || !name.starts_with(".rget-")
        || !name.ends_with(".part")
    {
        return Decision::Fresh;
    }
    let partial = root.join(name);
    let sidecar = root.join(format!("{name}.meta"));
    let db = saved
        .split_checkpoints
        .get(&prior.url)
        .filter(|record| record.partial_name == name);
    if let Ok(item) = super::split::recovery::prepare(&sidecar, &partial, &prior.url, db) {
        if prior.confirmed_filename.is_none()
            || super::http_partial_owner::verify(
                saved.http_partial_owners.get(&prior.url),
                name,
                &item.file,
            )
            .is_ok()
        {
            return Decision::Split(Box::new(item));
        }
        return Decision::Fresh;
    }
    saved.tasks.insert(prior.url.clone(), prior.clone());
    match super::recovery::prepare_one(saved, root, &prior.url, "downloading") {
        Ok(item) => Decision::Single(Box::new(item)),
        Err(_) => Decision::Fresh,
    }
}

pub(super) async fn run(
    url: &str,
    connections: u8,
    prior: Option<DownloadInfo>,
    tasks: Tasks,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
    control: watch::Receiver<TransferControl>,
) -> Result<PathBuf> {
    let mut decision = Decision::Fresh;
    let root = store.task_directory(url).await?;
    if let Some(old) = prior.as_ref().filter(|task| {
        task.state != "completed" && (task.state != "canceled" || task.confirmed_filename.is_some())
    }) {
        let mut saved = store.load().await?;
        let task_root = root.clone();
        let old = old.clone();
        let database = store.database_path();
        decision = tokio::task::spawn_blocking(move || -> Result<Decision> {
            if old.confirmed_filename.is_some() {
                let Some(name) = old.partial_name.as_deref() else {
                    return Ok(Decision::Fresh);
                };
                let anchor = super::task_directory::HttpRoot::open(&database, &old.url)?;
                let anchored_partial = anchor.child(&task_root.join(name))?;
                let anchored_root = anchored_partial.parent().ok_or("partial has no root")?;
                let mut selected = decide(&mut saved, anchored_root, &old);
                anchor.check()?;
                if let Decision::Single(item) = &mut selected {
                    item.partial = task_root.join(&item.record.partial_name);
                }
                Ok(selected)
            } else {
                Ok(decide(&mut saved, &task_root, &old))
            }
        }).await??;
    }
    if let Some(old) = prior
        .as_ref()
        .filter(|task| task.confirmed_filename.is_some())
    {
        let expected = old.confirmed_filename.as_deref();
        match &decision {
            Decision::Single(item) if Some(item.record.filename.as_str()) != expected => {
                return Err(
                    "cannot resume automatically: checkpoint filename differs from the confirmed filename; partial retained"
                        .into(),
                );
            }
            Decision::Split(item) if Some(item.record.filename.as_str()) != expected => {
                return Err(
                    "cannot resume automatically: checkpoint filename differs from the confirmed filename; partial retained"
                        .into(),
                );
            }
            Decision::Fresh => {
                return Err("cannot resume automatically: no verifiable task-owned partial; retained data was not changed; restart from beginning requires confirmation".into());
            }
            _ => {}
        }
    }
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == TransferControl::Canceled {
            return Err("download canceled".into());
        }
        let task = guard.get_mut(url).ok_or("task was removed")?;
        match &decision {
            Decision::Single(item) => {
                task.filename = Some(item.record.filename.clone());
                task.destination_path = Some(
                    root.join(&item.record.filename)
                        .to_string_lossy()
                        .into_owned(),
                );
                task.partial_name = Some(item.record.partial_name.clone());
                task.downloaded = item.record.downloaded;
                task.total = Some(item.record.total);
                task.resume_supported = true;
                task.active_connections = Some(1);
            }
            Decision::Split(item) => {
                task.filename = Some(item.record.filename.clone());
                task.destination_path = Some(
                    root.join(&item.record.filename)
                        .to_string_lossy()
                        .into_owned(),
                );
                task.partial_name = Some(item.record.partial_name.clone());
                task.downloaded = item
                    .record
                    .segments
                    .iter()
                    .map(|segment| segment.downloaded)
                    .sum();
                task.total = Some(item.record.total);
                task.resume_supported = true;
                task.live_connections_supported = true;
                task.segment_count = Some(item.record.segments.len() as u16);
                task.active_connections = Some(0);
            }
            Decision::Fresh => {
                task.partial_name = None;
                task.filename = None;
                task.downloaded = 0;
                task.total = None;
            }
        }
    }
    match decision {
        Decision::Single(item) => {
            super::recovered_transfer::run(*item, tasks, limiter, store, control).await
        }
        Decision::Split(item) => {
            let partial = root.join(&item.record.partial_name);
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .read_timeout(std::time::Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()?;
            super::split::live::run(*item, partial, client, tasks, limiter, store, control).await
        }
        Decision::Fresh => {
            super::transfer::download_file(
                url,
                connections,
                &root,
                tasks,
                limiter,
                store,
                prior.is_some_and(|task| task.state != "completed"),
                control,
            )
            .await
        }
    }
}

/// Requeue interrupted tasks that could not be reconciled, preserving the old
/// partial as evidence until the common decision has inspected it.
pub(super) fn startup_fallbacks(
    saved: &mut SavedState,
    old: &std::collections::BTreeMap<String, DownloadInfo>,
    states: &std::collections::HashMap<String, String>,
) -> Vec<(DownloadInfo, bool)> {
    let mut pending = Vec::new();
    for (id, state) in states {
        let Some(task) = saved.tasks.get_mut(id) else {
            continue;
        };
        if !task.state.starts_with("failed: recovery unavailable:") {
            continue;
        }
        let Some(previous) = old.get(id) else {
            continue;
        };
        if previous.confirmed_filename.is_some() && previous.partial_name.is_none() {
            task.state = "failed: cannot resume automatically: no task-owned partial is recorded; restart from beginning requires confirmation".into();
            task.resume_supported = false;
            continue;
        }
        let paused = matches!(state.as_str(), "paused" | "pausing");
        task.state = if paused { "paused" } else { "downloading" }.into();
        task.partial_name = previous.partial_name.clone();
        task.filename = previous.filename.clone();
        pending.push((previous.clone(), paused));
    }
    pending
}

pub(super) async fn spawn_restart(
    prior: DownloadInfo,
    paused: bool,
    tasks: Tasks,
    controls: super::control::Controls,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
) {
    let id = prior.url.clone();
    let connections = prior.requested_connections.unwrap_or(1).clamp(1, 32);
    let (sender, receiver) = watch::channel(if paused {
        TransferControl::Paused
    } else {
        TransferControl::Running
    });
    controls.lock().await.insert(id.clone(), sender);
    tokio::spawn(async move {
        let result = run(
            &id,
            connections,
            Some(prior),
            tasks.clone(),
            limiter.clone(),
            store.clone(),
            receiver.clone(),
        )
        .await;
        let mut guard = tasks.lock().await;
        if let Some(task) = guard.get_mut(&id) {
            task.speed_bps = 0;
            task.active_connections = Some(0);
            task.live_connections_supported = false;
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
                    Err(error) => {
                        task.state = format!("failed: {error}");
                        task.resume_supported = false;
                    }
                }
            }
        }
        drop(guard);
        super::progress_events::notify();
        if let Err(error) = store.save(&tasks, &limiter).await {
            eprintln!("queue checkpoint failed: {error}");
        }
    });
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

//! Opt-in, bounded progress stream. A single sampler orders all task changes.
use super::{
    control::Tasks, event_broadcaster::EventBroadcaster, post_download::PostDownloadState,
};
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
use tokio::{net::UnixStream, sync::Notify};

struct Hub {
    latest: BTreeMap<String, DownloadInfo>,
    events: EventBroadcaster,
    post_download: PostDownloadState,
    actions_ready: bool,
}
struct Shared {
    hub: Mutex<Hub>,
    changed: Notify,
}
fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        hub: Mutex::new(Hub {
            latest: BTreeMap::new(),
            events: EventBroadcaster::with_capacity(1024),
            post_download: PostDownloadState::default(),
            actions_ready: false,
        }),
        changed: Notify::new(),
    })
}
pub(super) fn notify() {
    shared().changed.notify_one();
}

fn completed_paths(
    previous: &BTreeMap<String, DownloadInfo>,
    snapshot: &[DownloadInfo],
) -> Vec<(String, String)> {
    snapshot
        .iter()
        .filter(|task| task.state == "completed")
        .filter_map(|task| {
            let path = task.final_path.as_ref()?;
            let already = previous.get(&task.url).is_some_and(|prev| {
                prev.state == "completed" && prev.final_path.as_ref() == Some(path)
            });
            (!already).then(|| (task.url.clone(), path.clone()))
        })
        .collect()
}

fn publish(snapshot: Vec<DownloadInfo>) {
    let (completions, settings, trigger) = {
        let mut hub = shared().hub.lock().unwrap_or_else(|e| e.into_inner());
        let trigger = hub.actions_ready;
        let completions = if trigger {
            completed_paths(&hub.latest, &snapshot)
        } else {
            Vec::new()
        };
        let settings = hub.post_download.get();
        let mut next = BTreeMap::new();
        for task in snapshot {
            if hub.latest.get(&task.url) != Some(&task) {
                hub.events.send(DaemonEvent::ProgressUpdate {
                    task_id: task.url.clone(),
                    task: Box::new(task.clone()),
                });
            }
            next.insert(task.url.clone(), task);
        }
        for id in hub.latest.keys().filter(|id| !next.contains_key(*id)) {
            hub.events.send(DaemonEvent::ProgressRemoved {
                task_id: id.clone(),
            });
        }
        hub.latest = next;
        hub.actions_ready = true;
        (completions, settings, trigger)
    };
    if !trigger || !settings.open_containing_folder || completions.is_empty() {
        return;
    }
    for (task_id, path) in completions {
        tokio::task::spawn_blocking(move || {
            match super::post_download::open_containing_folder(&path) {
                Ok(()) => tracing::info!(task_id = %task_id, "opened containing folder"),
                Err(error) => {
                    tracing::warn!(task_id = %task_id, %error, "post-download action failed");
                }
            }
        });
    }
}

pub(super) fn start(
    tasks: Tasks,
    post_download: PostDownloadState,
    initial: Vec<DownloadInfo>,
) -> tokio::task::JoinHandle<()> {
    {
        let mut hub = shared().hub.lock().unwrap_or_else(|e| e.into_inner());
        hub.latest = initial
            .into_iter()
            .map(|task| (task.url.clone(), task))
            .collect();
        hub.post_download = post_download;
        hub.actions_ready = true;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            tokio::select! { _ = interval.tick() => {}, _ = shared().changed.notified() => {} }
            let mut snapshot: Vec<_> = tasks.lock().await.values().cloned().collect();
            for task in &mut snapshot {
                task.destination_path = super::destination::path(task);
                if let Some(active) =
                    super::segment_events::active_count(&task.url, task.partial_name.as_deref())
                        .await
                {
                    task.active_connections = Some(active);
                }
            }
            publish(snapshot);
        }
    })
}

pub(super) async fn subscribe(mut stream: UnixStream) {
    let (mut rx, initial) = {
        let hub = shared().hub.lock().unwrap_or_else(|e| e.into_inner());
        (
            hub.events.subscribe(),
            hub.latest.values().cloned().collect::<Vec<_>>(),
        )
    };
    if super::segment_events::send(&mut stream, &DaemonEvent::ProgressReset)
        .await
        .is_err()
    {
        return;
    }
    for task in initial {
        if super::segment_events::send(
            &mut stream,
            &DaemonEvent::ProgressUpdate {
                task_id: task.url.clone(),
                task: Box::new(task),
            },
        )
        .await
        .is_err()
        {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let event = tokio::select! {
            event = rx.recv() => match event { Ok(event) => event, Err(_) => return },
            _ = heartbeat.tick() => DaemonEvent::ProgressHeartbeat,
        };
        if super::segment_events::send(&mut stream, &event)
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(url: &str, state: &str, final_path: Option<&str>) -> DownloadInfo {
        serde_json::from_value(json!({
            "url": url,
            "state": state,
            "filename": null,
            "downloaded": 0,
            "total": 10,
            "speed_bps": 0,
            "final_path": final_path,
        }))
        .unwrap()
    }

    #[test]
    fn completed_paths_detects_only_new_completions() {
        let first = task("a", "downloading", None);
        let completed = task("a", "completed", Some("/home/u/Downloads/a.bin"));
        let snapshot = vec![completed.clone()];
        assert_eq!(
            completed_paths(&BTreeMap::new(), &snapshot),
            vec![("a".to_string(), "/home/u/Downloads/a.bin".to_string())]
        );
        let previous = BTreeMap::from([("a".to_string(), completed.clone())]);
        assert!(completed_paths(&previous, &snapshot).is_empty());
        let previous = BTreeMap::from([("a".to_string(), first)]);
        assert_eq!(completed_paths(&previous, &snapshot).len(), 1);
    }

    #[test]
    fn completed_paths_ignores_incomplete_and_missing_paths() {
        let snapshot = vec![
            task("a", "downloading", Some("/home/u/Downloads/a.bin")),
            task("b", "completed", None),
        ];
        assert!(completed_paths(&BTreeMap::new(), &snapshot).is_empty());
    }

    #[test]
    fn completed_paths_reports_a_new_final_path() {
        let old = task("a", "completed", Some("/home/u/Downloads/a.bin"));
        let new = task("a", "completed", Some("/home/u/Downloads/a (2).bin"));
        let previous = BTreeMap::from([("a".to_string(), old)]);
        assert_eq!(
            completed_paths(&previous, &[new]),
            vec![("a".to_string(), "/home/u/Downloads/a (2).bin".to_string())]
        );
    }
}

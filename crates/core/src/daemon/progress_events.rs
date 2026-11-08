//! Opt-in, bounded progress stream. A single sampler orders all task changes.
use super::{control::Tasks, event_broadcaster::EventBroadcaster};
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::{collections::BTreeMap, sync::{Mutex, OnceLock}};
use tokio::{net::UnixStream, sync::Notify};

struct Hub { latest: BTreeMap<String, DownloadInfo>, events: EventBroadcaster }
struct Shared { hub: Mutex<Hub>, changed: Notify }
fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared { hub: Mutex::new(Hub { latest: BTreeMap::new(), events: EventBroadcaster::with_capacity(1024) }), changed: Notify::new() })
}
pub(super) fn notify() { shared().changed.notify_one(); }

fn publish(snapshot: Vec<DownloadInfo>) {
    let mut hub = shared().hub.lock().unwrap_or_else(|e| e.into_inner());
    let mut next = BTreeMap::new();
    for task in snapshot {
        if hub.latest.get(&task.url) != Some(&task) { hub.events.send(DaemonEvent::ProgressUpdate { task_id: task.url.clone(), task: task.clone() }); }
        next.insert(task.url.clone(), task);
    }
    for id in hub.latest.keys().filter(|id| !next.contains_key(*id)) {
        hub.events.send(DaemonEvent::ProgressRemoved { task_id: id.clone() });
    }
    hub.latest = next;
}

pub(super) fn start(tasks: Tasks) -> tokio::task::JoinHandle<()> {
    shared().hub.lock().unwrap_or_else(|e| e.into_inner()).latest.clear();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            tokio::select! { _ = interval.tick() => {}, _ = shared().changed.notified() => {} }
            let mut snapshot: Vec<_> = tasks.lock().await.values().cloned().collect();
            for task in &mut snapshot {
                task.destination_path = super::destination::path(task);
                if let Some(active) = super::segment_events::active_count(&task.url, task.partial_name.as_deref()).await { task.active_connections = Some(active); }
            }
            publish(snapshot);
        }
    })
}

pub(super) async fn subscribe(mut stream: UnixStream) {
    let (mut rx, initial) = {
        let hub = shared().hub.lock().unwrap_or_else(|e| e.into_inner());
        (hub.events.subscribe(), hub.latest.values().cloned().collect::<Vec<_>>())
    };
    if super::segment_events::send(&mut stream, &DaemonEvent::ProgressReset).await.is_err() { return; }
    for task in initial {
        if super::segment_events::send(&mut stream, &DaemonEvent::ProgressUpdate { task_id: task.url.clone(), task }).await.is_err() { return; }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let event = tokio::select! {
            event = rx.recv() => match event { Ok(event) => event, Err(_) => return },
            _ = heartbeat.tick() => DaemonEvent::ProgressHeartbeat,
        };
        if super::segment_events::send(&mut stream, &event).await.is_err() { return; }
    }
}

use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{watch, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransferControl { Running, Paused, Canceled }
pub(super) type Tasks = Arc<Mutex<BTreeMap<String, DownloadInfo>>>;
pub(super) type Controls = Arc<Mutex<BTreeMap<String, watch::Sender<TransferControl>>>>;

pub(super) async fn set_paused(tasks: &Tasks, controls: &Controls, id: String, pause: bool) -> DaemonEvent {
    let mut tasks = tasks.lock().await;
    let Some(task) = tasks.get_mut(&id) else {
        return DaemonEvent::TaskFailed { task_id: id, error: "task not found".into() };
    };
    let allowed = if pause { task.state == "downloading" && task.resume_supported } else { task.state == "paused" };
    if !allowed {
        return DaemonEvent::TaskFailed { task_id: id, error: if pause {
            "task is not downloading or safe range resume is unavailable".into()
        } else { "task is not paused".into() } };
    }
    let controls = controls.lock().await;
    let Some(sender) = controls.get(&id) else {
        return DaemonEvent::TaskFailed { task_id: id, error: "transfer control unavailable".into() };
    };
    let state = if pause { "pausing" } else { "resuming" };
    task.state = state.into();
    task.speed_bps = 0;
    sender.send_replace(if pause { TransferControl::Paused } else { TransferControl::Running });
    DaemonEvent::TaskStateChanged { task_id: id, state: state.into() }
}

/// Returns false on cancellation or closed control channel.
pub(super) async fn wait_running(tasks: &Tasks, id: &str, control: &mut watch::Receiver<TransferControl>) -> bool {
    loop {
        let value = *control.borrow_and_update();
        if value == TransferControl::Canceled { return false; }
        {
            let mut guard = tasks.lock().await;
            if *control.borrow() != value { continue; }
            let Some(task) = guard.get_mut(id) else { return false };
            match value {
                TransferControl::Running => {
                    if task.state == "resuming" { task.state = "downloading".into(); }
                    return true;
                }
                TransferControl::Paused => { task.state = "paused".into(); task.speed_bps = 0; }
                TransferControl::Canceled => return false,
            }
        }
        if control.changed().await.is_err() { return false; }
    }
}

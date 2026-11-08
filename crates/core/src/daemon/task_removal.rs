//! Cancel a transfer before removing its record and, optionally, its owned files.
use super::bandwidth::BandwidthLimiter;
use super::control::{Controls, Tasks, TransferControl};
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn active(state: &str) -> bool {
    matches!(state, "downloading" | "paused" | "pausing" | "resuming")
}

pub(super) async fn cancel(tasks: &Tasks, controls: &Controls, id: String) -> DaemonEvent {
    let mut guard = tasks.lock().await;
    let Some(task) = guard.get_mut(&id) else { return failed(id, "task not found"); };
    if task.state == "canceling" || task.state == "canceled" {
        return DaemonEvent::TaskStateChanged { task_id: id, state: task.state.clone() };
    }
    if !active(&task.state) { return failed(id, "task is not an active download"); }
    let senders = controls.lock().await;
    let Some(sender) = senders.get(&id) else { return failed(id, "transfer control unavailable"); };
    task.state = "canceling".into();
    task.speed_bps = 0;
    sender.send_replace(TransferControl::Canceled);
    DaemonEvent::TaskStateChanged { task_id: id, state: "canceling".into() }
}

pub(super) async fn remove(tasks: &Tasks, controls: &Controls, limiter: &Arc<BandwidthLimiter>, id: String, delete_file: bool) -> DaemonEvent {
    // Wait for the worker to close/clean its partial before unlinking or allowing
    // a new transfer with the same URL. Do not hold the task lock while waiting.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let mut guard = tasks.lock().await;
        let Some(task) = guard.get(&id) else { return failed(id, "task not found"); };
        if active(&task.state) {
            drop(guard);
            if let DaemonEvent::TaskFailed { error, .. } = cancel(tasks, controls, id.clone()).await {
                // The transfer may have completed between observing and canceling.
                if error != "task is not an active download" { return failed(id, &error); }
            }
        } else if matches!(task.state.as_str(), "canceling" | "finalizing") {
            drop(guard);
        } else {
            if delete_file {
                if let Err(error) = remove_owned_files(task).await { return failed(id, &error); }
            }
            guard.remove(&id);
            controls.lock().await.remove(&id);
            limiter.remove(&id).await;
            return DaemonEvent::TaskStateChanged { task_id: id, state: "removed".into() };
        }
        if tokio::time::Instant::now() >= deadline { return failed(id, "timed out waiting for the transfer to stop; task was not removed"); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn failed(id: String, error: &str) -> DaemonEvent {
    DaemonEvent::TaskFailed { task_id: id, error: error.into() }
}

fn single_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.contains('/') && !name.contains('\\') && matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

async fn unlink_if_present(path: &Path) -> Result<(), String> {
    match tokio::fs::symlink_metadata(path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not inspect {}: {e}", path.display())),
        Ok(meta) if !meta.is_file() => Err(format!("refusing to delete non-regular file: {}", path.display())),
        Ok(_) => match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not delete {}: {e}", path.display())),
        },
    }
}

async fn remove_owned_files(task: &DownloadInfo) -> Result<(), String> {
    if task.final_path.is_none() && task.partial_name.is_none() { return Ok(()); }
    let root = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
    remove_files_in(task, &root).await
}

async fn remove_files_in(task: &DownloadInfo, root: &Path) -> Result<(), String> {
    if let Some(final_path) = &task.final_path {
        let name = task.filename.as_deref().filter(|name| single_name(name)).ok_or("unsafe or missing final filename")?;
        let expected = root.join(name);
        if Path::new(final_path) != expected { return Err("refusing to delete a file outside the task's Downloads path".into()); }
        unlink_if_present(&expected).await?;
    }
    if let Some(name) = &task.partial_name {
        if !single_name(name) || !name.starts_with(".rget-") || !name.ends_with(".part") {
            return Err("refusing to delete an unsafe partial filename".into());
        }
        let part = root.join(name);
        unlink_if_present(&part).await?;
        let sidecar = root.join(format!("{name}.meta"));
        unlink_if_present(&sidecar).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_outside_paths_and_does_not_delete_unowned_final_files() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-delete-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join("existing.bin");
        tokio::fs::write(&path, b"unrelated").await.unwrap();
        let mut task = DownloadInfo { url: "https://example.test".into(), state: "failed".into(),
            filename: Some("existing.bin".into()), downloaded: 0, total: None, speed_bps: 0,
            final_path: None, resume_supported: false, partial_name: None, requested_connections: None, active_connections: None, segment_count: None, connection_note: None };
        remove_files_in(&task, &root).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"unrelated");
        task.final_path = Some(root.join("../outside").to_string_lossy().into_owned());
        assert!(remove_files_in(&task, &root).await.is_err());
        task.final_path = None;
        task.partial_name = Some(".rget-../outside.part".into());
        assert!(remove_files_in(&task, &root).await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"unrelated");
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

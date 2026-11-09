//! Route multi-file removal without changing legacy HTTP/single-file semantics.
use super::{bandwidth::BandwidthLimiter, control::{Controls, Tasks}, persistence::Persistence};
use downloader_ipc::DaemonEvent;
use std::{path::Path, sync::Arc};

pub(super) async fn remove(tasks: &Tasks, controls: &Controls, limiter: &Arc<BandwidthLimiter>, store: &Persistence,
    id: String, delete_file: bool) -> DaemonEvent {
    let root = std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join("Downloads"));
    remove_at_root(tasks, controls, limiter, store, id, delete_file, root.as_deref()).await
}

async fn remove_at_root(tasks: &Tasks, controls: &Controls, limiter: &Arc<BandwidthLimiter>, store: &Persistence,
    id: String, delete_file: bool, root: Option<&Path>) -> DaemonEvent {
    #[cfg(feature = "torrent")]
    {
        let task = tasks.lock().await.get(&id).cloned();
        if let Some(task) = task.filter(|task| task.torrent.is_some()) {
            let database = store.database_path(); let lookup_id = id.clone();
            let tracked = tokio::task::spawn_blocking(move || -> Result<bool, String> {
                let stages = super::torrent_stage_store::load(&database).map_err(|e| e.to_string())?;
                let published = super::torrent_published_store::load(&database, &lookup_id).map_err(|e| e.to_string())?;
                Ok(published.is_some() || stages.iter().any(|stage| stage.task_id == lookup_id))
            }).await;
            let tracked = match tracked {
                Ok(Ok(tracked)) => tracked,
                Ok(Err(error)) => return failed(id, &format!("cannot inspect torrent ownership: {error}")),
                Err(error) => return failed(id, &format!("torrent ownership lookup failed: {error}")),
            };
            if tracked || task.torrent.as_ref().is_some_and(|identity| identity.root_name.is_some()) {
                // Do not wait on a worker while holding its lifecycle lock; no
                // multi-file worker is enabled yet. Refuse until it has stopped.
                if !(task.state == "completed" || task.state == "canceled" || task.state.starts_with("failed:")) {
                    return failed(id, "stop the multi-file torrent before removing it");
                }
                let result = if delete_file {
                    if task.state != "completed" { return failed(id, "deletion of an unfinished multi-file torrent is not yet available"); }
                    let Some(root) = root else { return failed(id, "HOME is not set"); };
                    store.remove_completed_torrent(tasks, &id, root).await
                } else { store.forget_torrent_keep_files(tasks, &id).await };
                return match result {
                    Ok(()) => {
                        controls.lock().await.remove(&id); limiter.remove(&id).await;
                        super::progress_events::notify();
                        DaemonEvent::TaskStateChanged { task_id: id, state: "removed".into(), destination_path: None }
                    }
                    Err(error) => failed(id, &error.to_string()),
                };
            }
        }
    }
    #[cfg(not(feature = "torrent"))]
    let _ = (store, root);
    super::task_removal::remove(tasks, controls, limiter, id, delete_file).await
}

#[cfg(feature = "torrent")]
fn failed(id: String, error: &str) -> DaemonEvent { DaemonEvent::TaskFailed { task_id: id, error: error.into() } }

#[cfg(all(test, feature = "torrent", target_env = "gnu"))]
mod tests {
    use super::*;
    use super::super::{torrent_publish_commit_tests, torrent_publish_intent as journal};
    #[tokio::test]
    async fn routed_keep_and_delete_use_owned_multi_file_paths() {
        for delete in [false, true] {
            let lab = torrent_publish_commit_tests::setup().await; let database = lab.store.database_path();
            let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap();
            torrent_publish_commit_tests::rename(&lab);
            lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
            let controls: Controls = Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
            let limiter = Arc::new(BandwidthLimiter::new());
            let event = remove_at_root(&lab.tasks, &controls, &limiter, &lab.store, "task".into(), delete, Some(&lab.downloads)).await;
            assert!(matches!(event, DaemonEvent::TaskStateChanged { state, .. } if state == "removed"));
            assert_eq!(lab.downloads.join("bundle").exists(), !delete);
            assert!(!lab.tasks.lock().await.contains_key("task"));
            std::fs::remove_dir_all(lab.root).unwrap();
        }
    }

    #[tokio::test]
    async fn tracked_active_torrent_is_not_removed_by_legacy_fallback() {
        let lab = torrent_publish_commit_tests::setup().await;
        let controls: Controls = Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
        let limiter = Arc::new(BandwidthLimiter::new());
        let event = remove_at_root(&lab.tasks, &controls, &limiter, &lab.store, "task".into(), true, Some(&lab.downloads)).await;
        assert!(matches!(event, DaemonEvent::TaskFailed { .. }));
        assert!(lab.tasks.lock().await.contains_key("task")); assert!(lab.area.verify(&lab.stage).is_ok());
        std::fs::remove_dir_all(lab.root).unwrap();
    }
}

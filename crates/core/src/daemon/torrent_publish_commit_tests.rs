#![cfg(target_env = "gnu")]
use super::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence,
    torrent_publish_intent as journal, torrent_stage_store as stages};
use crate::protocols::torrent::{directory_publish, staging_tree::{StageArea, StageRecord}};
use downloader_ipc::DownloadInfo;
use std::{collections::BTreeMap, fs::{self, File}, path::PathBuf, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use tokio::sync::Mutex;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Lab { pub root: PathBuf, pub downloads: PathBuf, pub store: Persistence, pub tasks: Tasks, pub area: StageArea, pub stage: StageRecord }
pub(super) async fn setup() -> Lab {
    let root = std::env::temp_dir().join(format!("rget-publish-commit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&root).unwrap(); let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let store = Persistence::new(root.join("state/queue.sqlite3"));
    let area = StageArea::open(&File::open(&downloads).unwrap()).unwrap(); let name = StageArea::candidate_name();
    stages::reserve(&store.database_path(), "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap(); stages::confirm(&store.database_path(), &stage).unwrap();
    area.create_payload(&stage).unwrap(); stages::retain(&store.database_path(), &stage).unwrap();
    let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"finalizing","filename":null,"downloaded":3,"total":3,"speed_bps":0,"final_path":null,"torrent":{"source":"file.torrent","info_hash":"abc","created_at":1}}"#).unwrap();
    let tasks = Arc::new(Mutex::new(BTreeMap::from([("task".into(), task)])));
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    Lab { root, downloads, store, tasks, area, stage }
}
pub(super) fn rename(lab: &Lab) {
    let stage = lab.area.verify(&lab.stage).unwrap(); let root = File::open(&lab.downloads).unwrap();
    directory_publish::publish_exact(&stage, "payload", &root, "bundle").unwrap();
}

#[tokio::test]
async fn published_intent_and_completed_task_commit_together_with_warning() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap();
    assert!(lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.is_err());
    assert_eq!(lab.tasks.lock().await["task"].state, "finalizing"); assert_eq!(journal::load(&database).unwrap().len(), 1);
    rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, Some("destination sync failed".into())).await.unwrap();
    let task = lab.tasks.lock().await["task"].clone();
    assert_eq!(task.state, "completed"); assert_eq!(task.final_path.as_deref(), lab.downloads.join("bundle").to_str());
    assert_eq!(task.torrent.unwrap().publication_warning.as_deref(), Some("destination sync failed"));
    assert!(journal::load(&database).unwrap().is_empty());
    let published = super::torrent_published_store::load(&database, "task").unwrap().unwrap();
    use std::os::unix::fs::MetadataExt;
    let root_meta = fs::metadata(&lab.downloads).unwrap(); let dest_meta = fs::metadata(lab.downloads.join("bundle")).unwrap();
    assert_eq!(published.name, "bundle"); assert_eq!((published.root_device, published.root_inode), (root_meta.dev(), root_meta.ino()));
    assert_eq!((published.device, published.inode), (dest_meta.dev(), dest_meta.ino()));
    assert_eq!(stages::load(&database).unwrap().len(), 1); // owned empty stage retained until cleanup
    let persisted = lab.store.load().await.unwrap().tasks.remove("task").unwrap();
    assert_eq!(persisted.state, "completed");
    assert_eq!(persisted.torrent.unwrap().publication_warning.as_deref(), Some("destination sync failed"));
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn database_failure_keeps_intent_and_does_not_claim_completion() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    let conn = super::persistence::open(&database).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_completion BEFORE UPDATE ON rget_queue_v1 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap(); drop(conn);
    assert!(lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.is_err());
    assert_eq!(lab.tasks.lock().await["task"].state, "finalizing");
    assert_eq!(journal::load(&database).unwrap(), vec![intent]);
    assert!(super::torrent_published_store::load(&database, "task").unwrap().is_none());
    assert_eq!(stages::load(&database).unwrap().len(), 1); assert!(lab.downloads.join("bundle").is_dir());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn stale_durable_task_is_not_overwritten_by_completion() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    let mut durable = lab.tasks.lock().await["task"].clone(); durable.state = "canceled".into();
    let conn = super::persistence::open(&database).unwrap();
    conn.execute("UPDATE rget_queue_v1 SET info=?1 WHERE url=?2", rusqlite::params![serde_json::to_string(&durable).unwrap(), "task"]).unwrap(); drop(conn);
    assert!(lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.is_err());
    assert_eq!(lab.tasks.lock().await["task"].state, "finalizing");
    assert_eq!(journal::load(&database).unwrap(), vec![intent]);
    assert!(lab.downloads.join("bundle").is_dir());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn existing_published_identity_cannot_be_overwritten() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    super::torrent_published_store::ensure_schema(&database).unwrap();
    let conn = super::persistence::open(&database).unwrap();
    conn.execute("INSERT INTO rget_torrent_published_v1 VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params!["task", "old", "1", "2", "3", "4"]).unwrap(); drop(conn);
    assert!(lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.is_err());
    assert_eq!(super::torrent_published_store::load(&database, "task").unwrap().unwrap().name, "old");
    assert_eq!(journal::load(&database).unwrap(), vec![intent]);
    assert_eq!(lab.tasks.lock().await["task"].state, "finalizing"); assert!(lab.downloads.join("bundle").is_dir());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn published_removal_refuses_replacement_and_preserves_records() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    let task = lab.tasks.lock().await["task"].clone(); let root = File::open(&lab.downloads).unwrap();
    fs::rename(lab.downloads.join("bundle"), lab.downloads.join("original")).unwrap();
    fs::create_dir(lab.downloads.join("bundle")).unwrap();
    fs::write(lab.downloads.join("bundle/unrelated"), b"keep").unwrap();
    assert!(super::torrent_published_remove::delete_files(&database, &lab.downloads, &root, &task).is_err());
    assert_eq!(fs::read(lab.downloads.join("bundle/unrelated")).unwrap(), b"keep");
    fs::remove_dir_all(lab.downloads.join("bundle")).unwrap(); fs::rename(lab.downloads.join("original"), lab.downloads.join("bundle")).unwrap();
    super::torrent_published_remove::delete_files(&database, &lab.downloads, &root, &task).unwrap();
    assert!(!lab.downloads.join("bundle").exists());
    assert!(super::torrent_published_store::load(&database, "task").unwrap().is_some());
    assert_eq!(lab.tasks.lock().await["task"].state, "completed");
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn journaled_removal_deletes_owned_stage_published_files_and_task() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    lab.store.remove_completed_torrent(&lab.tasks, "task", &lab.downloads).await.unwrap();
    assert!(!lab.downloads.join("bundle").exists()); assert!(lab.tasks.lock().await.get("task").is_none());
    assert!(stages::load(&database).unwrap().is_empty());
    assert!(super::torrent_published_store::load(&database, "task").unwrap().is_none());
    assert!(lab.store.load().await.unwrap().tasks.get("task").is_none());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn failed_final_removal_keeps_task_and_durable_intent() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    let conn = super::persistence::open(&database).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_remove BEFORE DELETE ON rget_queue_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    assert!(lab.store.remove_completed_torrent(&lab.tasks, "task", &lab.downloads).await.is_err());
    assert!(!lab.downloads.join("bundle").exists()); assert_eq!(lab.tasks.lock().await["task"].state, "completed");
    assert!(super::torrent_published_store::load(&database, "task").unwrap().is_some());
    let conn = super::persistence::open(&database).unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM rget_torrent_remove_intents_v1 WHERE task_id='task'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1); drop(conn);
    let report = super::torrent_remove_recovery::recover(&database, &lab.downloads).unwrap();
    assert_eq!(report.unresolved, 1); assert_eq!(report.removed, 0);
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn removal_recovery_resumes_only_when_identity_is_still_proven() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    let unsafe_entry = lab.downloads.join(".rget-staging").join(&lab.stage.name).join("unexpected");
    fs::write(&unsafe_entry, b"keep").unwrap();
    assert!(lab.store.remove_completed_torrent(&lab.tasks, "task", &lab.downloads).await.is_err());
    let report = super::torrent_remove_recovery::recover(&database, &lab.downloads).unwrap();
    assert_eq!(report.unresolved, 1); assert!(unsafe_entry.exists()); assert!(lab.downloads.join("bundle").exists());
    fs::remove_file(unsafe_entry).unwrap();
    let report = super::torrent_remove_recovery::recover(&database, &lab.downloads).unwrap();
    assert_eq!(report.removed, 1); assert!(!lab.downloads.join("bundle").exists());
    assert!(lab.store.load().await.unwrap().tasks.get("task").is_none());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn remove_from_list_keeps_published_and_detached_stage() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    lab.store.forget_torrent_keep_files(&lab.tasks, "task").await.unwrap();
    assert!(lab.tasks.lock().await.get("task").is_none());
    assert!(lab.downloads.join("bundle").is_dir()); assert!(lab.area.verify(&lab.stage).is_ok());
    assert_eq!(stages::load(&database).unwrap()[0].state, stages::State::Retained);
    assert!(super::torrent_published_store::load(&database, "task").unwrap().is_some());
    assert!(lab.store.load().await.unwrap().tasks.get("task").is_none());
    let report = super::torrent_stage_recovery::recover(&database, &lab.downloads).unwrap();
    assert_eq!(report.retained, 1); assert!(lab.area.verify(&lab.stage).is_ok());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn keep_files_refuses_pending_removal_intent() {
    let lab = setup().await; let database = lab.store.database_path();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap(); rename(&lab);
    lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
    let unexpected = lab.downloads.join(".rget-staging").join(&lab.stage.name).join("unexpected"); fs::write(&unexpected, b"keep").unwrap();
    assert!(lab.store.remove_completed_torrent(&lab.tasks, "task", &lab.downloads).await.is_err());
    assert!(lab.store.forget_torrent_keep_files(&lab.tasks, "task").await.is_err());
    assert_eq!(lab.tasks.lock().await["task"].state, "completed");
    assert!(lab.downloads.join("bundle").exists()); assert!(unexpected.exists());
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn unfinished_removal_cleans_only_owned_stage_and_rejects_journal() {
    let lab = setup().await; let database = lab.store.database_path();
    lab.tasks.lock().await.get_mut("task").unwrap().state = "failed: verification".into();
    lab.store.save(&lab.tasks, &BandwidthLimiter::new()).await.unwrap();
    let intent = journal::begin(&database, &lab.area, &lab.stage, "bundle").unwrap();
    assert!(lab.store.remove_unfinished_torrent(&lab.tasks, "task", &lab.downloads).await.is_err());
    assert_eq!(journal::load(&database).unwrap(), vec![intent]); assert!(lab.area.verify(&lab.stage).is_ok());
    fs::remove_dir_all(lab.root).unwrap();

    let lab = setup().await; let database = lab.store.database_path();
    lab.tasks.lock().await.get_mut("task").unwrap().state = "canceled".into();
    lab.store.save(&lab.tasks, &BandwidthLimiter::new()).await.unwrap();
    lab.store.remove_unfinished_torrent(&lab.tasks, "task", &lab.downloads).await.unwrap();
    assert!(stages::load(&database).unwrap().is_empty()); assert!(!lab.tasks.lock().await.contains_key("task"));
    fs::remove_dir_all(lab.root).unwrap();
}

#![cfg(target_env = "gnu")]
use super::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence,
    torrent_publish_intent as journal, torrent_stage_store as stages};
use crate::protocols::torrent::{directory_publish, staging_tree::{StageArea, StageRecord}};
use downloader_ipc::DownloadInfo;
use std::{collections::BTreeMap, fs::{self, File}, path::PathBuf, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use tokio::sync::Mutex;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Lab { root: PathBuf, downloads: PathBuf, store: Persistence, tasks: Tasks, area: StageArea, stage: StageRecord }
async fn setup() -> Lab {
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
fn rename(lab: &Lab) {
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

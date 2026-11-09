use super::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence, torrent_publish_intent as journal,
    torrent_publish_proof as proof, torrent_stage_store as stages};
use crate::protocols::torrent::staging_tree::{StageArea, StageRecord};
use downloader_ipc::DownloadInfo;
use std::{collections::BTreeMap, fs::{self, File}, path::PathBuf, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use tokio::sync::Mutex;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Lab { root: PathBuf, downloads: PathBuf, store: Persistence, tasks: Tasks, area: StageArea, stage: StageRecord }
async fn setup() -> Lab {
    let root = std::env::temp_dir().join(format!("rget-proof-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&root).unwrap(); let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let store = Persistence::new(root.join("state/queue.sqlite3"));
    let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"finalizing","filename":null,"downloaded":3,"total":3,"speed_bps":0,"final_path":null,"torrent":{"source":"source.torrent","info_hash":"0123456789abcdef0123456789abcdef01234567","created_at":1,"root_name":"bundle","files":[{"path":"dir/a","length":3,"priority":1}]}}"#).unwrap();
    let tasks = Arc::new(Mutex::new(BTreeMap::from([("task".into(), task)])));
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    let area = StageArea::open(&File::open(&downloads).unwrap()).unwrap(); let name = StageArea::candidate_name();
    stages::reserve(&store.database_path(), "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap(); stages::confirm(&store.database_path(), &stage).unwrap();
    area.create_payload(&stage).unwrap(); stages::retain(&store.database_path(), &stage).unwrap();
    Lab { root, downloads, store, tasks, area, stage }
}
fn payload(lab: &Lab) -> PathBuf { lab.downloads.join(".rget-staging").join(&lab.stage.name).join("payload") }
fn write_payload(lab: &Lab) {
    fs::create_dir(payload(lab).join("dir")).unwrap(); fs::write(payload(lab).join("dir/a"), b"abc").unwrap();
}

#[tokio::test]
async fn only_exact_synced_tree_and_durable_manifest_get_evidence() {
    let lab = setup().await; let path = lab.store.database_path(); let task = lab.tasks.lock().await["task"].clone();
    assert!(proof::begin_verified(&path, &lab.area, &lab.stage, "bundle", &task).is_err());
    assert!(journal::load(&path).unwrap().is_empty());
    write_payload(&lab);
    let mut wrong = task.clone(); wrong.torrent.as_mut().unwrap().files.as_mut().unwrap()[0].length = 4;
    assert!(proof::begin_verified(&path, &lab.area, &lab.stage, "bundle", &wrong).is_err());
    assert!(journal::load(&path).unwrap().is_empty());
    let intent = proof::begin_verified(&path, &lab.area, &lab.stage, "bundle", &task).unwrap();
    assert!(proof::is_verified(&path, &intent, &task).unwrap());
    assert!(!proof::is_verified(&path, &intent, &wrong).unwrap());
    let mut wrong_destination = intent.clone(); wrong_destination.destination = "other".into();
    assert!(!proof::is_verified(&path, &wrong_destination, &task).unwrap());
    #[cfg(target_env = "gnu")]
    {
        let stage_dir = lab.area.verify(&lab.stage).unwrap(); let destination = File::open(&lab.downloads).unwrap();
        crate::protocols::torrent::directory_publish::publish_exact(&stage_dir, "payload", &destination, "bundle").unwrap();
        lab.store.commit_torrent_publication(&lab.tasks, &lab.downloads, &intent, None).await.unwrap();
        let conn = super::persistence::open(&path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM rget_torrent_publish_proofs_v1", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0); assert!(journal::load(&path).unwrap().is_empty());
    }
    fs::remove_dir_all(lab.root).unwrap();
}

#[tokio::test]
async fn failure_to_commit_evidence_leaves_unproven_intent_for_inspection() {
    let lab = setup().await; let path = lab.store.database_path(); let task = lab.tasks.lock().await["task"].clone();
    write_payload(&lab); proof::ensure_schema(&path).unwrap();
    let conn = super::persistence::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_proof BEFORE INSERT ON rget_torrent_publish_proofs_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    assert!(proof::begin_verified(&path, &lab.area, &lab.stage, "bundle", &task).is_err());
    let entries = journal::load(&path).unwrap(); assert_eq!(entries.len(), 1);
    assert!(!proof::is_verified(&path, &entries[0], &task).unwrap());
    assert!(payload(&lab).join("dir/a").exists()); fs::remove_dir_all(lab.root).unwrap();
}

use super::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence, torrent_stage_store as stages};
use crate::protocols::torrent::staging_manifest::{ManifestFile, ValidatedManifest};
use downloader_ipc::DownloadInfo;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use tokio::sync::Mutex;
static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> PathBuf {
    let path = std::env::temp_dir().join(format!("rget-manifest-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&path).unwrap(); path
}
fn manifest() -> ValidatedManifest {
    ValidatedManifest::new("bundle".into(), vec![ManifestFile { components: vec!["dir".into(), "file".into()], length: 3 },
        ManifestFile { components: vec!["empty".into()], length: 0 }], 3).unwrap()
}
async fn setup() -> (PathBuf, Persistence, Tasks) {
    let root = lab(); let store = Persistence::new(root.join("state/queue.sqlite3"));
    let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"metadata","filename":null,"downloaded":0,"total":null,"speed_bps":0,"final_path":null,"torrent":{"source":"file.torrent","info_hash":"abc","created_at":1}}"#).unwrap();
    let tasks = Arc::new(Mutex::new(BTreeMap::from([("task".into(), task)])));
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap(); (root, store, tasks)
}
#[tokio::test]
async fn manifest_is_recorded_and_reloaded_with_normal_priorities() {
    let (root, store, tasks) = setup().await;
    store.save_torrent_manifest(&tasks, "task", "abc", &manifest()).await.unwrap();
    let task = tasks.lock().await["task"].clone(); let torrent = task.torrent.unwrap();
    assert_eq!(torrent.root_name.as_deref(), Some("bundle"));
    let files = torrent.files.unwrap(); assert_eq!(files.len(), 2); assert_eq!(files[0].path, "dir/file"); assert!(files.iter().all(|f| f.priority == 1));
    let restored = store.load().await.unwrap().tasks.remove("task").unwrap().torrent.unwrap();
    assert_eq!(restored.root_name.as_deref(), Some("bundle")); assert_eq!(restored.files.unwrap(), files);
    fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn wrong_identity_or_existing_reservation_never_mutates_task() {
    let (root, store, tasks) = setup().await;
    assert!(store.save_torrent_manifest(&tasks, "task", "wrong", &manifest()).await.is_err());
    stages::reserve(&store.database_path(), "task", "task-reserved", 1).unwrap();
    assert!(store.save_torrent_manifest(&tasks, "task", "abc", &manifest()).await.is_err());
    let task = tasks.lock().await["task"].clone(); assert!(task.torrent.unwrap().root_name.is_none());
    let restored = store.load().await.unwrap().tasks.remove("task").unwrap(); assert!(restored.torrent.unwrap().files.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn durable_identity_or_state_change_cannot_be_overwritten() {
    let (root, store, tasks) = setup().await;
    let original = tasks.lock().await["task"].clone();
    for change in 0..6 {
        let mut durable = original.clone();
        match change {
            0 => durable.state = "canceled".into(),
            1 => durable.torrent.as_mut().unwrap().source = "other.torrent".into(),
            2 => durable.torrent.as_mut().unwrap().info_hash = Some("other".into()),
            3 => durable.torrent.as_mut().unwrap().root_name = Some("other".into()),
            4 => durable.torrent.as_mut().unwrap().created_at += 1,
            _ => durable.torrent.as_mut().unwrap().stage_name = Some("task-existing".into()),
        }
        let conn = super::persistence::open(&store.database_path()).unwrap();
        conn.execute("UPDATE rget_queue_v1 SET info=?1 WHERE url=?2", rusqlite::params![serde_json::to_string(&durable).unwrap(), "task"]).unwrap();
        drop(conn);
        assert!(store.save_torrent_manifest(&tasks, "task", "abc", &manifest()).await.is_err());
        assert!(tasks.lock().await["task"].torrent.as_ref().unwrap().files.is_none());
        let conn = super::persistence::open(&store.database_path()).unwrap();
        let json: String = conn.query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", ["task"], |row| row.get(0)).unwrap();
        let after: DownloadInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(after.state, durable.state);
        assert_eq!(after.torrent.unwrap(), durable.torrent.unwrap());
    }
    fs::remove_dir_all(root).unwrap();
}

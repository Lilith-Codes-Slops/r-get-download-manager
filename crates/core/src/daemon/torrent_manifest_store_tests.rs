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

#[tokio::test]
async fn manifest_and_reservation_commit_together_before_stage_creation() {
    use std::time::Duration;
    let (root, store, tasks) = setup().await;
    let reservation = store.save_manifest_and_reserve(&tasks, "task", "abc", &manifest()).await.unwrap();
    let saved = store.load().await.unwrap().tasks.remove("task").unwrap();
    assert_eq!(saved.torrent.unwrap().root_name.as_deref(), Some("bundle"));
    let entries = stages::load(&store.database_path()).unwrap();
    assert_eq!(entries.len(), 1); assert_eq!(entries[0].name, reservation.name);
    assert_eq!(entries[0].state, stages::State::Reserved);
    assert!(!root.join(".rget-staging").exists());
    assert!(tokio::time::timeout(Duration::from_millis(25), super::torrent_lifecycle::global().lock("task")).await.is_err());
    drop(reservation);
    let _guard = tokio::time::timeout(Duration::from_secs(1), super::torrent_lifecycle::global().lock("task")).await.unwrap().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn reservation_failure_rolls_back_manifest_and_releases_guard() {
    use std::time::Duration;
    let (root, store, tasks) = setup().await;
    stages::load(&store.database_path()).unwrap();
    let conn = super::persistence::open(&store.database_path()).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_reservation BEFORE INSERT ON rget_torrent_stages_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    assert!(store.save_manifest_and_reserve(&tasks, "task", "abc", &manifest()).await.is_err());
    assert!(stages::load(&store.database_path()).unwrap().is_empty());
    assert!(tasks.lock().await["task"].torrent.as_ref().unwrap().files.is_none());
    assert!(store.load().await.unwrap().tasks["task"].torrent.as_ref().unwrap().files.is_none());
    let _guard = tokio::time::timeout(Duration::from_secs(1), super::torrent_lifecycle::global().lock("task")).await.unwrap().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn guarded_allocation_confirms_stage_before_payload() {
    use std::{fs::File, time::Duration};
    let (root, store, tasks) = setup().await;
    let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let reservation = store.save_manifest_and_reserve(&tasks, "task", "abc", &manifest()).await.unwrap();
    let name = reservation.name.clone();
    let allocation = reservation.allocate(File::open(&downloads).unwrap()).await.unwrap();
    let entries = stages::load(&store.database_path()).unwrap();
    assert_eq!(entries[0].state, stages::State::Staging);
    assert_eq!(entries[0].identity.as_ref(), Some(&allocation.record));
    assert!(allocation.payload.metadata().unwrap().is_dir());
    assert!(downloads.join(".rget-staging").join(&name).join("payload").is_dir());
    assert!(tokio::time::timeout(Duration::from_millis(25), super::torrent_lifecycle::global().lock("task")).await.is_err());
    stages::cleanup_recorded(&store.database_path(), &allocation.area, "task", &name, false).unwrap();
    drop(allocation); assert!(stages::load(&store.database_path()).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn allocation_never_adopts_existing_stage_and_retains_reservation() {
    use std::fs::File;
    let (root, store, tasks) = setup().await;
    let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let parent = downloads.join(".rget-staging"); fs::create_dir(&parent).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let reservation = store.save_manifest_and_reserve(&tasks, "task", "abc", &manifest()).await.unwrap();
    let existing = parent.join(&reservation.name); fs::create_dir(&existing).unwrap(); fs::write(existing.join("unrelated"), b"keep").unwrap();
    assert!(reservation.allocate(File::open(&downloads).unwrap()).await.is_err());
    assert_eq!(fs::read(existing.join("unrelated")).unwrap(), b"keep");
    assert_eq!(stages::load(&store.database_path()).unwrap()[0].state, stages::State::Reserved);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn failed_confirmation_cleans_proven_stage_but_keeps_reservation() {
    use std::fs::File;
    let (root, store, tasks) = setup().await;
    let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let reservation = store.save_manifest_and_reserve(&tasks, "task", "abc", &manifest()).await.unwrap();
    let stage = downloads.join(".rget-staging").join(&reservation.name);
    let conn = super::persistence::open(&store.database_path()).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_confirmation BEFORE UPDATE ON rget_torrent_stages_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    assert!(reservation.allocate(File::open(&downloads).unwrap()).await.is_err());
    assert!(!stage.exists()); assert_eq!(stages::load(&store.database_path()).unwrap()[0].state, stages::State::Reserved);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_env = "gnu")]
async fn publication_lab() -> (PathBuf, Persistence, Tasks, super::torrent_stage_allocate::AllocatedStage) {
    use std::fs::File;
    let (root, store, tasks) = setup().await;
    let hash = "0123456789abcdef0123456789abcdef01234567";
    tasks.lock().await.get_mut("task").unwrap().torrent.as_mut().unwrap().info_hash = Some(hash.into());
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let reservation = store.save_manifest_and_reserve(&tasks, "task", hash, &manifest()).await.unwrap();
    let stage = reservation.allocate(File::open(&downloads).unwrap()).await.unwrap();
    tasks.lock().await.get_mut("task").unwrap().state = "finalizing".into();
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    (root, store, tasks, stage)
}

#[cfg(target_env = "gnu")]
fn fill_stage(root: &std::path::Path, stage: &super::torrent_stage_allocate::AllocatedStage) {
    let payload = root.join("Downloads/.rget-staging").join(&stage.record.name).join("payload");
    fs::create_dir(payload.join("dir")).unwrap(); fs::write(payload.join("dir/file"), b"abc").unwrap();
    fs::write(payload.join("empty"), b"").unwrap();
}

#[cfg(target_env = "gnu")]
#[tokio::test]
async fn guarded_publish_retains_verifies_and_journals_before_rename() {
    let (root, store, tasks, stage) = publication_lab().await; fill_stage(&root, &stage);
    let task = tasks.lock().await["task"].clone();
    let published = stage.publish_exact(task, "bundle".into()).await.unwrap();
    assert!(root.join("Downloads/bundle/dir/file").is_file());
    assert_eq!(stages::load(&store.database_path()).unwrap()[0].state, stages::State::Retained);
    assert_eq!(super::torrent_publish_intent::load(&store.database_path()).unwrap(), vec![published.intent.clone()]);
    assert!(super::torrent_publish_proof::is_verified(&store.database_path(), &published.intent, &tasks.lock().await["task"]).unwrap());
    assert_eq!(tasks.lock().await["task"].state, "finalizing");
    drop(published); fs::remove_dir_all(root).unwrap();
}

#[cfg(target_env = "gnu")]
#[tokio::test]
async fn verification_failure_and_collision_keep_owned_stage() {
    let (root, store, tasks, stage) = publication_lab().await;
    let name = stage.record.name.clone(); let task = tasks.lock().await["task"].clone();
    assert!(stage.publish_exact(task, "bundle".into()).await.is_err());
    assert_eq!(stages::load(&store.database_path()).unwrap()[0].state, stages::State::Retained);
    assert!(super::torrent_publish_intent::load(&store.database_path()).unwrap().is_empty());
    assert!(root.join("Downloads/.rget-staging").join(name).join("payload").is_dir());
    fs::remove_dir_all(root).unwrap();

    let (root, store, tasks, stage) = publication_lab().await; fill_stage(&root, &stage);
    let name = stage.record.name.clone(); fs::create_dir(root.join("Downloads/bundle")).unwrap();
    let task = tasks.lock().await["task"].clone();
    assert!(stage.publish_exact(task, "bundle".into()).await.is_err());
    assert!(root.join("Downloads/bundle").is_dir());
    assert!(root.join("Downloads/.rget-staging").join(name).join("payload/dir/file").is_file());
    assert_eq!(stages::load(&store.database_path()).unwrap()[0].state, stages::State::Retained);
    assert_eq!(super::torrent_publish_intent::load(&store.database_path()).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

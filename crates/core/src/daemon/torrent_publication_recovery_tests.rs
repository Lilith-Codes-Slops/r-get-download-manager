use super::{bandwidth::BandwidthLimiter, control::Tasks, persistence::Persistence,
    torrent_manifest_store_tests::{manifest, setup}, torrent_publication_recovery as recovery,
    torrent_recovery_info_tests::recovery_metainfo, torrent_stage_store as stages};
use librqbit_sha1_wrapper::{ISha1, Sha1};
use std::{fs::{self, File}, path::PathBuf};

async fn published() -> (PathBuf, Persistence, Tasks) {
    let (root, store, tasks) = setup().await; let mut bytes = recovery_metainfo();
    let mut sha = Sha1::new(); sha.update(b"abc"); let piece = sha.finish();
    let start = bytes.len() - 22; bytes[start..start + 20].copy_from_slice(&piece);
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    tasks.lock().await.get_mut("task").unwrap().torrent.as_mut().unwrap().info_hash = Some(hash.clone());
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    let downloads = root.join("Downloads"); fs::create_dir(&downloads).unwrap();
    let reservation = store.save_manifest_info_and_reserve(&tasks, "task", &hash, &manifest(), &bytes).await.unwrap();
    let stage = reservation.allocate(File::open(&downloads).unwrap()).await.unwrap();
    let payload = downloads.join(".rget-staging").join(&stage.record.name).join("payload");
    fs::create_dir(payload.join("dir")).unwrap(); fs::write(payload.join("dir/file"), b"abc").unwrap(); fs::write(payload.join("empty"), b"").unwrap();
    tasks.lock().await.get_mut("task").unwrap().state = "finalizing".into();
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    let task = tasks.lock().await["task"].clone();
    drop(stage.publish_exact(task, "bundle".into()).await.unwrap());
    (root, store, tasks)
}

#[tokio::test]
async fn crash_after_rename_recovers_only_after_piece_verification() {
    let (root, store, _) = published().await; let path = store.database_path(); let downloads = root.join("Downloads");
    let report = recovery::recover(&path, &downloads).unwrap(); assert_eq!(report.completed, 1); assert_eq!(report.unresolved, 0);
    let task = store.load().await.unwrap().tasks.remove("task").unwrap();
    assert_eq!(task.state, "completed"); assert_eq!(task.final_path.as_deref(), downloads.join("bundle").to_str());
    assert!(downloads.join("bundle/dir/file").is_file()); assert!(stages::load(&path).unwrap().is_empty());
    assert!(super::torrent_publish_intent::load(&path).unwrap().is_empty());
    assert_eq!(recovery::recover(&path, &downloads).unwrap().completed, 0); fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn tampering_or_missing_piece_metadata_never_completes_or_deletes() {
    for missing in [false, true] {
        let (root, store, _) = published().await; let path = store.database_path(); let downloads = root.join("Downloads");
        if missing {
            let conn = super::persistence::open(&path).unwrap();
            conn.execute("DELETE FROM rget_torrent_recovery_info_v1 WHERE task_id=?1", ["task"]).unwrap();
        } else { fs::write(downloads.join("bundle/dir/file"), b"abd").unwrap(); }
        let report = recovery::recover(&path, &downloads).unwrap(); assert_eq!(report.completed, 0); assert_eq!(report.unresolved, 1);
        assert_eq!(super::torrent_publish_intent::load(&path).unwrap().len(), 1);
        assert_eq!(stages::load(&path).unwrap()[0].state, stages::State::Retained);
        assert!(downloads.join("bundle/dir/file").is_file()); fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn source_present_or_failed_completion_never_causes_replay_or_cleanup() {
    let (root, store, _) = published().await; let path = store.database_path(); let downloads = root.join("Downloads");
    let stage = stages::load(&path).unwrap().remove(0); let payload = downloads.join(".rget-staging").join(stage.name).join("payload");
    fs::rename(downloads.join("bundle"), &payload).unwrap();
    let report = recovery::recover(&path, &downloads).unwrap(); assert_eq!(report.unresolved, 1); assert_eq!(report.completed, 0);
    assert!(payload.join("dir/file").is_file()); assert!(!downloads.join("bundle").exists());
    assert_eq!(super::torrent_publish_intent::load(&path).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();

    let (root, store, _) = published().await; let path = store.database_path(); let downloads = root.join("Downloads");
    let conn = super::persistence::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_recovery BEFORE UPDATE ON rget_queue_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    let report = recovery::recover(&path, &downloads).unwrap(); assert_eq!(report.unresolved, 1); assert_eq!(report.completed, 0);
    assert!(downloads.join("bundle/dir/file").is_file()); assert_eq!(stages::load(&path).unwrap()[0].state, stages::State::Retained);
    assert_eq!(super::torrent_publish_intent::load(&path).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn crash_after_completion_cleans_only_empty_proven_stage() {
    let (root, store, tasks) = published().await; let path = store.database_path(); let downloads = root.join("Downloads");
    let intent = super::torrent_publish_intent::load(&path).unwrap().remove(0);
    store.commit_torrent_publication(&tasks, &downloads, &intent, None).await.unwrap();
    assert_eq!(stages::load(&path).unwrap().len(), 1);
    let report = recovery::recover(&path, &downloads).unwrap(); assert_eq!(report.cleaned, 1);
    assert!(stages::load(&path).unwrap().is_empty()); assert!(downloads.join("bundle/dir/file").is_file());
    fs::remove_dir_all(root).unwrap();
}

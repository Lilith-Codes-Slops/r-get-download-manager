use super::{bandwidth::BandwidthLimiter, torrent_manifest_store_tests::{manifest, setup}, torrent_stage_store as stages};
use std::fs;

pub(super) fn recovery_metainfo() -> Vec<u8> {
    let mut bytes = b"d8:announce27:https://example.test/secret4:infod5:filesl".to_vec();
    bytes.extend_from_slice(b"d6:lengthi3e4:pathl3:dir4:fileee");
    bytes.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyee");
    bytes.extend_from_slice(b"e4:name6:bundle12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&[0; 20]); bytes.extend_from_slice(b"ee"); bytes
}

#[tokio::test]
async fn recovery_info_manifest_and_reservation_are_committed_together() {
    use crate::protocols::torrent::recovery_metadata;
    let (root, store, tasks) = setup().await; let bytes = recovery_metainfo();
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    tasks.lock().await.get_mut("task").unwrap().torrent.as_mut().unwrap().info_hash = Some(hash.clone());
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    assert!(store.save_manifest_info_and_reserve(&tasks, "task", "0000000000000000000000000000000000000000", &manifest(), &bytes).await.is_err());
    let other = crate::protocols::torrent::staging_manifest::ValidatedManifest::new("other".into(), manifest().files().to_vec(), 3).unwrap();
    assert!(store.save_manifest_info_and_reserve(&tasks, "task", &hash, &other, &bytes).await.is_err());
    assert!(stages::load(&store.database_path()).unwrap().is_empty());
    let reservation = store.save_manifest_info_and_reserve(&tasks, "task", &hash, &manifest(), &bytes).await.unwrap();
    let conn = super::persistence::open(&store.database_path()).unwrap();
    let info: Vec<u8> = conn.query_row("SELECT info FROM rget_torrent_recovery_info_v1 WHERE task_id=?1", ["task"], |r| r.get(0)).unwrap();
    assert!(!info.windows(b"example.test".len()).any(|window| window == b"example.test"));
    recovery_metadata::validate_info(&info, &hash, &manifest()).unwrap();
    let mut changed = info.clone(); let last = changed.len()-1; changed[last] = b'x';
    assert!(recovery_metadata::validate_info(&changed, &hash, &manifest()).is_err());
    assert_eq!(stages::load(&store.database_path()).unwrap()[0].name, reservation.name);
    assert_eq!(store.load().await.unwrap().tasks["task"].torrent.as_ref().unwrap().root_name.as_deref(), Some("bundle"));
    drop(conn); drop(reservation); fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn recovery_info_failure_cannot_commit_manifest_or_reservation() {
    let (root, store, tasks) = setup().await; let bytes = recovery_metainfo();
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    tasks.lock().await.get_mut("task").unwrap().torrent.as_mut().unwrap().info_hash = Some(hash.clone());
    store.save(&tasks, &BandwidthLimiter::new()).await.unwrap();
    let conn = super::persistence::open(&store.database_path()).unwrap();
    conn.execute_batch("CREATE TABLE rget_torrent_recovery_info_v1 (task_id TEXT PRIMARY KEY NOT NULL, info BLOB NOT NULL); CREATE TRIGGER fail_recovery_info BEFORE INSERT ON rget_torrent_recovery_info_v1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap(); drop(conn);
    assert!(store.save_manifest_info_and_reserve(&tasks, "task", &hash, &manifest(), &bytes).await.is_err());
    assert!(stages::load(&store.database_path()).unwrap().is_empty());
    assert!(tasks.lock().await["task"].torrent.as_ref().unwrap().files.is_none());
    assert!(store.load().await.unwrap().tasks["task"].torrent.as_ref().unwrap().files.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_env = "gnu")]
#[tokio::test]
async fn journaled_destination_piece_inspection_detects_tampering_and_missing_info() {
    use librqbit_sha1_wrapper::{ISha1, Sha1};
    use super::torrent_recovery_content::{self, ContentEvidence};
    use std::fs::File;
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
    let published = stage.publish_exact(task.clone(), "bundle".into()).await.unwrap();
    let inspect = || torrent_recovery_content::inspect(&store.database_path(), &published.stage.area, &published.stage.root, &published.intent, &task);
    assert_eq!(inspect().unwrap(), ContentEvidence::DestinationPiecesMatch);
    fs::write(downloads.join("bundle/dir/file"), b"abd").unwrap(); assert!(inspect().is_err());
    fs::write(downloads.join("bundle/dir/file"), b"abc").unwrap();
    let conn = super::persistence::open(&store.database_path()).unwrap();
    conn.execute("UPDATE rget_torrent_recovery_info_v1 SET info=?1 WHERE task_id=?2", rusqlite::params![b"invalid".as_slice(), "task"]).unwrap();
    assert!(inspect().is_err());
    conn.execute("DELETE FROM rget_torrent_recovery_info_v1 WHERE task_id=?1", ["task"]).unwrap(); drop(conn);
    assert_eq!(inspect().unwrap(), ContentEvidence::MissingInfo);
    assert_eq!(tasks.lock().await["task"].state, "finalizing");
    assert_eq!(super::torrent_publish_intent::load(&store.database_path()).unwrap().len(), 1);
    assert!(downloads.join("bundle/dir/file").is_file());
    drop(published); fs::remove_dir_all(root).unwrap();
}

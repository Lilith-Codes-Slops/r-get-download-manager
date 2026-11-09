#![cfg(target_env = "gnu")]
use super::{torrent_publish_inspection::{self as inspection, Inspection}, torrent_publish_intent as journal, torrent_stage_store as store};
use crate::protocols::torrent::{directory_publish, staging_tree::StageArea};
use downloader_ipc::DownloadInfo;
use std::{fs::{self, File}, os::unix::fs::symlink, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> PathBuf {
    let path = std::env::temp_dir().join(format!("rget-inspect-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&path).unwrap(); path
}
fn task() -> DownloadInfo {
    serde_json::from_str(r#"{"url":"task","state":"finalizing","filename":null,"downloaded":3,"total":3,"speed_bps":0,"final_path":null,"torrent":{"source":"source.torrent","info_hash":"0123456789abcdef0123456789abcdef01234567","created_at":1,"root_name":"bundle","files":[{"path":"dir/a","length":3,"priority":1},{"path":"empty","length":0,"priority":1}]}}"#).unwrap()
}
#[test]
fn inspects_but_never_adopts_or_changes_published_data() {
    let root = lab(); let db = root.join("state/queue.sqlite3"); let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap(); let parent = File::open(&downloads).unwrap(); let area = StageArea::open(&parent).unwrap();
    let name = StageArea::candidate_name(); store::reserve(&db, "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap(); store::confirm(&db, &stage).unwrap(); area.create_payload(&stage).unwrap();
    let payload = downloads.join(".rget-staging").join(&name).join("payload"); fs::create_dir(payload.join("dir")).unwrap();
    fs::write(payload.join("dir/a"), b"abc").unwrap(); fs::write(payload.join("empty"), b"").unwrap(); store::retain(&db, &stage).unwrap();
    let intent = journal::begin(&db, &area, &stage, "bundle").unwrap(); let task = task();
    assert_eq!(inspection::inspect(&area, &parent, &intent, &task).unwrap(), Inspection::SourcePresent);
    directory_publish::publish_exact(&area.verify(&stage).unwrap(), "payload", &parent, "bundle").unwrap();
    assert_eq!(inspection::inspect(&area, &parent, &intent, &task).unwrap(), Inspection::DestinationLayoutMatches);
    assert_eq!(journal::load(&db).unwrap(), vec![intent.clone()]);
    fs::write(downloads.join("bundle/dir/a"), b"wrong length").unwrap();
    assert!(inspection::inspect(&area, &parent, &intent, &task).is_err());
    assert!(downloads.join("bundle/dir/a").exists()); assert_eq!(journal::load(&db).unwrap().len(), 1);
    fs::remove_file(downloads.join("bundle/dir/a")).unwrap(); symlink("../empty", downloads.join("bundle/dir/a")).unwrap();
    assert!(inspection::inspect(&area, &parent, &intent, &task).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn refuses_missing_or_unsafe_stored_manifest() {
    let root = lab(); let db = root.join("state/queue.sqlite3"); let parent = File::open(&root).unwrap(); let area = StageArea::open(&parent).unwrap();
    let name = StageArea::candidate_name(); store::reserve(&db, "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap(); store::confirm(&db, &stage).unwrap(); area.create_payload(&stage).unwrap(); store::retain(&db, &stage).unwrap();
    let intent = journal::begin(&db, &area, &stage, "bundle").unwrap(); let mut task = task();
    task.torrent.as_mut().unwrap().files = None; assert!(inspection::inspect(&area, &parent, &intent, &task).is_err());
    let mut malformed = task.clone();
    malformed.torrent.as_mut().unwrap().files = Some(vec![downloader_ipc::TorrentFile { path: "../escape".into(), length: 3, priority: 1 }]);
    assert!(inspection::inspect(&area, &parent, &intent, &malformed).is_err());
    fs::remove_dir_all(root).unwrap();
}

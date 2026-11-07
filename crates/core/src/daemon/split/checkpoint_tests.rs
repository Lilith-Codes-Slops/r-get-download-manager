use super::{checkpoint::{SegmentRecord, SplitCheckpoint, SplitRecord}, recovery, runner};
use crate::daemon::{bandwidth::BandwidthLimiter, checkpoint::write_atomic, control::Tasks, persistence::Persistence};
use downloader_ipc::DownloadInfo;
use std::{collections::BTreeMap, os::unix::fs::FileExt, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

struct Fixture { dir: PathBuf, partial: PathBuf, sidecar: PathBuf, record: SplitRecord }
impl Fixture {
    fn new(url: &str, total: u64) -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("rget-split-recovery-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let partial = dir.join(".rget-1-1.part");
        let sidecar = dir.join(".rget-1-1.part.meta");
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&partial).unwrap();
        file.write_all_at(&[0, 1, 2], 0).unwrap();
        file.write_all_at(&[99, 98], total / 2).unwrap();
        let record = SplitRecord { version: 2, sequence: 2, task_url: url.into(), resolved_url: url.into(),
            filename: "file.bin".into(), partial_name: ".rget-1-1.part".into(), etag: "\"v1\"".into(),
            total, requested_connections: 2, segments: vec![
                SegmentRecord { start: 0, end: total / 2, downloaded: 3, hash: blake3::hash(&[0, 1, 2]).to_hex().to_string() },
                SegmentRecord { start: total / 2, end: total, downloaded: 0, hash: blake3::hash(&[]).to_hex().to_string() },
            ] };
        write_atomic(&sidecar, &record).unwrap();
        Self { dir, partial, sidecar, record }
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); } }

#[test]
fn recovers_newer_sidecar_and_leaves_uncheckpointed_bytes_untouched() {
    let f = Fixture::new("https://example.test/file", 8);
    let mut old = f.record.clone();
    old.sequence = 1;
    old.segments[0].downloaded = 1;
    old.segments[0].hash = blake3::hash(&[0]).to_hex().to_string();
    let before = std::fs::read(&f.partial).unwrap();
    let prepared = recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, Some(&old)).unwrap();
    assert_eq!(prepared.record.sequence, 2);
    assert_eq!(std::fs::read(&f.partial).unwrap(), before);
}

#[test]
fn reconciles_newer_database_and_validates_both_snapshots() {
    let f = Fixture::new("https://example.test/file", 8);
    let mut older = f.record.clone();
    older.sequence = 1;
    older.segments[0].downloaded = 1;
    older.segments[0].hash = blake3::hash(&[0]).to_hex().to_string();
    write_atomic(&f.sidecar, &older).unwrap();
    let prepared = recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, Some(&f.record)).unwrap();
    assert_eq!(prepared.record.sequence, 2);
    // Even when the newer prefix matches, a conflicting older prefix must not be ignored.
    older.segments[0].hash = "0".repeat(64);
    write_atomic(&f.sidecar, &older).unwrap();
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, Some(&f.record)).is_err());
}

#[test]
fn rejects_tampering_bad_maps_and_inconsistent_sequences() {
    let f = Fixture::new("https://example.test/file", 8);
    let mut bad = f.record.clone();
    bad.segments[1].start = 3;
    write_atomic(&f.sidecar, &bad).unwrap();
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, None).is_err());
    write_atomic(&f.sidecar, &f.record).unwrap();
    let mut conflicting = f.record.clone();
    conflicting.segments[0].hash = "0".repeat(64);
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, Some(&conflicting)).is_err());
    std::fs::OpenOptions::new().write(true).open(&f.partial).unwrap().write_all_at(&[9], 0).unwrap();
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, None).is_err());
}

#[test]
fn rejects_symlinked_sidecar_and_partial() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("https://example.test/file", 8);
    let other = f.dir.join("other");
    std::fs::rename(&f.sidecar, &other).unwrap();
    symlink(&other, &f.sidecar).unwrap();
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, None).is_err());
    std::fs::remove_file(&f.sidecar).unwrap();
    std::fs::rename(&other, &f.sidecar).unwrap();
    std::fs::rename(&f.partial, &other).unwrap();
    symlink(&other, &f.partial).unwrap();
    assert!(recovery::prepare(&f.sidecar, &f.partial, &f.record.task_url, None).is_err());
}

#[tokio::test]
async fn recovered_ranges_resume_and_commit_to_sqlite() {
    let (url, server) = super::tests::server(0).await;
    let f = Fixture::new(&url, super::tests::TOTAL);
    let prepared = recovery::prepare(&f.sidecar, &f.partial, &url, None).unwrap();
    let task = DownloadInfo { url: url.clone(), state: "downloading".into(), filename: Some("file.bin".into()),
        downloaded: 3, total: Some(f.record.total), speed_bps: 0, final_path: None,
        resume_supported: true, partial_name: Some(f.record.partial_name.clone()) };
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::from([(url.clone(), task)])));
    let store = Arc::new(Persistence::new(f.dir.join("state/queue.sqlite3")));
    let checkpoint = SplitCheckpoint { sidecar: f.sidecar.clone(), record: prepared.record.clone() };
    tokio::time::timeout(std::time::Duration::from_secs(15), runner::run_checkpointed(
        prepared, checkpoint, reqwest::Client::new(), Arc::new(BandwidthLimiter::new()), store.clone(), tasks,
    )).await.unwrap().unwrap();
    let bytes = std::fs::read(&f.partial).unwrap();
    assert_eq!(bytes.len() as u64, f.record.total);
    assert!(bytes.iter().enumerate().all(|(index, byte)| *byte == (index as u64 % 251) as u8));
    let saved = store.load_split_recovery(&url).await.unwrap().unwrap();
    assert!(saved.sequence >= 4);
    assert!(saved.segments.iter().all(|s| s.downloaded == s.end - s.start));
    let db = rusqlite::Connection::open(f.dir.join("state/queue.sqlite3")).unwrap();
    let json: String = db.query_row("SELECT info FROM rget_queue_v1 WHERE url = ?1", [&url], |row| row.get(0)).unwrap();
    let saved_task: DownloadInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(saved_task.downloaded, f.record.total);
    server.abort();
}

#[tokio::test]
async fn checkpoint_waits_for_the_write_barrier() {
    let url = "https://example.test/file";
    let f = Fixture::new(url, 8);
    let prepared = recovery::prepare(&f.sidecar, &f.partial, url, None).unwrap();
    let task = DownloadInfo { url: url.into(), state: "downloading".into(), filename: Some("file.bin".into()),
        downloaded: 3, total: Some(8), speed_bps: 0, final_path: None,
        resume_supported: true, partial_name: Some(f.record.partial_name.clone()) };
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::from([(url.into(), task)])));
    let store = Arc::new(Persistence::new(f.dir.join("state/queue.sqlite3")));
    let map = prepared.map.clone();
    let guard = map.lock().await;
    let mut checkpoint = SplitCheckpoint { sidecar: f.sidecar.clone(), record: prepared.record };
    let map_for_save = map.clone();
    let file = prepared.file;
    let store_for_save = store.clone();
    let mut save = tokio::spawn(async move { checkpoint.save(&map_for_save, &file, &store_for_save, &tasks).await });
    assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut save).await.is_err());
    drop(guard);
    save.await.unwrap().unwrap();
    let saved = store.load_split_recovery(url).await.unwrap().unwrap();
    assert_eq!(saved.sequence, 3);
}

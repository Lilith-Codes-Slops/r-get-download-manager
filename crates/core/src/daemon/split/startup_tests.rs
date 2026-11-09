use super::{checkpoint::{SegmentRecord, SplitRecord}, startup};
use crate::daemon::persistence::SavedState;
use downloader_ipc::DownloadInfo;

#[test]
fn published_split_requires_complete_per_range_hashes() {
    for valid in [true, false] {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("rget-split-published-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("file.bin"), if valid { b"abcdefgh" } else { b"abcdXXXX" }).unwrap();
        let id = "https://example.test/file";
        let record = SplitRecord { version: 2, sequence: 5, task_url: id.into(), resolved_url: id.into(),
            filename: "file.bin".into(), partial_name: ".rget-1-1.part".into(), etag: "\"v1\"".into(),
            total: 8, requested_connections: 2, segments: vec![
                SegmentRecord { start: 0, end: 4, downloaded: 4, hash: blake3::hash(b"abcd").to_hex().to_string() },
                SegmentRecord { start: 4, end: 8, downloaded: 4, hash: blake3::hash(b"efgh").to_hex().to_string() },
            ] };
        let mut state = SavedState::default();
        state.prior_states.insert(id.into(), "finalizing".into());
        state.split_checkpoints.insert(id.into(), record);
        state.tasks.insert(id.into(), DownloadInfo { url: id.into(), state: "failed: interrupted".into(),
            filename: None, downloaded: 0, total: None, speed_bps: 0, final_path: None, destination_path: None, suggested_filename: None, torrent: None,
            resume_supported: false, partial_name: Some(".rget-1-1.part".into()), requested_connections: None, active_connections: None, segment_count: None, connection_note: None, live_connections_supported: false });
        assert!(startup::reconcile(&mut state, &dir).is_empty());
        if valid { assert_eq!(state.tasks[id].state, "completed"); }
        else { assert!(state.tasks[id].state.starts_with("failed:")); }
        assert!(dir.join("file.bin").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

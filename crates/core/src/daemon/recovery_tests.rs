use super::checkpoint::{write_atomic, RecoveryRecord};
use super::persistence::SavedState;
use super::recovery::reconcile;
use downloader_ipc::DownloadInfo;
use std::path::PathBuf;

struct Fixture {
    dir: PathBuf,
    partial: PathBuf,
    sidecar: PathBuf,
    state: SavedState,
}
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rget-reconcile-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let partial = dir.join(".rget-1-1.part");
        let sidecar = dir.join(".rget-1-1.part.meta");
        std::fs::write(&partial, b"abcdtail").unwrap();
        let url = "https://example.test/file";
        let record = RecoveryRecord {
            version: 1,
            sequence: 2,
            task_url: url.into(),
            resolved_url: url.into(),
            filename: "file.bin".into(),
            partial_name: ".rget-1-1.part".into(),
            etag: "\"v1\"".into(),
            total: 12,
            downloaded: 4,
            prefix_blake3: blake3::hash(b"abcd").to_hex().to_string(),
        };
        write_atomic(&sidecar, &record).unwrap();
        let mut state = SavedState::default();
        state.prior_states.insert(url.into(), "paused".into());
        state.checkpoints.insert(url.into(), record);
        state.tasks.insert(
            url.into(),
            DownloadInfo {
                url: url.into(),
                state: "failed: interrupted".into(),
                filename: None,
                downloaded: 0,
                total: None,
                speed_bps: 0,
                final_path: None,
                destination_path: None,
                suggested_filename: None,
                torrent: None,
                resume_supported: false,
                partial_name: Some(".rget-1-1.part".into()),
                requested_connections: None,
                active_connections: None,
                segment_count: None,
                connection_note: None,
                live_connections_supported: false,
            },
        );
        Self {
            dir,
            partial,
            sidecar,
            state,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn restores_paused_state_and_truncates_only_uncheckpointed_tail() {
    let mut f = Fixture::new();
    let result = reconcile(&mut f.state, &f.dir);
    assert_eq!(result.len(), 1);
    assert!(result[0].paused);
    assert_eq!(std::fs::read(&f.partial).unwrap(), b"abcd");
    assert_eq!(f.state.tasks["https://example.test/file"].state, "paused");
}

#[test]
fn missing_or_corrupt_sidecar_never_truncates_the_partial() {
    for corrupt in [false, true] {
        let mut f = Fixture::new();
        if corrupt {
            std::fs::write(&f.sidecar, b"not json").unwrap();
        } else {
            std::fs::remove_file(&f.sidecar).unwrap();
        }
        assert!(reconcile(&mut f.state, &f.dir).is_empty());
        assert!(f.state.tasks["https://example.test/file"]
            .state
            .starts_with("failed: recovery unavailable"));
        assert_eq!(std::fs::read(&f.partial).unwrap(), b"abcdtail");
    }
}

#[test]
fn existing_final_file_is_not_assumed_complete_from_size_alone() {
    let mut f = Fixture::new();
    std::fs::write(f.dir.join("file.bin"), b"abcdefghijkl").unwrap();
    let prepared = reconcile(&mut f.state, &f.dir);
    assert_eq!(prepared.len(), 1);
    assert_eq!(f.state.tasks["https://example.test/file"].state, "paused");
    assert_eq!(std::fs::read(&f.partial).unwrap(), b"abcd");
    assert_eq!(
        std::fs::read(f.dir.join("file.bin")).unwrap(),
        b"abcdefghijkl"
    );
}

#[test]
fn reconciles_sidecar_newer_than_database() {
    let mut f = Fixture::new();
    let old = f
        .state
        .checkpoints
        .get_mut("https://example.test/file")
        .unwrap();
    old.sequence = 1;
    old.downloaded = 2;
    old.prefix_blake3 = blake3::hash(b"ab").to_hex().to_string();
    let prepared = reconcile(&mut f.state, &f.dir);
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].record.sequence, 2);
    assert_eq!(std::fs::read(&f.partial).unwrap(), b"abcd");
}

#[test]
fn reconciles_database_newer_than_sidecar_after_verifying_prefix() {
    let mut f = Fixture::new();
    let new = f
        .state
        .checkpoints
        .get_mut("https://example.test/file")
        .unwrap();
    new.sequence = 3;
    new.downloaded = 6;
    new.prefix_blake3 = blake3::hash(b"abcdta").to_hex().to_string();
    let prepared = reconcile(&mut f.state, &f.dir);
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].record.sequence, 3);
    assert_eq!(std::fs::read(&f.partial).unwrap(), b"abcdta");
    let sidecar: RecoveryRecord =
        serde_json::from_slice(&std::fs::read(&f.sidecar).unwrap()).unwrap();
    assert_eq!(sidecar.sequence, 3);
}

#[test]
fn accepts_an_already_published_file_only_after_full_hash_match() {
    let mut f = Fixture::new();
    let complete = f
        .state
        .checkpoints
        .get_mut("https://example.test/file")
        .unwrap();
    complete.downloaded = 12;
    complete.prefix_blake3 = blake3::hash(b"abcdefghijkl").to_hex().to_string();
    std::fs::write(f.dir.join("file.bin"), b"abcdefghijkl").unwrap();
    assert!(reconcile(&mut f.state, &f.dir).is_empty());
    assert_eq!(
        f.state.tasks["https://example.test/file"].state,
        "completed"
    );
}

#[test]
fn ignores_checkpoint_from_an_older_attempt_of_the_same_url() {
    let mut f = Fixture::new();
    let old = f
        .state
        .checkpoints
        .get_mut("https://example.test/file")
        .unwrap();
    old.partial_name = ".rget-older.part".into();
    old.sequence = 99;
    let prepared = reconcile(&mut f.state, &f.dir);
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].record.partial_name, ".rget-1-1.part");
}

use super::*;
use crate::daemon::checkpoint::{write_atomic, RecoveryRecord};

#[test]
fn same_url_valid_checkpoint_resumes_and_invalid_never_truncates() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rget-retry-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let partial = root.join(".rget-1-1.part");
    let sidecar = root.join(".rget-1-1.part.meta");
    let url = "https://example.test/file";
    let prior: DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{url}","state":"failed: interrupted","filename":"file.bin","downloaded":4,"total":12,"speed_bps":0,"final_path":null,"partial_name":".rget-1-1.part"}}"#)).unwrap();
    let record = RecoveryRecord {
        version: 1,
        sequence: 1,
        task_url: url.into(),
        resolved_url: url.into(),
        filename: "file.bin".into(),
        partial_name: ".rget-1-1.part".into(),
        etag: "\"v1\"".into(),
        total: 12,
        downloaded: 4,
        prefix_blake3: blake3::hash(b"abcd").to_hex().to_string(),
    };
    let mut saved = SavedState::default();
    std::fs::write(&partial, b"abcd").unwrap();
    write_atomic(&sidecar, &record).unwrap();
    assert!(matches!(
        decide(&mut saved, &root, &prior),
        Decision::Single(_)
    ));
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
    for corrupt in [true, false] {
        std::fs::write(&partial, b"abcdtail").unwrap();
        if corrupt {
            std::fs::write(&sidecar, b"invalid").unwrap();
        } else {
            std::fs::remove_file(&sidecar).unwrap();
        }
        assert!(matches!(decide(&mut saved, &root, &prior), Decision::Fresh));
        assert_eq!(std::fs::read(&partial).unwrap(), b"abcdtail");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn retry_renames_even_when_the_original_name_is_free() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rget-retry-name-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("file.bin");
    assert_eq!(
        super::super::collision::available_from(&path, true)
            .await
            .unwrap(),
        root.join("file (1).bin")
    );
    std::fs::write(root.join("file (1).bin"), b"existing").unwrap();
    assert_eq!(
        super::super::collision::available_from(&path, true)
            .await
            .unwrap(),
        root.join("file (2).bin")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn valid_split_checkpoint_is_selected_without_rewriting_the_partial() {
    use crate::daemon::split::checkpoint::{SegmentRecord, SplitRecord};
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rget-split-retry-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let url = "https://example.test/file";
    let partial = root.join(".rget-1-1.part");
    let prior: DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{url}","state":"failed: interrupted","filename":"file.bin","downloaded":4,"total":8,"speed_bps":0,"final_path":null,"partial_name":".rget-1-1.part"}}"#)).unwrap();
    let segments = vec![
        SegmentRecord {
            start: 0,
            end: 4,
            downloaded: 4,
            hash: blake3::hash(b"abcd").to_hex().to_string(),
        },
        SegmentRecord {
            start: 4,
            end: 8,
            downloaded: 0,
            hash: blake3::hash(b"").to_hex().to_string(),
        },
    ];
    let record = SplitRecord {
        version: 2,
        sequence: 1,
        task_url: url.into(),
        resolved_url: url.into(),
        filename: "file.bin".into(),
        partial_name: ".rget-1-1.part".into(),
        etag: "\"v1\"".into(),
        total: 8,
        requested_connections: 2,
        segments,
    };
    std::fs::write(&partial, b"abcdtail").unwrap();
    write_atomic(&root.join(".rget-1-1.part.meta"), &record).unwrap();
    assert!(matches!(
        decide(&mut SavedState::default(), &root, &prior),
        Decision::Split(_)
    ));
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcdtail");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn confirmed_retry_never_truncates_on_name_mismatch_or_uncheckpointed_tail() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rget-confirmed-retry-validation-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let partial = root.join(".rget-1-1.part");
    let sidecar = root.join(".rget-1-1.part.meta");
    let url = "https://example.test/file";
    let mut prior: DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{url}","state":"failed: interrupted","filename":"other.bin","downloaded":4,"total":12,"speed_bps":0,"final_path":null,"confirmed_filename":"approved.bin","partial_name":".rget-1-1.part"}}"#)).unwrap();
    let mut record = RecoveryRecord {
        version: 1,
        sequence: 1,
        task_url: url.into(),
        resolved_url: url.into(),
        filename: "other.bin".into(),
        partial_name: ".rget-1-1.part".into(),
        etag: "\"v1\"".into(),
        total: 12,
        downloaded: 4,
        prefix_blake3: blake3::hash(b"abcd").to_hex().to_string(),
    };
    std::fs::write(&partial, b"abcdtail").unwrap();
    write_atomic(&sidecar, &record).unwrap();
    assert!(matches!(
        decide(&mut SavedState::default(), &root, &prior),
        Decision::Fresh
    ));
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcdtail");
    record.filename = "approved.bin".into();
    prior.filename = Some("approved.bin".into());
    write_atomic(&sidecar, &record).unwrap();
    assert!(matches!(
        decide(&mut SavedState::default(), &root, &prior),
        Decision::Fresh
    ));
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcdtail");
    std::fs::write(&partial, b"abcd").unwrap();
    let mut saved = SavedState::default();
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(&partial).unwrap();
    saved.http_partial_owners.insert(
        url.into(),
        super::super::http_partial_owner::Owner {
            name: ".rget-1-1.part".into(),
            device: meta.dev(),
            inode: meta.ino(),
        },
    );
    assert!(matches!(
        decide(&mut saved, &root, &prior),
        Decision::Single(_)
    ));
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_never_requeues_confirmed_task_without_recorded_partial() {
    let url = "https://example.test/file";
    let previous: DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{url}","state":"downloading","filename":"file.bin","downloaded":4,"total":12,"speed_bps":0,"final_path":null,"confirmed_filename":"file.bin","partial_name":null}}"#)).unwrap();
    let mut failed = previous.clone();
    failed.state = "failed: recovery unavailable: no partial".into();
    let mut saved = SavedState::default();
    saved.tasks.insert(url.into(), failed);
    let old = std::collections::BTreeMap::from([(url.into(), previous)]);
    let states = std::collections::HashMap::from([(url.into(), "downloading".into())]);
    assert!(startup_fallbacks(&mut saved, &old, &states).is_empty());
    assert!(saved.tasks[url]
        .state
        .contains("cannot resume automatically"));
    assert!(!saved.tasks[url].resume_supported);
}

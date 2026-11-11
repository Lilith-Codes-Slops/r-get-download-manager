use super::{
    torrent_stage_recovery::{recover, Report},
    torrent_stage_store::{self as store, State},
};
use crate::protocols::torrent::staging_tree::{StageArea, StageRecord};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rget-stage-recovery-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}
fn reserve(database: &Path, area: &StageArea, id: &str, payload: bool) -> StageRecord {
    let name = StageArea::candidate_name();
    store::reserve(database, id, &name, 1).unwrap();
    let stage = area.create_reserved(&name, id).unwrap();
    store::confirm(database, &stage).unwrap();
    if payload {
        area.create_payload(&stage).unwrap();
    }
    stage
}
fn stage_path(root: &Path, record: &StageRecord) -> PathBuf {
    root.join(".rget-staging").join(&record.name)
}

#[test]
fn cleans_only_recorded_owned_interrupted_payloads_and_keeps_ambiguous_cases() {
    let root = lab();
    let database = root.join("state/queue.sqlite3");
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let area = StageArea::open(&File::open(&downloads).unwrap()).unwrap();
    let active = reserve(&database, &area, "active", true);
    fs::write(
        stage_path(&downloads, &active).join("payload/partial"),
        b"data",
    )
    .unwrap();
    let retained = reserve(&database, &area, "retained", true);
    store::retain(&database, &retained).unwrap();
    let missing_payload = reserve(&database, &area, "ambiguous", false);
    let invalid = reserve(&database, &area, "invalid", true);
    fs::write(
        stage_path(&downloads, &invalid).join(".rget-owner"),
        b"wrong",
    )
    .unwrap();
    let reserved_name = StageArea::candidate_name();
    store::reserve(&database, "reserved", &reserved_name, 1).unwrap();
    let unrecorded = area.create("unrecorded").unwrap();
    let report = recover(&database, &downloads).unwrap();
    assert_eq!(
        report,
        Report {
            cleaned: 1,
            retained: 1,
            unresolved: 3
        }
    );
    assert!(!stage_path(&downloads, &active).exists());
    assert!(area.verify(&retained).is_ok());
    assert!(area.verify(&missing_payload).is_ok());
    assert!(stage_path(&downloads, &invalid).exists());
    assert!(area.verify(&unrecorded).is_ok());
    let entries = store::load(&database).unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries
        .iter()
        .any(|entry| entry.task_id == "retained" && entry.state == State::Retained));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_or_untrusted_parent_does_not_create_or_delete_staging() {
    let root = lab();
    let database = root.join("state/queue.sqlite3");
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let area = StageArea::open(&File::open(&downloads).unwrap()).unwrap();
    let stage = reserve(&database, &area, "active", true);
    fs::rename(
        downloads.join(".rget-staging"),
        downloads.join("old-staging"),
    )
    .unwrap();
    let report = recover(&database, &downloads).unwrap();
    assert_eq!(report.unresolved, 1);
    assert!(!downloads.join(".rget-staging").exists());
    assert!(downloads.join("old-staging").join(&stage.name).exists());
    assert_eq!(store::load(&database).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

use super::{
    torrent_publish_intent::{self as journal, Observation},
    torrent_stage_store as store,
};
use crate::protocols::torrent::{directory_publish, staging_tree::StageArea};
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rget-intent-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}
#[test]
fn journal_requires_retained_identity_and_blocks_cleanup() {
    let root = lab();
    let db = root.join("state/queue.sqlite3");
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let area = StageArea::open(&File::open(&downloads).unwrap()).unwrap();
    let name = StageArea::candidate_name();
    store::reserve(&db, "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap();
    store::confirm(&db, &stage).unwrap();
    area.create_payload(&stage).unwrap();
    assert!(journal::begin(&db, &area, &stage, "bundle").is_err());
    store::retain(&db, &stage).unwrap();
    assert!(journal::begin(&db, &area, &stage, "../escape").is_err());
    let intent = journal::begin(&db, &area, &stage, "bundle").unwrap();
    assert_eq!(journal::load(&db).unwrap(), vec![intent.clone()]);
    assert!(journal::begin(&db, &area, &stage, "other").is_err());
    assert!(store::cleanup_recorded(&db, &area, "task", &name, true).is_err());
    assert!(store::forget(&db, &stage).is_err());
    assert!(area.verify(&stage).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(target_env = "gnu")]
fn exact_collision_never_overwrites_and_rename_can_be_observed() {
    let root = lab();
    let db = root.join("state/queue.sqlite3");
    let downloads = root.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let destination = File::open(&downloads).unwrap();
    let area = StageArea::open(&destination).unwrap();
    let name = StageArea::candidate_name();
    store::reserve(&db, "task", &name, 1).unwrap();
    let stage = area.create_reserved(&name, "task").unwrap();
    store::confirm(&db, &stage).unwrap();
    area.create_payload(&stage).unwrap();
    store::retain(&db, &stage).unwrap();
    let intent = journal::begin(&db, &area, &stage, "bundle").unwrap();
    let stage_dir = area.verify(&stage).unwrap();
    fs::create_dir(downloads.join("bundle")).unwrap();
    assert_eq!(
        journal::inspect(&area, &destination, &intent).unwrap(),
        Observation::SourcePresent
    );
    assert_eq!(
        directory_publish::publish_exact(&stage_dir, "payload", &destination, "bundle")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert!(downloads.join("bundle").is_dir());
    fs::remove_dir(downloads.join("bundle")).unwrap();
    let published =
        directory_publish::publish_exact(&stage_dir, "payload", &destination, "bundle").unwrap();
    assert_eq!(published.name, "bundle");
    assert_eq!(
        journal::inspect(&area, &destination, &intent).unwrap(),
        Observation::DestinationMatches
    );
    assert_eq!(journal::load(&db).unwrap(), vec![intent]); // observation does not mutate state
    fs::remove_dir_all(root).unwrap();
}

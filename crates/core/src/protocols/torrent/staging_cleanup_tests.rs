use super::{
    staging_cleanup::cleanup,
    staging_tree::{StageArea, StageRecord},
};
use std::{
    fs::{self, File},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn setup() -> (PathBuf, StageArea, StageRecord) {
    let root = std::env::temp_dir().join(format!(
        "rget-clean-stage-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let area = StageArea::open(&File::open(&root).unwrap()).unwrap();
    let record = area.create("torrent-id").unwrap();
    area.create_payload(&record).unwrap();
    (root, area, record)
}
fn stage(root: &Path, record: &StageRecord) -> PathBuf {
    root.join(".rget-staging").join(&record.name)
}
#[test]
fn removes_nested_owned_tree_but_leaves_unrelated_stage() {
    let (root, area, record) = setup();
    let p = stage(&root, &record).join("payload");
    fs::create_dir(p.join("dir")).unwrap();
    fs::write(p.join("dir/file"), b"partial").unwrap();
    let other = area.create("other-task").unwrap();
    cleanup(&area, &record).unwrap();
    assert!(!stage(&root, &record).exists());
    assert!(area.verify(&other).is_ok());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_bad_identity_unsafe_entries_and_does_not_touch_external_data() {
    let (root, area, record) = setup();
    let p = stage(&root, &record).join("payload");
    fs::write(root.join("outside"), b"original").unwrap();
    fs::write(p.join("good"), b"partial").unwrap();
    symlink(root.join("outside"), p.join("bad")).unwrap();
    let mut wrong = record.clone();
    wrong.inode = wrong.inode.wrapping_add(1);
    assert!(cleanup(&area, &wrong).is_err());
    assert!(cleanup(&area, &record).is_err());
    assert_eq!(fs::read(p.join("good")).unwrap(), b"partial");
    assert_eq!(fs::read(root.join("outside")).unwrap(), b"original");
    fs::remove_file(p.join("bad")).unwrap();
    fs::hard_link(root.join("outside"), p.join("hardlink")).unwrap();
    assert!(cleanup(&area, &record).is_err());
    assert!(p.join("good").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_unknown_stage_root_entry_without_removing_anything() {
    let (root, area, record) = setup();
    let s = stage(&root, &record);
    fs::write(s.join("unknown"), b"keep").unwrap();
    assert!(cleanup(&area, &record).is_err());
    assert!(s.join("unknown").exists());
    assert!(s.join(".rget-owner").exists());
    fs::remove_dir_all(root).unwrap();
}

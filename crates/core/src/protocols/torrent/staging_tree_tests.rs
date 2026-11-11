use super::staging_tree::StageArea;
use std::{
    fs::{self, File},
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rget-stage-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn unique_owned_directories_and_marker_are_checked() {
    let root = lab();
    let file = File::open(&root).unwrap();
    let area = StageArea::open(&file).unwrap();
    let a = area.create("task-one").unwrap();
    let b = area.create("task-two").unwrap();
    assert_ne!(a.name, b.name);
    assert!(area.verify(&a).is_ok());
    assert!(area.verify(&b).is_ok());
    let mut fake = a.clone();
    fake.task_id = "wrong-owner".into();
    assert!(area.verify(&fake).is_err());
    fake = a.clone();
    fake.inode = fake.inode.wrapping_add(1);
    assert!(area.verify(&fake).is_err());
    fs::write(
        root.join(".rget-staging").join(&a.name).join(".rget-owner"),
        b"tampered",
    )
    .unwrap();
    assert!(area.verify(&a).is_err());
    assert!(area.verify(&b).is_ok());
    let marker = root.join(".rget-staging").join(&b.name).join(".rget-owner");
    fs::remove_file(&marker).unwrap();
    symlink("/etc/passwd", &marker).unwrap();
    assert!(area.verify(&b).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unsafe_parent_and_symlinked_stage() {
    let root = lab();
    let file = File::open(&root).unwrap();
    fs::create_dir(root.join("other")).unwrap();
    symlink("other", root.join(".rget-staging")).unwrap();
    assert!(StageArea::open(&file).is_err());
    fs::remove_file(root.join(".rget-staging")).unwrap();
    fs::create_dir(root.join(".rget-staging")).unwrap();
    fs::set_permissions(
        root.join(".rget-staging"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(StageArea::open(&file).is_err());
    fs::set_permissions(
        root.join(".rget-staging"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let area = StageArea::open(&file).unwrap();
    let record = area.create("task").unwrap();
    let path = root.join(".rget-staging").join(&record.name);
    fs::rename(&path, root.join("moved")).unwrap();
    symlink("../moved", &path).unwrap();
    assert!(area.verify(&record).is_err());
    fs::remove_dir_all(root).unwrap();
}

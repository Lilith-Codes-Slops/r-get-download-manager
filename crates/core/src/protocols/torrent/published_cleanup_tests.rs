use super::published_cleanup::{cleanup, PublishedIdentity};
use std::{fs::{self, File}, os::unix::fs::{symlink, MetadataExt, PermissionsExt}, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn lab() -> (PathBuf, File, PublishedIdentity) {
    let root = std::env::temp_dir().join(format!("rget-published-clean-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&root).unwrap(); fs::create_dir(root.join("bundle")).unwrap();
    fs::set_permissions(root.join("bundle"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.join("bundle/dir")).unwrap(); fs::write(root.join("bundle/dir/file"), b"data").unwrap();
    let parent = File::open(&root).unwrap(); let p = parent.metadata().unwrap(); let d = fs::metadata(root.join("bundle")).unwrap();
    let record = PublishedIdentity { name: "bundle".into(), root_device: p.dev(), root_inode: p.ino(), device: d.dev(), inode: d.ino() };
    (root, parent, record)
}
#[test]
fn removes_only_owned_directory() {
    let (root, parent, record) = lab(); fs::write(root.join("unrelated"), b"keep").unwrap();
    cleanup(&parent, &record).unwrap(); assert!(!root.join("bundle").exists());
    assert_eq!(fs::read(root.join("unrelated")).unwrap(), b"keep"); assert!(cleanup(&parent, &record).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn refuses_replacement_symlinks_and_hardlinks_before_deleting_anything() {
    let (root, parent, record) = lab();
    let mut wrong = record.clone(); wrong.inode = wrong.inode.wrapping_add(1); assert!(cleanup(&parent, &wrong).is_err());
    fs::write(root.join("external"), b"untouched").unwrap(); symlink("../../external", root.join("bundle/dir/link")).unwrap();
    assert!(cleanup(&parent, &record).is_err()); assert!(root.join("bundle/dir/file").exists());
    fs::remove_file(root.join("bundle/dir/link")).unwrap();
    fs::hard_link(root.join("bundle/dir/file"), root.join("outside-link")).unwrap();
    assert!(cleanup(&parent, &record).is_err()); assert!(root.join("bundle/dir/file").exists());
    assert_eq!(fs::read(root.join("external")).unwrap(), b"untouched");
    fs::remove_dir_all(root).unwrap();
}

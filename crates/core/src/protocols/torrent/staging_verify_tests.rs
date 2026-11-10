use super::{
    staging_manifest::{ManifestFile, ValidatedManifest},
    staging_tree::StageArea,
    staging_verify::verify_and_sync,
};
use std::{
    fs::{self, File},
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn setup() -> (
    PathBuf,
    StageArea,
    super::staging_tree::StageRecord,
    ValidatedManifest,
) {
    let root = std::env::temp_dir().join(format!(
        "rget-verify-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let area = StageArea::open(&File::open(&root).unwrap()).unwrap();
    let record = area.create("torrent-id").unwrap();
    area.create_payload(&record).unwrap();
    let manifest = ValidatedManifest::new(
        "root".into(),
        vec![
            ManifestFile {
                components: vec!["folder".into(), "a".into()],
                length: 3,
            },
            ManifestFile {
                components: vec!["empty".into()],
                length: 0,
            },
        ],
        3,
    )
    .unwrap();
    (root, area, record, manifest)
}
fn payload(root: &std::path::Path, name: &str) -> PathBuf {
    root.join(".rget-staging").join(name).join("payload")
}
fn write_expected(path: &std::path::Path) {
    fs::create_dir(path.join("folder")).unwrap();
    fs::write(path.join("folder/a"), b"abc").unwrap();
    fs::write(path.join("empty"), b"").unwrap();
}
#[test]
fn exact_tree_is_verified_and_missing_or_wrong_length_is_retained() {
    let (root, area, record, manifest) = setup();
    let p = payload(&root, &record.name);
    assert!(verify_and_sync(&area, &record, &manifest).is_err());
    write_expected(&p);
    assert!(verify_and_sync(&area, &record, &manifest).is_ok());
    fs::write(p.join("folder/a"), b"wrong").unwrap();
    assert!(verify_and_sync(&area, &record, &manifest).is_err());
    assert!(p.join("folder/a").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_extras_symlinks_and_hardlinks() {
    let (root, area, record, manifest) = setup();
    let p = payload(&root, &record.name);
    write_expected(&p);
    fs::write(p.join("extra"), b"x").unwrap();
    assert!(verify_and_sync(&area, &record, &manifest).is_err());
    fs::remove_file(p.join("extra")).unwrap();
    fs::remove_file(p.join("folder/a")).unwrap();
    symlink("../empty", p.join("folder/a")).unwrap();
    assert!(verify_and_sync(&area, &record, &manifest).is_err());
    fs::remove_file(p.join("folder/a")).unwrap();
    fs::write(root.join("external"), b"abc").unwrap();
    fs::hard_link(root.join("external"), p.join("folder/a")).unwrap();
    assert!(verify_and_sync(&area, &record, &manifest).is_err());
    assert_eq!(fs::read(root.join("external")).unwrap(), b"abc");
    fs::remove_dir_all(root).unwrap();
}

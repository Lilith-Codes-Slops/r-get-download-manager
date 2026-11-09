use super::{multi_file_metadata, recovery_metadata, recovery_piece_verify};
use librqbit_sha1_wrapper::{ISha1, Sha1};
use std::{fs::{self, File}, os::unix::fs::{symlink, MetadataExt, PermissionsExt}, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn sha(data: &[u8]) -> [u8; 20] { let mut sha = Sha1::new(); sha.update(data); sha.finish() }
fn lab() -> (PathBuf, Vec<u8>, String, super::staging_manifest::ValidatedManifest) {
    let root = std::env::temp_dir().join(format!("rget-piece-check-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir(&root).unwrap(); fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.join("dir")).unwrap(); fs::write(root.join("a"), b"abc").unwrap();
    fs::write(root.join("dir/b"), b"defgh").unwrap(); fs::write(root.join("empty"), b"").unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl1:aee".to_vec();
    bytes.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyee");
    bytes.extend_from_slice(b"d6:lengthi5e4:pathl3:dir1:beee4:name6:bundle12:piece lengthi4e6:pieces40:");
    bytes.extend_from_slice(&sha(b"abcd")); bytes.extend_from_slice(&sha(b"efgh")); bytes.extend_from_slice(b"ee");
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    let manifest = multi_file_metadata::preflight_local(&bytes).unwrap().unwrap();
    let info = recovery_metadata::from_metainfo(&bytes, &hash, &manifest).unwrap();
    (root, info, hash, manifest)
}
#[test]
fn verifies_cross_file_pieces_and_rejects_same_size_corruption() {
    let (root, info, hash, manifest) = lab(); let dir = File::open(&root).unwrap(); let dev = dir.metadata().unwrap().dev();
    recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).unwrap();
    fs::write(root.join("dir/b"), b"dEfgh").unwrap();
    assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn refuses_symlinks_hardlinks_extra_files_and_wrong_metadata() {
    let (root, info, hash, manifest) = lab(); let dir = File::open(&root).unwrap(); let dev = dir.metadata().unwrap().dev();
    assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, "0000000000000000000000000000000000000000").is_err());
    fs::write(root.join("extra"), b"").unwrap(); assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).is_err()); fs::remove_file(root.join("extra")).unwrap();
    fs::remove_file(root.join("a")).unwrap(); symlink("dir/b", root.join("a")).unwrap();
    assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).is_err()); fs::remove_file(root.join("a")).unwrap();
    fs::write(root.join("a"), b"abc").unwrap(); let link = root.with_extension("link"); fs::hard_link(root.join("a"), &link).unwrap();
    assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).is_err());
    fs::remove_file(link).unwrap(); fs::remove_dir_all(root).unwrap();
}

#[test]
fn verifies_short_final_piece_and_rejects_wrong_length() {
    let (root, _, _, _) = lab(); let dir = File::open(&root).unwrap(); let dev = dir.metadata().unwrap().dev();
    fs::write(root.join("dir/b"), b"defg").unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl1:aee".to_vec();
    bytes.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyee");
    bytes.extend_from_slice(b"d6:lengthi4e4:pathl3:dir1:beee4:name6:bundle12:piece lengthi4e6:pieces40:");
    bytes.extend_from_slice(&sha(b"abcd")); bytes.extend_from_slice(&sha(b"efg")); bytes.extend_from_slice(b"ee");
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    let manifest = multi_file_metadata::preflight_local(&bytes).unwrap().unwrap();
    let info = recovery_metadata::from_metainfo(&bytes, &hash, &manifest).unwrap();
    recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).unwrap();
    fs::write(root.join("dir/b"), b"defgh").unwrap();
    assert!(recovery_piece_verify::verify(&dir, dev, &manifest, &info, &hash).is_err());
    fs::remove_dir_all(root).unwrap();
}

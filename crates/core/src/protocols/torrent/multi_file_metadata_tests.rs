use super::multi_file_metadata::{extract, file_records, preflight_local};
use librqbit::{TorrentMetaV1File, TorrentMetaV1Info};

fn file(path: &[&str], length: u64) -> TorrentMetaV1File<Vec<u8>> {
    TorrentMetaV1File { length, path: path.iter().map(|part| part.as_bytes().to_vec()).collect(), attr: None, sha1: None, symlink_path: None }
}
fn info(files: Vec<TorrentMetaV1File<Vec<u8>>>) -> TorrentMetaV1Info<Vec<u8>> {
    TorrentMetaV1Info { name: Some(b"bundle".to_vec()), files: Some(files), piece_length: 16384, pieces: vec![0; 20], ..Default::default() }
}
#[test]
fn extracts_multi_file_records_including_empty_files() {
    let manifest = extract(&info(vec![file(&["a"], 3), file(&["dir", "empty"], 0)])).unwrap();
    assert_eq!(manifest.root_name(), "bundle"); assert_eq!(manifest.total_bytes(), 3);
    let files = file_records(&manifest); assert_eq!(files.len(), 2);
    assert_eq!(files[1].path, "dir/empty"); assert_eq!(files[1].length, 0);
    assert!(files.iter().all(|file| file.priority == 1));
}
#[test]
fn rejects_unsafe_paths_conflicts_and_non_utf8() {
    for files in [vec![file(&["..", "escape"], 3)], vec![file(&["/abs"], 3)],
        vec![file(&[""], 3)], vec![file(&["a"], 1), file(&["a"], 2)],
        vec![file(&["dir"], 1), file(&["dir", "a"], 2)]] {
        assert!(extract(&info(files)).is_err());
    }
    let mut bad = info(vec![file(&["a"], 3)]); bad.files.as_mut().unwrap()[0].path[0] = vec![255];
    assert!(extract(&bad).is_err());
}
#[test]
fn rejects_length_overflow_hash_mismatch_and_special_files_before_engine_validation() {
    assert!(extract(&info(vec![file(&["a"], u64::MAX), file(&["b"], 1)])).is_err());
    let mut bad = info(vec![file(&["a"], 3)]); bad.piece_length = 0; assert!(extract(&bad).is_err());
    bad.piece_length = 16384; bad.pieces.clear(); assert!(extract(&bad).is_err());
    bad.pieces = vec![0; 20]; bad.files.as_mut().unwrap()[0].attr = Some(b"p".to_vec()); assert!(extract(&bad).is_err());
    bad.files.as_mut().unwrap()[0].attr = None; bad.files.as_mut().unwrap()[0].symlink_path = Some(vec![b"target".to_vec()]);
    assert!(extract(&bad).is_err());
}

#[test]
fn local_bencoded_metainfo_extracts_without_a_session_or_filesystem() {
    let mut bytes = b"d4:infod5:filesl".to_vec();
    bytes.extend_from_slice(b"d6:lengthi3e4:pathl1:aee");
    bytes.extend_from_slice(b"d6:lengthi0e4:pathl3:dir5:emptyee");
    bytes.extend_from_slice(b"e4:name6:bundle12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&[0; 20]); bytes.extend_from_slice(b"ee");
    let manifest = preflight_local(&bytes).unwrap().unwrap();
    assert_eq!(manifest.root_name(), "bundle"); assert_eq!(file_records(&manifest).len(), 2);
    assert!(preflight_local(b"not bencode").is_err());
}

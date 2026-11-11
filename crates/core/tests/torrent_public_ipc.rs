#![cfg(all(target_os = "linux", target_env = "gnu", feature = "torrent"))]
//! Opt-in public-network torrent tests with local metainfo and real peers.
use downloader_core::protocols::torrent::{
    multi_file_layout, multi_file_metadata, recovery_metadata, recovery_piece_verify,
};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Daemon {
    root: PathBuf,
    child: Child,
}
impl Daemon {
    fn start(root: PathBuf) -> Self {
        fs::create_dir_all(root.join("run")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root)
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("RGET_TORRENT_MAX_ACTIVE", "1")
            .env_remove("RGET_TORRENT_TEST_DIR")
            .env_remove("RGET_TORRENT_TEST_POINT")
            .env_remove("RGET_TEST_CHECKPOINT_HOOK")
            .env_remove("RGET_TEST_CHECKPOINT_MARKER")
            .env_remove("RGET_TEST_WRITE_BLOCK")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Self { root, child };
        let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(daemon.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            thread::sleep(Duration::from_millis(50));
        }
        daemon
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn task(&self, id: &str) -> Option<DownloadInfo> {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks.into_iter().find(|task| task.url == id),
            other => panic!("queue request failed: {other:?}"),
        }
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(serde::Deserialize, Default)]
struct V2Info {
    #[serde(rename = "meta version", default)]
    meta_version: Option<serde::de::IgnoredAny>,
    #[serde(rename = "file tree", default)]
    file_tree: Option<serde::de::IgnoredAny>,
}
#[derive(serde::Deserialize)]
struct V2Metainfo {
    #[serde(rename = "meta version", default)]
    meta_version: Option<serde::de::IgnoredAny>,
    #[serde(rename = "piece layers", default)]
    piece_layers: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    info: V2Info,
}
/// Reject v2/hybrid markers before any download. The v1 parser ignores these
/// unknown fields, so it alone cannot prove that a torrent is v1-only.
fn assert_v1_only(bytes: &[u8]) {
    let flags: V2Metainfo =
        librqbit_bencode::from_bytes(bytes).expect("cannot inspect bencoded metainfo");
    assert!(
        flags.meta_version.is_none()
            && flags.piece_layers.is_none()
            && flags.info.meta_version.is_none()
            && flags.info.file_tree.is_none(),
        "v2 or hybrid torrents are unsupported; these tests require v1-only metainfo"
    );
}

struct SingleFile {
    name: String,
    length: u64,
    piece_length: u64,
    hashes: Vec<u8>,
    info_hash: String,
}
fn inspect_single_file(bytes: &[u8]) -> SingleFile {
    assert_v1_only(bytes);
    let parsed = librqbit::torrent_from_bytes(bytes).expect("cannot parse metainfo");
    let info = &parsed.info.data;
    assert!(
        info.files.is_none() && !info.private,
        "expected a public v1 single-file torrent"
    );
    let raw_name = info.name.as_ref().expect("torrent name missing").as_ref();
    let name = std::str::from_utf8(raw_name)
        .expect("torrent name is not UTF-8")
        .to_owned();
    multi_file_layout::validate_component(&name).expect("unsafe torrent name");
    let length = info.length.expect("single-file length missing");
    let piece_length = u64::from(info.piece_length);
    assert!(
        length > 0 && piece_length > 0,
        "zero length or piece length is unsupported"
    );
    let hashes = info.pieces.as_ref().to_vec();
    assert_eq!(hashes.len() % 20, 0, "invalid v1 piece hash list");
    let pieces =
        usize::try_from(length.div_ceil(piece_length)).expect("piece count exceeds address space");
    assert_eq!(
        hashes.len() / 20,
        pieces,
        "v1 piece count does not match file length"
    );
    SingleFile {
        name,
        length,
        piece_length,
        hashes,
        info_hash: parsed.info_hash.as_string(),
    }
}
/// Direct single-file v1 piece-hash pass. This is not the multi-file recovery
/// verifier and does not count toward multi-file acceptance.
fn verify_single_file(path: &Path, meta: &SingleFile) {
    use librqbit_sha1_wrapper::{ISha1, Sha1};
    let sym = fs::symlink_metadata(path).expect("published file missing");
    let parent = path.parent().expect("published file has no parent");
    assert!(
        sym.is_file() && sym.nlink() == 1 && sym.len() == meta.length,
        "published file type, links or length mismatch: {path:?}"
    );
    assert_eq!(
        sym.dev(),
        fs::metadata(parent).unwrap().dev(),
        "published file device changed"
    );
    let mut file = File::open(path).unwrap();
    let mut digest = Sha1::new();
    let mut remaining = meta.length;
    let mut in_piece = 0u64;
    let mut index = 0usize;
    let mut buf = [0u8; 65536];
    while remaining > 0 {
        let take = remaining
            .min(meta.piece_length - in_piece)
            .min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..take]).unwrap();
        digest.update(&buf[..take]);
        remaining -= take as u64;
        in_piece += take as u64;
        if in_piece == meta.piece_length {
            let hash = std::mem::replace(&mut digest, Sha1::new()).finish();
            assert_eq!(
                meta.hashes.get(index..index + 20),
                Some(hash.as_slice()),
                "v1 piece {index} hash mismatch"
            );
            index += 20;
            in_piece = 0;
        }
    }
    if in_piece != 0 {
        let hash = digest.finish();
        assert_eq!(
            meta.hashes.get(index..index + 20),
            Some(hash.as_slice()),
            "final v1 piece hash mismatch"
        );
        index += 20;
    }
    assert_eq!(index, meta.hashes.len(), "v1 piece count mismatch");
    assert_eq!(
        file.metadata().unwrap().len(),
        meta.length,
        "published file changed during verification"
    );
}

fn public_source(name: &str) -> PathBuf {
    let source = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to a local .torrent file")),
    );
    let source = fs::canonicalize(&source).expect("cannot open torrent file");
    assert!(
        source.is_file()
            && source
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
    );
    source
}
fn public_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("RGET_PUBLIC_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200u64),
    )
}
fn completed_task(daemon: &Daemon, id: &str, timeout: Duration) -> DownloadInfo {
    let deadline = Instant::now() + timeout;
    let mut log = Instant::now();
    loop {
        let task = daemon.task(id).expect("torrent task disappeared");
        assert!(
            !task.state.starts_with("failed:"),
            "download failed: {task:?}"
        );
        if task.state == "completed" {
            return task;
        }
        assert!(
            Instant::now() < deadline,
            "public torrent timed out: {task:?}"
        );
        if log.elapsed() >= Duration::from_secs(15) {
            println!("{}: {} bytes", task.state, task.downloaded);
            log = Instant::now();
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// The accepted public-network milestone: a v1 multi-file torrent with peers.
#[test]
#[ignore = "requires an explicitly selected public v1 multi-file .torrent and reachable peers"]
fn public_multifile_download_and_verify_all_v1_pieces() {
    let source = public_source("RGET_PUBLIC_TORRENT");
    let bytes = fs::read(&source).expect("cannot read torrent file");
    assert_v1_only(&bytes);
    let manifest = multi_file_metadata::preflight_local(&bytes)
        .expect("invalid or unsupported metadata")
        .expect("torrent is not multi-file; acceptance requires a v1 multi-file .torrent");
    assert!(
        manifest.files().len() >= 2,
        "acceptance requires at least two files in the torrent"
    );
    let parsed = librqbit::torrent_from_bytes(&bytes).expect("cannot parse metainfo");
    let hash = parsed.info_hash.as_string();
    let info = recovery_metadata::from_metainfo(&bytes, &hash, &manifest)
        .expect("cannot validate exact info dictionary");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rget-public-torrent-{}-{nonce}",
        std::process::id()
    ));
    println!("isolated test home: {}", root.display());
    println!(
        "expected root: {}; files: {}; bytes: {}; info-hash: {}",
        manifest.root_name(),
        manifest.files().len(),
        manifest.total_bytes(),
        hash
    );
    let daemon = Daemon::start(root.clone());
    let id = match daemon.request(ClientRequest::AddTask {
        url: source.to_str().expect("non-UTF-8 source path").into(),
        mirrors: vec![],
        connections: 8,
        extract: false,
        category: None,
        not_before: None,
    }) {
        DaemonEvent::TaskStateChanged { task_id, .. } => task_id,
        event => panic!("add failed: {event:?}"),
    };
    let task = completed_task(&daemon, &id, public_timeout());
    let identity = task.torrent.as_ref().expect("torrent identity missing");
    assert_eq!(identity.root_name.as_deref(), Some(manifest.root_name()));
    assert_eq!(
        identity.files.as_ref().map(Vec::len),
        Some(manifest.files().len())
    );
    let path = root.join("Downloads").join(manifest.root_name());
    assert_eq!(
        task.final_path.as_deref().map(Path::new),
        Some(path.as_path())
    );
    let payload = File::open(&path).expect("published directory missing");
    let device = payload.metadata().unwrap().dev();
    recovery_piece_verify::verify(&payload, device, &manifest, &info, &hash)
        .expect("post-publication v1 piece verification failed");
    println!(
        "completed: {} ({} files); all v1 pieces verified",
        path.display(),
        manifest.files().len()
    );
    drop(daemon);
    fs::remove_dir_all(&root).expect("failed to remove isolated test home");
}

/// Additional real-world P2P check for an ordinary v1 single-file ISO torrent.
/// It does not satisfy the separately required multi-file acceptance test.
#[test]
#[ignore = "requires an explicitly selected public v1 single-file .torrent and reachable peers"]
fn public_singlefile_download_and_verify_all_v1_pieces() {
    let source = public_source("RGET_PUBLIC_SINGLE_TORRENT");
    let bytes = fs::read(&source).expect("cannot read torrent file");
    let meta = inspect_single_file(&bytes);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rget-public-single-{}-{nonce}", std::process::id()));
    println!("isolated test home: {}", root.display());
    println!(
        "expected file: {}; bytes: {}; pieces: {}; info-hash: {}",
        meta.name,
        meta.length,
        meta.hashes.len() / 20,
        meta.info_hash
    );
    let daemon = Daemon::start(root.clone());
    let id = match daemon.request(ClientRequest::AddTask {
        url: source.to_str().expect("non-UTF-8 source path").into(),
        mirrors: vec![],
        connections: 8,
        extract: false,
        category: None,
        not_before: None,
    }) {
        DaemonEvent::TaskStateChanged { task_id, .. } => task_id,
        event => panic!("add failed: {event:?}"),
    };
    let task = completed_task(&daemon, &id, public_timeout());
    let identity = task.torrent.as_ref().expect("torrent identity missing");
    let files = identity.files.as_ref().expect("recorded files missing");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, meta.name);
    assert_eq!(files[0].length, meta.length);
    assert_eq!(task.total, Some(meta.length));
    let downloads = root.join("Downloads");
    let final_path = PathBuf::from(task.final_path.as_deref().expect("final path missing"));
    assert_eq!(final_path.parent(), Some(downloads.as_path()));
    verify_single_file(&final_path, &meta);
    println!(
        "completed: {} ({} bytes); all v1 pieces verified",
        final_path.display(),
        meta.length
    );
    drop(daemon);
    fs::remove_dir_all(&root).expect("failed to remove isolated test home");
}

#[test]
fn v1_only_guard_accepts_v1_and_rejects_v2_markers() {
    assert_v1_only(b"d4:infod5:filesleee");
    assert!(
        std::panic::catch_unwind(|| assert_v1_only(b"d12:meta versioni2e4:infod5:filesleee"))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| assert_v1_only(b"d4:infod9:file treed5:filesleeee")).is_err()
    );
}
#[test]
fn single_file_verifier_detects_corruption() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rget-single-verify-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("abc.bin");
    fs::write(&path, b"abc").unwrap();
    let mut bytes = b"d4:infod6:lengthi3e4:name7:abc.bin12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50, 0xc2,
        0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
    ]);
    bytes.extend_from_slice(b"ee");
    let meta = inspect_single_file(&bytes);
    verify_single_file(&path, &meta);
    fs::write(&path, b"abd").unwrap();
    assert!(std::panic::catch_unwind(|| verify_single_file(&path, &meta)).is_err());
    fs::remove_dir_all(root).unwrap();
}

#![cfg(all(target_os = "linux", target_env = "gnu", feature = "torrent"))]
//! Opt-in public-network test. Supply a local, public v1 multi-file torrent.
use downloader_core::protocols::torrent::{multi_file_metadata, recovery_metadata, recovery_piece_verify};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::{fs::{self, File}, os::unix::{fs::MetadataExt, net::UnixStream}, path::{Path, PathBuf},
    process::{Child, Command, Stdio}, thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

struct Daemon { root: PathBuf, child: Child }
impl Daemon {
    fn start(root: PathBuf) -> Self {
        fs::create_dir_all(root.join("run")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root).env("XDG_RUNTIME_DIR", root.join("run")).env("XDG_STATE_HOME", root.join("state"))
            .env("RGET_TORRENT_MAX_ACTIVE", "1")
            .env_remove("RGET_TORRENT_TEST_DIR").env_remove("RGET_TORRENT_TEST_POINT")
            .env_remove("RGET_TEST_CHECKPOINT_HOOK").env_remove("RGET_TEST_CHECKPOINT_MARKER").env_remove("RGET_TEST_WRITE_BLOCK")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let daemon = Self { root, child }; let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(daemon.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start"); thread::sleep(Duration::from_millis(50));
        }
        daemon
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap(); FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn task(&self, id: &str) -> Option<DownloadInfo> {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks.into_iter().find(|task| task.url == id),
            other => panic!("queue request failed: {other:?}"),
        }
    }
}
impl Drop for Daemon { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

#[derive(serde::Deserialize, Default)]
struct V2Info {
    #[serde(rename = "meta version", default)] meta_version: Option<serde::de::IgnoredAny>,
    #[serde(rename = "file tree", default)] file_tree: Option<serde::de::IgnoredAny>,
}
#[derive(serde::Deserialize)]
struct V2Metainfo {
    #[serde(rename = "meta version", default)] meta_version: Option<serde::de::IgnoredAny>,
    #[serde(rename = "piece layers", default)] piece_layers: Option<serde::de::IgnoredAny>,
    #[serde(default)] info: V2Info,
}
/// Reject v2/hybrid markers before any download. The v1 parser ignores these
/// unknown fields, so it alone cannot prove that a torrent is v1-only.
fn assert_v1_only(bytes: &[u8]) {
    let flags: V2Metainfo = librqbit_bencode::from_bytes(bytes).expect("cannot inspect bencoded metainfo");
    assert!(flags.meta_version.is_none() && flags.piece_layers.is_none()
        && flags.info.meta_version.is_none() && flags.info.file_tree.is_none(),
        "v2 or hybrid torrents are unsupported; acceptance requires v1-only metainfo");
}

/// This test needs real external peers; it is not an automated default test.
/// The torrent source must be a local file, not a magnet or HTTP URL.
#[test]
#[ignore = "requires an explicitly selected public multi-file .torrent and reachable peers"]
fn public_multifile_download_and_verify_all_v1_pieces() {
    let source = PathBuf::from(std::env::var_os("RGET_PUBLIC_TORRENT").expect("set RGET_PUBLIC_TORRENT to a local .torrent file"));
    let source = fs::canonicalize(&source).expect("cannot open torrent file");
    assert!(source.is_file() && source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("torrent")));
    let bytes = fs::read(&source).expect("cannot read torrent file");
    assert_v1_only(&bytes);
    let manifest = multi_file_metadata::preflight_local(&bytes).expect("invalid or unsupported metadata")
        .expect("torrent is not multi-file; acceptance requires a v1 multi-file .torrent");
    assert!(manifest.files().len() >= 2, "acceptance requires at least two files in the torrent");
    let parsed = librqbit::torrent_from_bytes(&bytes).expect("cannot parse metainfo"); let hash = parsed.info_hash.as_string();
    let info = recovery_metadata::from_metainfo(&bytes, &hash, &manifest).expect("cannot validate exact info dictionary");
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-public-torrent-{}-{nonce}", std::process::id()));
    println!("isolated test home: {}", root.display());
    println!("expected root: {}; files: {}; bytes: {}; info-hash: {}", manifest.root_name(), manifest.files().len(), manifest.total_bytes(), hash);
    let daemon = Daemon::start(root.clone());
    let id = match daemon.request(ClientRequest::AddTask { url: source.to_str().expect("non-UTF-8 source path").into(),
        mirrors: vec![], connections: 8, extract: false, category: None, not_before: None }) {
        DaemonEvent::TaskStateChanged { task_id, .. } => task_id, event => panic!("add failed: {event:?}"),
    };
    let timeout = std::env::var("RGET_PUBLIC_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(1200u64);
    let deadline = Instant::now() + Duration::from_secs(timeout); let mut log = Instant::now();
    let task = loop {
        let task = daemon.task(&id).expect("torrent task disappeared");
        assert!(!task.state.starts_with("failed:"), "download failed: {task:?}");
        if task.state == "completed" { break task; }
        assert!(Instant::now() < deadline, "public torrent timed out: {task:?}");
        if log.elapsed() >= Duration::from_secs(15) { println!("{}: {} bytes", task.state, task.downloaded); log = Instant::now(); }
        thread::sleep(Duration::from_secs(1));
    };
    let identity = task.torrent.as_ref().expect("torrent identity missing");
    assert_eq!(identity.root_name.as_deref(), Some(manifest.root_name()));
    assert_eq!(identity.files.as_ref().map(Vec::len), Some(manifest.files().len()));
    let path = root.join("Downloads").join(manifest.root_name());
    assert_eq!(task.final_path.as_deref().map(Path::new), Some(path.as_path()));
    let payload = File::open(&path).expect("published directory missing"); let device = payload.metadata().unwrap().dev();
    recovery_piece_verify::verify(&payload, device, &manifest, &info, &hash).expect("post-publication v1 piece verification failed");
    println!("completed: {} ({} files); all v1 pieces verified", path.display(), manifest.files().len());
    drop(daemon); fs::remove_dir_all(&root).expect("failed to remove isolated test home");
}

#[test]
fn v1_only_guard_accepts_v1_and_rejects_v2_markers() {
    assert_v1_only(b"d4:infod5:filesleee");
    assert!(std::panic::catch_unwind(|| assert_v1_only(b"d12:meta versioni2e4:infod5:filesleee")).is_err());
    assert!(std::panic::catch_unwind(|| assert_v1_only(b"d4:infod9:file treed5:filesleeee")).is_err());
}

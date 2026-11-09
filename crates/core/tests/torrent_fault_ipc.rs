#![cfg(all(target_os = "linux", target_env = "gnu", feature = "torrent-test-hooks"))]

use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use librqbit::{AddTorrent, AddTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions};
use std::{fs, io::{Read, Write}, net::{TcpListener, TcpStream}, os::unix::net::UnixStream,
    path::{Path, PathBuf}, process::{Child, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

struct Daemon { root: PathBuf, child: Child }
impl Daemon {
    fn start(root: &Path, point: Option<&str>) -> Self {
        fs::create_dir_all(root.join("run")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_downloader-core"));
        // Force two active slots and clear inherited hooks. Otherwise a shell
        // with RGET_TORRENT_MAX_ACTIVE=1 serializes the concurrency test, and
        // leftover fault variables can pause an ostensibly hook-free restart.
        command.env("HOME", root).env("XDG_RUNTIME_DIR", root.join("run")).env("XDG_STATE_HOME", root.join("state"))
            .env("RGET_TORRENT_MAX_ACTIVE", "2")
            .env_remove("RGET_TORRENT_TEST_DIR").env_remove("RGET_TORRENT_TEST_POINT")
            .env_remove("RGET_TEST_CHECKPOINT_HOOK").env_remove("RGET_TEST_CHECKPOINT_MARKER").env_remove("RGET_TEST_WRITE_BLOCK")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(point) = point {
            let hooks = root.join("hooks"); let _ = fs::remove_dir_all(&hooks); fs::create_dir_all(&hooks).unwrap();
            command.env("RGET_TORRENT_TEST_DIR", hooks).env("RGET_TORRENT_TEST_POINT", point);
        }
        let daemon = Self { root: root.into(), child: command.spawn().unwrap() };
        wait(|| UnixStream::connect(daemon.root.join("run/rget.sock")).is_ok(), "daemon start"); daemon
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(40))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap(); FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn add(&self, source: &Path) -> String {
        match self.request(ClientRequest::AddTask { url: source.to_str().unwrap().into(), mirrors: vec![], connections: 1,
            extract: false, category: None, not_before: None }) {
            DaemonEvent::TaskStateChanged { task_id, .. } => task_id, other => panic!("add failed: {other:?}")
        }
    }
    fn task(&self, id: &str) -> Option<DownloadInfo> {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks.into_iter().find(|task| task.url == id), other => panic!("queue: {other:?}")
        }
    }
    fn ready(&self) { wait(|| self.root.join("hooks/ready").exists(), "fault hook"); }
    fn release(&self) { fs::write(self.root.join("hooks/release"), b"go").unwrap(); }
    fn terminal(&self, id: &str) -> DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Some(task) = self.task(id) {
                if task.state == "completed" || task.state == "canceled" || task.state.starts_with("failed:") { return task; }
            }
            assert!(Instant::now() < deadline, "task timed out: {id}"); thread::sleep(Duration::from_millis(50));
        }
    }
}
impl Drop for Daemon { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }
fn wait(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while !condition() { assert!(Instant::now() < deadline, "timed out: {what}"); thread::sleep(Duration::from_millis(20)); }
}
fn tracker_reply(mut stream: TcpStream, port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2))); let mut request = [0; 4096]; let _ = stream.read(&mut request);
    let mut body = b"d8:intervali30e5:peers6:".to_vec(); body.extend_from_slice(&[127, 0, 0, 1]);
    body.extend_from_slice(&port.to_be_bytes()); body.push(b'e');
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()); let _ = stream.write_all(&body);
}
struct Lab { root: PathBuf, home: PathBuf, source: PathBuf, seed: Arc<Session>, stop: Arc<AtomicBool>, tracker: Option<thread::JoinHandle<()>> }
impl Lab {
    async fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-torrent-fault-{}-{nonce}", std::process::id()));
        let home = root.join("home"); let seed_root = root.join("seed"); let payload = seed_root.join("bundle");
        fs::create_dir_all(payload.join("dir")).unwrap(); fs::write(payload.join("dir/a"), b"abc").unwrap(); fs::write(payload.join("empty"), b"").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        let mut torrent = format!("d8:announce{}:{}4:infod5:filesld6:lengthi3e4:pathl3:dir1:aee", url.len(), url).into_bytes();
        torrent.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyeee4:name6:bundle12:piece lengthi16384e6:pieces20:");
        torrent.extend_from_slice(&[0xa9,0x99,0x3e,0x36,0x47,0x06,0x81,0x6a,0xba,0x3e,0x25,0x71,0x78,0x50,0xc2,0x6c,0x9c,0xd0,0xd8,0x9d]);
        torrent.extend_from_slice(b"ee"); let source = root.join("fixture.torrent"); fs::write(&source, &torrent).unwrap();
        let seed = Session::new_with_opts(seed_root, SessionOptions { dht: None, disable_trackers: true,
            disable_local_service_discovery: true, listen: Some(ListenerOptions { mode: ListenerMode::TcpOnly,
                listen_addr: ([127,0,0,1], 0).into(), ..Default::default() }), ..Default::default() }).await.unwrap();
        let port = seed.listen_addr().unwrap().port();
        let handle = seed.add_torrent(AddTorrent::from_bytes(torrent), Some(AddTorrentOptions { overwrite: true,
            output_folder: Some(payload.to_str().unwrap().into()), ..Default::default() })).await.unwrap().into_handle().unwrap();
        tokio::time::timeout(Duration::from_secs(15), handle.wait_until_initialized()).await.unwrap().unwrap(); assert!(handle.stats().finished);
        let stop = Arc::new(AtomicBool::new(false)); let done = stop.clone();
        let tracker = thread::spawn(move || while !done.load(Ordering::Relaxed) {
            match listener.accept() { Ok((stream, _)) => tracker_reply(stream, port),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)), Err(_) => break }
        });
        Self { root, home, source, seed, stop, tracker: Some(tracker) }
    }
    fn stage(&self) -> PathBuf {
        let parent = self.home.join("Downloads/.rget-staging");
        let mut entries = fs::read_dir(parent).unwrap().map(|e| e.unwrap().path());
        let stage = entries.next().expect("stage missing"); assert!(entries.next().is_none()); stage
    }
    async fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed); self.tracker.take().unwrap().join().unwrap(); self.seed.stop().await;
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(tracker) = self.tracker.take() { let _ = tracker.join(); }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publication_crash_boundaries_recover_without_republishing() {
    for point in ["after-rename", "after-completion"] {
        let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some(point)); let id = daemon.add(&lab.source);
        daemon.ready(); let stage = lab.stage(); let destination = lab.home.join("Downloads/bundle/dir/a");
        assert_eq!(fs::read(&destination).unwrap(), b"abc"); assert!(!stage.join("payload").exists());
        drop(daemon); let restarted = Daemon::start(&lab.home, None);
        let task = restarted.terminal(&id); assert_eq!(task.state, "completed");
        assert_eq!(fs::read(&destination).unwrap(), b"abc"); assert!(!stage.exists(), "empty owned stage not cleaned");
        assert!(!lab.home.join("Downloads/bundle (1)").exists());
        drop(restarted); let restarted = Daemon::start(&lab.home, None);
        assert_eq!(restarted.terminal(&id).state, "completed"); assert_eq!(fs::read(destination).unwrap(), b"abc");
        drop(restarted); lab.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tamper_after_barrier_fails_verification_and_retains_stage() {
    let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some("after-barrier")); let id = daemon.add(&lab.source);
    daemon.ready(); assert!(lab.home.join("hooks/late-write-rejected").exists());
    let stage = lab.stage(); fs::write(stage.join("payload/dir/a"), b"abd").unwrap(); daemon.release();
    let task = daemon.terminal(&id); assert!(task.state.starts_with("failed:"), "{task:?}");
    assert!(task.state.contains("piece hash mismatch"), "{task:?}"); assert!(task.final_path.is_none());
    assert!(!lab.home.join("Downloads/bundle").exists()); assert_eq!(fs::read(stage.join("payload/dir/a")).unwrap(), b"abd");
    drop(daemon); let restarted = Daemon::start(&lab.home, None);
    assert!(restarted.task(&id).unwrap().state.starts_with("failed:")); assert!(stage.exists());
    drop(restarted); lab.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn root_replacement_before_publication_fails_closed() {
    let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some("after-barrier")); let id = daemon.add(&lab.source);
    daemon.ready(); let downloads = lab.home.join("Downloads"); let detached = lab.home.join("old-downloads");
    fs::rename(&downloads, &detached).unwrap(); fs::create_dir(&downloads).unwrap(); fs::write(downloads.join("unrelated"), b"safe").unwrap();
    daemon.release(); let task = daemon.terminal(&id); assert!(task.state.starts_with("failed:"), "{task:?}");
    assert!(task.state.contains("download root changed"), "{task:?}");
    assert!(!downloads.join("bundle").exists()); assert!(!detached.join("bundle").exists());
    assert_eq!(fs::read(downloads.join("unrelated")).unwrap(), b"safe");
    drop(daemon); lab.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn destination_collision_keeps_existing_tree_and_owned_stage() {
    let lab = Lab::new().await; fs::create_dir_all(lab.home.join("Downloads/bundle")).unwrap();
    fs::write(lab.home.join("Downloads/bundle/sentinel"), b"safe").unwrap();
    let daemon = Daemon::start(&lab.home, None); let id = daemon.add(&lab.source); let task = daemon.terminal(&id);
    assert!(task.state.starts_with("failed:"), "{task:?}"); assert!(task.final_path.is_none());
    assert_eq!(fs::read(lab.home.join("Downloads/bundle/sentinel")).unwrap(), b"safe");
    let stage = lab.stage(); assert_eq!(fs::read(stage.join("payload/dir/a")).unwrap(), b"abc");
    drop(daemon); let restarted = Daemon::start(&lab.home, None);
    assert!(restarted.task(&id).unwrap().state.starts_with("failed:")); assert!(stage.exists());
    assert!(matches!(restarted.request(ClientRequest::RemoveTask { task_id: id, delete_file: true }), DaemonEvent::TaskStateChanged { state, .. } if state == "removed"));
    assert!(!stage.exists()); assert_eq!(fs::read(lab.home.join("Downloads/bundle/sentinel")).unwrap(), b"safe");
    drop(restarted); lab.finish().await;

    let lab = Lab::new().await; fs::create_dir_all(lab.home.join("Downloads/bundle")).unwrap();
    fs::write(lab.home.join("Downloads/bundle/sentinel"), b"safe").unwrap();
    let daemon = Daemon::start(&lab.home, None); let id = daemon.add(&lab.source);
    assert!(daemon.terminal(&id).state.starts_with("failed:")); let stage = lab.stage(); drop(daemon);
    let restarted = Daemon::start(&lab.home, None);
    assert!(matches!(restarted.request(ClientRequest::RemoveTask { task_id: id, delete_file: false }), DaemonEvent::TaskStateChanged { state, .. } if state == "removed"));
    assert!(stage.exists()); assert_eq!(fs::read(stage.join("payload/dir/a")).unwrap(), b"abc");
    assert_eq!(fs::read(lab.home.join("Downloads/bundle/sentinel")).unwrap(), b"safe");
    drop(restarted); lab.finish().await;
}

fn remove_in_background(home: PathBuf, id: String, delete_file: bool) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut stream = UnixStream::connect(home.join("run/rget.sock")).unwrap();
        FrameEncoder::write_frame(&mut stream, &ClientRequest::RemoveTask { task_id: id, delete_file }).unwrap();
        let _ = FrameDecoder::read_frame::<_, DaemonEvent>(&mut stream);
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_delete_replays_only_unchanged_safe_ownership() {
    for scenario in ["unchanged", "replaced", "hardlink", "symlink"] {
        let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some("delete-intent"));
        let id = daemon.add(&lab.source); assert_eq!(daemon.terminal(&id).state, "completed");
        let remover = remove_in_background(lab.home.clone(), id.clone(), true); daemon.ready(); drop(daemon); remover.join().unwrap();
        let destination = lab.home.join("Downloads/bundle"); let sentinel = lab.home.join("sentinel");
        fs::write(&sentinel, b"outside").unwrap();
        match scenario {
            "replaced" => { fs::rename(&destination, lab.home.join("Downloads/original")).unwrap(); fs::create_dir(&destination).unwrap(); fs::write(destination.join("safe"), b"replacement").unwrap(); }
            "hardlink" => { fs::hard_link(destination.join("dir/a"), lab.home.join("hardlink")).unwrap(); }
            "symlink" => { std::os::unix::fs::symlink(&sentinel, destination.join("link")).unwrap(); }
            _ => {}
        }
        let restarted = Daemon::start(&lab.home, None);
        if scenario == "unchanged" { assert!(restarted.task(&id).is_none()); assert!(!destination.exists()); }
        else {
            assert_eq!(restarted.task(&id).unwrap().state, "completed"); assert!(destination.exists());
            if scenario == "replaced" { assert_eq!(fs::read(destination.join("safe")).unwrap(), b"replacement"); }
            else { assert_eq!(fs::read(destination.join("dir/a")).unwrap(), b"abc"); }
        }
        assert_eq!(fs::read(sentinel).unwrap(), b"outside"); drop(restarted); lab.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_files_crash_is_atomic_before_and_after_commit() {
    for point in ["keep-before-commit", "keep-after-commit"] {
        let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some(point));
        let id = daemon.add(&lab.source); assert_eq!(daemon.terminal(&id).state, "completed");
        let remover = remove_in_background(lab.home.clone(), id.clone(), false); daemon.ready(); drop(daemon); remover.join().unwrap();
        let restarted = Daemon::start(&lab.home, None); let task = restarted.task(&id);
        if point == "keep-before-commit" { assert_eq!(task.unwrap().state, "completed"); } else { assert!(task.is_none()); }
        assert_eq!(fs::read(lab.home.join("Downloads/bundle/dir/a")).unwrap(), b"abc");
        drop(restarted); lab.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_distinct_torrents_same_root_do_not_interfere() {
    let lab = Lab::new().await;
    let bytes = fs::read(&lab.source).unwrap(); let needle = b"12:piece lengthi16384e";
    let offset = bytes.windows(needle.len()).position(|window| window == needle).unwrap();
    let mut second = bytes[..offset].to_vec(); second.extend_from_slice(b"12:piece lengthi32768e"); second.extend_from_slice(&bytes[offset + needle.len()..]);
    let source = lab.root.join("second.torrent"); fs::write(&source, &second).unwrap();
    let handle = lab.seed.add_torrent(AddTorrent::from_bytes(second), Some(AddTorrentOptions { overwrite: true,
        output_folder: Some(lab.root.join("seed/bundle").to_str().unwrap().into()), ..Default::default() })).await.unwrap().into_handle().unwrap();
    tokio::time::timeout(Duration::from_secs(15), handle.wait_until_initialized()).await.unwrap().unwrap(); assert!(handle.stats().finished);
    let daemon = Daemon::start(&lab.home, Some("after-barrier")); let first = daemon.add(&lab.source); let second = daemon.add(&source);
    assert_ne!(first, second); let hooks = lab.home.join("hooks");
    let ready = || fs::read_dir(&hooks).map(|entries| entries.filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-")).count()).unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(60); let mut report = Instant::now();
    while ready() < 2 {
        if report.elapsed() >= Duration::from_secs(5) {
            eprintln!("barriers: {}; first={:?}; second={:?}", ready(), daemon.task(&first), daemon.task(&second));
            report = Instant::now();
        }
        assert!(Instant::now() < deadline, "timed out: both storage barriers; first={:?}; second={:?}", daemon.task(&first), daemon.task(&second));
        thread::sleep(Duration::from_millis(50));
    }
    daemon.release(); let a = daemon.terminal(&first); let b = daemon.terminal(&second);
    assert!((a.state == "completed" && b.state.starts_with("failed:")) || (b.state == "completed" && a.state.starts_with("failed:")), "{a:?} {b:?}");
    assert_eq!(fs::read(lab.home.join("Downloads/bundle/dir/a")).unwrap(), b"abc");
    assert_eq!(fs::read(lab.stage().join("payload/dir/a")).unwrap(), b"abc");
    drop(daemon); lab.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_with_active_engine_drains_before_cleanup_and_preserves_unrelated() {
    let lab = Lab::new().await; fs::create_dir_all(lab.home.join("Downloads/unrelated")).unwrap();
    fs::write(lab.home.join("Downloads/unrelated/keep"), b"safe").unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl1:aeee4:name6:cancel12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[0; 20]); bytes.extend_from_slice(b"ee");
    let source = lab.root.join("no-peers.torrent"); fs::write(&source, bytes).unwrap();
    let daemon = Daemon::start(&lab.home, Some("after-barrier")); let id = daemon.add(&source);
    wait(|| daemon.task(&id).is_some_and(|task| task.state == "peers" && task.torrent.and_then(|t| t.stage_name).is_some()), "active engine");
    let stage = lab.stage();
    assert!(matches!(daemon.request(ClientRequest::Cancel { task_id: id.clone() }), DaemonEvent::TaskStateChanged { .. }));
    daemon.ready(); assert!(lab.home.join("hooks/late-write-rejected").exists());
    assert!(stage.exists(), "cleanup happened before barrier"); daemon.release();
    assert_eq!(daemon.terminal(&id).state, "canceled"); assert!(!stage.exists());
    assert_eq!(fs::read(lab.home.join("Downloads/unrelated/keep")).unwrap(), b"safe");
    assert!(!lab.home.join("Downloads/cancel").exists());
    drop(daemon); lab.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_staged_file_after_barrier_fails_and_retains_stage() {
    let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some("after-barrier")); let id = daemon.add(&lab.source);
    daemon.ready(); let stage = lab.stage(); fs::remove_file(stage.join("payload/empty")).unwrap(); daemon.release();
    let task = daemon.terminal(&id); assert!(task.state.starts_with("failed:"), "{task:?}");
    assert!(task.final_path.is_none()); assert!(!lab.home.join("Downloads/bundle").exists());
    assert!(stage.exists()); assert_eq!(fs::read(stage.join("payload/dir/a")).unwrap(), b"abc");
    drop(daemon); let restarted = Daemon::start(&lab.home, None); assert!(stage.exists());
    assert_ne!(restarted.task(&id).unwrap().state, "completed"); drop(restarted); lab.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_after_rename_then_tampering_never_claims_completion() {
    let lab = Lab::new().await; let daemon = Daemon::start(&lab.home, Some("after-rename")); let id = daemon.add(&lab.source);
    daemon.ready(); let stage = lab.stage(); let file = lab.home.join("Downloads/bundle/dir/a"); drop(daemon);
    fs::write(&file, b"abd").unwrap(); let restarted = Daemon::start(&lab.home, None);
    assert_ne!(restarted.task(&id).unwrap().state, "completed"); assert_eq!(fs::read(file).unwrap(), b"abd");
    assert!(stage.exists()); drop(restarted); lab.finish().await;
}

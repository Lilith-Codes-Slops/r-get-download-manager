#![cfg(all(target_os = "linux", feature = "torrent"))]

use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use librqbit::{AddTorrent, AddTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions};
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, os::unix::net::UnixStream,
    path::PathBuf, process::{Child, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread, time::{Duration, Instant}};

struct Fixture { root: PathBuf, child: Child }
impl Fixture {
    fn start(root: PathBuf) -> Self {
        std::fs::create_dir_all(root.join("run")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root).env("XDG_RUNTIME_DIR", root.join("run"))
            .env("XDG_STATE_HOME", root.join("state")).stdin(Stdio::null())
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let f = Self { root, child };
        let deadline = Instant::now() + Duration::from_secs(15);
        while UnixStream::connect(f.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon failed to start");
            thread::sleep(Duration::from_millis(20));
        }
        f
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn add(&self, source: &str) -> String {
        let response = self.request(ClientRequest::AddTask { url: source.into(), mirrors: vec![],
            connections: 1, extract: false, category: None, not_before: None });
        match response { DaemonEvent::TaskStateChanged { task_id, .. } => task_id, other => panic!("{other:?}") }
    }
    async fn completed(&self, source: &str) -> DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let DaemonEvent::DetailedQueue { tasks } = self.request(ClientRequest::GetDetailedQueue) {
                if let Some(task) = tasks.into_iter().find(|task| task.url == source) {
                    assert!(!task.state.starts_with("failed:"), "{task:?}");
                    if task.state == "completed" { return task; }
                }
            }
            assert!(Instant::now() < deadline, "torrent timed out: {source}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

fn tracker_reply(mut stream: TcpStream, peer_port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut data = [0; 4096]; let _ = stream.read(&mut data);
    let mut body = b"d8:intervali30e5:peers6:".to_vec();
    body.extend_from_slice(&[127, 0, 0, 1]); body.extend_from_slice(&peer_port.to_be_bytes()); body.push(b'e');
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(header.as_bytes()); let _ = stream.write_all(&body);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn torrent_file_and_magnet_download_from_local_tracker_without_clobbering() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-torrent-ipc-{}-{nonce}", std::process::id()));
    let seed_dir = root.join("seed"); let lab = root.join("lab");
    std::fs::create_dir_all(&seed_dir).unwrap();
    std::fs::write(seed_dir.join("fixture.bin"), b"abc").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let tracker_url = format!("http://{}/announce", listener.local_addr().unwrap());
    let hash: [u8; 20] = [0xa9,0x99,0x3e,0x36,0x47,0x06,0x81,0x6a,0xba,0x3e,0x25,0x71,0x78,0x50,0xc2,0x6c,0x9c,0xd0,0xd8,0x9d];
    let mut torrent = format!("d8:announce{}:{}4:infod6:lengthi3e4:name11:fixture.bin12:piece lengthi16384e6:pieces20:", tracker_url.len(), tracker_url).into_bytes();
    torrent.extend_from_slice(&hash); torrent.extend_from_slice(b"ee");
    let source_path = root.join("fixture.torrent"); std::fs::write(&source_path, &torrent).unwrap();
    let seed = Session::new_with_opts(seed_dir.clone(), SessionOptions {
        dht: None, disable_trackers: true, disable_local_service_discovery: true,
        listen: Some(ListenerOptions { mode: ListenerMode::TcpOnly,
            listen_addr: ([127, 0, 0, 1], 0).into(), ..Default::default() }),
        ..Default::default()
    }).await.unwrap();
    let peer_port = seed.listen_addr().unwrap().port();
    let handle = seed.add_torrent(AddTorrent::from_bytes(torrent), Some(AddTorrentOptions {
        overwrite: true, ..Default::default()
    })).await.unwrap().into_handle().unwrap();
    tokio::time::timeout(Duration::from_secs(15), handle.wait_until_initialized()).await.unwrap().unwrap();
    assert!(handle.stats().finished, "seed file was not verified");
    let quit = Arc::new(AtomicBool::new(false)); let done = quit.clone();
    let tracker = thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => tracker_reply(stream, peer_port),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
    });
    let f = Fixture::start(lab.clone());
    let file_source = source_path.to_str().unwrap(); let first_id = f.add(file_source);
    assert_ne!(first_id, file_source);
    let tracker_encoded = tracker_url.replace(':', "%3A").replace('/', "%2F");
    let magnet = format!("magnet:?xt=urn:btih:{}&tr={tracker_encoded}", handle.info_hash().as_string());
    assert_eq!(f.add(&magnet), first_id, "duplicate should reuse even while metadata is pending");
    let first = f.completed(&first_id).await;
    assert_eq!(first.torrent.as_ref().unwrap().source, file_source);
    assert!(!first.torrent.as_ref().unwrap().seed);
    let files = first.torrent.as_ref().unwrap().files.as_ref().expect("recorded files");
    assert_eq!(files.len(), 1); assert_eq!(files[0].path, "fixture.bin");
    assert_eq!(files[0].length, 3); assert_eq!(files[0].priority, 1);
    let metrics = first.torrent.as_ref().unwrap().metrics.as_ref().expect("torrent metrics");
    assert!(metrics.metadata_fetch_ms.is_some());
    assert!(metrics.time_to_first_piece_ms.is_some());
    assert!(metrics.first_piece_to_finished_ms.is_some());
    assert_eq!(first.filename.as_deref(), Some("fixture.bin"));
    assert_eq!(std::fs::read(lab.join("Downloads/fixture.bin")).unwrap(), b"abc");
    assert_eq!(f.add(&magnet), first_id, "duplicate magnet should reuse completed task");
    f.request(ClientRequest::SetLimit { task_id: None, kbps: 0 }); // Persist before the test kills the daemon.
    drop(f);
    let f = Fixture::start(lab.clone());
    assert_eq!(f.add(&magnet), first_id, "identity should persist across restart");
    let restored = f.completed(&first_id).await;
    assert_eq!(restored.torrent.unwrap().files.unwrap()[0].priority, 1);
    assert!(matches!(f.request(ClientRequest::RemoveTask { task_id: first_id.clone(), delete_file: false }), DaemonEvent::TaskStateChanged { .. }));
    assert_eq!(std::fs::read(lab.join("Downloads/fixture.bin")).unwrap(), b"abc");
    let second_id = f.add(&magnet);
    assert_ne!(second_id, first_id);
    let second = f.completed(&second_id).await;
    assert_eq!(second.filename.as_deref(), Some("fixture (1).bin"));
    assert_eq!(std::fs::read(lab.join("Downloads/fixture (1).bin")).unwrap(), b"abc");
    assert_eq!(std::fs::read(lab.join("Downloads/fixture.bin")).unwrap(), b"abc");
    assert!(matches!(f.request(ClientRequest::RemoveTask { task_id: second_id, delete_file: true }), DaemonEvent::TaskStateChanged { .. }));
    assert!(!lab.join("Downloads/fixture (1).bin").exists());
    assert_eq!(std::fs::read(lab.join("Downloads/fixture.bin")).unwrap(), b"abc");
    drop(f); quit.store(true, Ordering::Relaxed); tracker.join().unwrap();
    seed.stop().await;
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn malformed_metainfo_fails_without_touching_existing_downloads() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-torrent-bad-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(root.join("Downloads")).unwrap();
    std::fs::write(root.join("Downloads/sentinel"), b"keep").unwrap();
    let bad = root.join("invalid.torrent"); std::fs::write(&bad, b"not bencode").unwrap();
    let f = Fixture::start(root.clone());
    let response = f.request(ClientRequest::AddTask { url: bad.to_str().unwrap().into(), mirrors: vec![],
        connections: 1, extract: false, category: None, not_before: None });
    assert!(matches!(response, DaemonEvent::TaskFailed { .. }), "{response:?}");
    assert_eq!(std::fs::read(root.join("Downloads/sentinel")).unwrap(), b"keep");
    drop(f); let _ = std::fs::remove_dir_all(root);
}

#[test]
fn torrent_transition_subscription_is_opt_in() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-torrent-events-{}-{nonce}", std::process::id()));
    let f = Fixture::start(root.clone());
    let mut stream = UnixStream::connect(root.join("run/rget.sock")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    FrameEncoder::write_frame(&mut stream, &ClientRequest::SubscribeTorrentEvents).unwrap();
    let event: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
    assert_eq!(event, DaemonEvent::TorrentEventsReady);
    drop(stream); drop(f); let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_multifile_is_validated_before_allocating_a_stage() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-multi-preflight-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(root.join("Downloads")).unwrap();
    let source = root.join("bundle.torrent");
    let mut torrent = b"d4:infod5:filesld6:lengthi3e4:pathl2:..eee4:name6:bundle12:piece lengthi16384e6:pieces20:".to_vec();
    torrent.extend_from_slice(&[0; 20]); torrent.extend_from_slice(b"ee");
    std::fs::write(&source, torrent).unwrap();
    let f = Fixture::start(root.clone());
    let response = f.request(ClientRequest::AddTask { url: source.to_str().unwrap().into(), mirrors: vec![],
        connections: 1, extract: false, category: None, not_before: None });
    assert!(matches!(response, DaemonEvent::TaskFailed { error, .. } if error.contains("invalid multi-file metadata")));
    assert!(std::fs::read_dir(root.join("Downloads")).unwrap().next().is_none());
    drop(f); let _ = std::fs::remove_dir_all(root);
}
#[cfg(target_env = "gnu")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_multifile_publishes_and_removes_over_ipc() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-multi-ipc-{}-{nonce}", std::process::id()));
    let seed_root = root.join("seed"); let payload = seed_root.join("bundle"); let lab = root.join("lab");
    std::fs::create_dir_all(payload.join("dir")).unwrap();
    std::fs::write(payload.join("dir/a"), b"abc").unwrap(); std::fs::write(payload.join("empty"), b"").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap();
    let tracker_url = format!("http://{}/announce", listener.local_addr().unwrap());
    let mut torrent = format!("d8:announce{}:{}4:infod5:filesld6:lengthi3e4:pathl3:dir1:aee", tracker_url.len(), tracker_url).into_bytes();
    torrent.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyeee4:name6:bundle12:piece lengthi16384e6:pieces20:");
    torrent.extend_from_slice(&[0xa9,0x99,0x3e,0x36,0x47,0x06,0x81,0x6a,0xba,0x3e,0x25,0x71,0x78,0x50,0xc2,0x6c,0x9c,0xd0,0xd8,0x9d]);
    torrent.extend_from_slice(b"ee");
    let source = root.join("multi.torrent"); std::fs::write(&source, &torrent).unwrap();
    let seed = Session::new_with_opts(seed_root, SessionOptions {
        dht: None, disable_trackers: true, disable_local_service_discovery: true,
        listen: Some(ListenerOptions { mode: ListenerMode::TcpOnly, listen_addr: ([127,0,0,1], 0).into(), ..Default::default() }),
        ..Default::default()
    }).await.unwrap();
    let peer_port = seed.listen_addr().unwrap().port();
    let handle = seed.add_torrent(AddTorrent::from_bytes(torrent), Some(AddTorrentOptions {
        overwrite: true, output_folder: Some(payload.to_str().unwrap().into()), ..Default::default()
    })).await.unwrap().into_handle().unwrap();
    tokio::time::timeout(Duration::from_secs(15), handle.wait_until_initialized()).await.unwrap().unwrap();
    assert!(handle.stats().finished);
    let quit = Arc::new(AtomicBool::new(false)); let done = quit.clone();
    let tracker = thread::spawn(move || while !done.load(Ordering::Relaxed) {
        match listener.accept() { Ok((stream, _)) => tracker_reply(stream, peer_port),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)), Err(_) => break }
    });
    let f = Fixture::start(lab.clone());
    let id = f.add(source.to_str().unwrap()); let completed = f.completed(&id).await;
    let identity = completed.torrent.unwrap(); assert_eq!(identity.root_name.as_deref(), Some("bundle"));
    assert_eq!(identity.files.unwrap().len(), 2); assert_eq!(std::fs::read(lab.join("Downloads/bundle/dir/a")).unwrap(), b"abc");
    assert!(matches!(f.request(ClientRequest::RemoveTask { task_id: id, delete_file: true }), DaemonEvent::TaskStateChanged { state, .. } if state == "removed"));
    assert!(!lab.join("Downloads/bundle").exists());
    let id = f.add(source.to_str().unwrap()); f.completed(&id).await;
    assert!(matches!(f.request(ClientRequest::RemoveTask { task_id: id, delete_file: false }), DaemonEvent::TaskStateChanged { state, .. } if state == "removed"));
    assert_eq!(std::fs::read(lab.join("Downloads/bundle/dir/a")).unwrap(), b"abc");
    // A later download must fail closed instead of replacing the kept tree.
    let collision_id = f.add(source.to_str().unwrap()); let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(ClientRequest::GetDetailedQueue) {
            if let Some(task) = tasks.into_iter().find(|task| task.url == collision_id) {
                if task.state.starts_with("failed:") { assert!(task.state.contains("publication failed")); break; }
                assert_ne!(task.state, "completed");
            }
        }
        assert!(Instant::now() < deadline, "collision did not fail"); tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(std::fs::read(lab.join("Downloads/bundle/dir/a")).unwrap(), b"abc");
    drop(f); quit.store(true, Ordering::Relaxed); tracker.join().unwrap(); seed.stop().await;
    let _ = std::fs::remove_dir_all(root);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_multifile_cancel_drains_and_cleans_owned_stage() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-multi-cancel-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl1:aeee4:name6:bundle12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[0; 20]); bytes.extend_from_slice(b"ee");
    let source = root.join("cancel.torrent"); std::fs::write(&source, bytes).unwrap();
    let f = Fixture::start(root.clone()); let id = f.add(source.to_str().unwrap());
    let deadline = Instant::now() + Duration::from_secs(25);
    let stage = loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(ClientRequest::GetDetailedQueue) {
            if let Some(task) = tasks.into_iter().find(|task| task.url == id) {
                assert!(!task.state.starts_with("failed:"), "{task:?}");
                if let Some(name) = task.torrent.and_then(|torrent| torrent.stage_name) { break name; }
            }
        }
        assert!(Instant::now() < deadline, "stage not created"); tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(matches!(f.request(ClientRequest::Cancel { task_id: id.clone() }), DaemonEvent::TaskStateChanged { .. }));
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(ClientRequest::GetDetailedQueue) {
            if tasks.into_iter().any(|task| task.url == id && task.state == "canceled") { break; }
        }
        assert!(Instant::now() < deadline, "torrent did not cancel"); tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!root.join("Downloads/.rget-staging").join(stage).exists());
    assert!(!root.join("Downloads/bundle").exists()); drop(f); let _ = std::fs::remove_dir_all(root);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_multifile_recovers_only_recorded_stage() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-multi-crash-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(root.join("Downloads/unrelated")).unwrap();
    std::fs::write(root.join("Downloads/unrelated/keep"), b"untouched").unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl1:aeee4:name6:bundle12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[0; 20]); bytes.extend_from_slice(b"ee");
    let source = root.join("crash.torrent"); std::fs::write(&source, bytes).unwrap();
    let f = Fixture::start(root.clone()); let id = f.add(source.to_str().unwrap());
    let deadline = Instant::now() + Duration::from_secs(25);
    let stage = loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(ClientRequest::GetDetailedQueue) {
            if let Some(task) = tasks.into_iter().find(|task| task.url == id) {
                assert!(!task.state.starts_with("failed:"), "{task:?}");
                if let Some(name) = task.torrent.and_then(|torrent| torrent.stage_name) { break name; }
            }
        }
        assert!(Instant::now() < deadline, "stage not created"); tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let path = root.join("Downloads/.rget-staging").join(stage); let unknown = root.join("Downloads/.rget-staging/unrecorded");
    std::fs::create_dir(&unknown).unwrap(); std::fs::write(unknown.join("keep"), b"unknown").unwrap();
    assert!(path.join("payload").is_dir()); drop(f); // SIGKILL, without graceful shutdown.
    let f = Fixture::start(root.clone()); assert!(!path.exists(), "recorded interrupted stage was not cleaned");
    assert_eq!(std::fs::read(unknown.join("keep")).unwrap(), b"unknown");
    assert_eq!(std::fs::read(root.join("Downloads/unrelated/keep")).unwrap(), b"untouched");
    assert!(!root.join("Downloads/bundle").exists());
    if let DaemonEvent::DetailedQueue { tasks } = f.request(ClientRequest::GetDetailedQueue) {
        let task = tasks.into_iter().find(|task| task.url == id).expect("interrupted task missing");
        assert_ne!(task.state, "completed");
    } else { panic!("queue unavailable"); }
    drop(f); let _ = std::fs::remove_dir_all(root);
}

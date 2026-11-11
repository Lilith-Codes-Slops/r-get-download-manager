#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const TOTAL: u64 = 4 * 1024 * 1024;
struct Fixture {
    root: PathBuf,
    daemon: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rget-split-live-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        Self { root, daemon: None }
    }
    fn start(&mut self) {
        self.start_with_hook(None);
    }
    fn start_with_hook(&mut self, hook: Option<&str>) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_downloader-core"));
        command
            .env("HOME", &self.root)
            .env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env_remove("RGET_TEST_CHECKPOINT_HOOK")
            .env_remove("RGET_TEST_CHECKPOINT_MARKER")
            .env_remove("RGET_TEST_WRITE_BLOCK")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(hook) = hook {
            command
                .env("RGET_TEST_CHECKPOINT_HOOK", hook)
                .env("RGET_TEST_CHECKPOINT_MARKER", self.root.join("marker"));
            if hook == "write_block" {
                command.env("RGET_TEST_WRITE_BLOCK", self.root.join("marker"));
            }
        }
        self.daemon = Some(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(15);
        while UnixStream::connect(self.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() {
                panic!("daemon exited: {status}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn request(&self, req: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, &req).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn add(&self, url: &str, connections: u8) {
        let event = self.request(ClientRequest::AddTask {
            url: url.into(),
            mirrors: vec![],
            connections,
            extract: false,
            category: None,
            not_before: None,
        });
        assert!(
            matches!(event, DaemonEvent::TaskStateChanged { .. }),
            "{event:?}"
        );
    }
    fn task(&self, url: &str) -> DownloadInfo {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks
                .into_iter()
                .find(|t| t.url == url)
                .expect("task missing"),
            event => panic!("unexpected response: {event:?}"),
        }
    }
    fn wait(&self, url: &str, state: &str) -> DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let task = self.task(url);
            if task.state == state || (state == "failed:" && task.state.starts_with("failed:")) {
                return task;
            }
            assert!(!task.state.starts_with("failed:"), "{}", task.state);
            assert!(
                Instant::now() < deadline,
                "task did not reach {state}: {task:?}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn wait_checkpoint(&self, url: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let task = self.task(url);
            assert!(!task.state.starts_with("failed:"), "{}", task.state);
            let db = self.root.join("state/rget/queue.sqlite3");
            if let Ok(conn) = rusqlite::Connection::open(db) {
                let row = conn.query_row(
                    "SELECT info FROM rget_split_checkpoints_v2 WHERE url = ?1",
                    [url],
                    |r| r.get::<_, String>(0),
                );
                if let Ok(json) = row {
                    let record: serde_json::Value = serde_json::from_str(&json).unwrap();
                    let segments = record["segments"].as_array().unwrap();
                    if record["sequence"].as_u64().unwrap() >= 2
                        && segments
                            .iter()
                            .all(|s| s["downloaded"].as_u64().unwrap() > 0)
                    {
                        return record;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "split checkpoint not reached: {task:?}"
            );
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct HttpServer {
    url: String,
    release: Arc<AtomicBool>,
    // Work on it later; see state.md: these handles are only cloned into the server thread.
    _changed: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    _requests: Arc<Mutex<Vec<(u64, u64, bool)>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl HttpServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let release = Arc::new(AtomicBool::new(false));
        let changed = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (rel, chg, run, log) = (
            release.clone(),
            changed.clone(),
            running.clone(),
            requests.clone(),
        );
        let thread = std::thread::spawn(move || {
            while run.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (rel, chg, run, log) =
                            (rel.clone(), chg.clone(), run.clone(), log.clone());
                        std::thread::spawn(move || serve(stream, rel, chg, run, log));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            release,
            _changed: changed,
            running,
            _requests: requests,
            thread: Some(thread),
        }
    }
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.release.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    release: Arc<AtomicBool>,
    changed: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<(u64, u64, bool)>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let mut request = Vec::new();
    let mut b = [0; 1];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
        if !matches!(stream.read(&mut b), Ok(1)) {
            return;
        }
        request.push(b[0]);
    }
    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
    let range = request
        .lines()
        .find_map(|line| line.strip_prefix("range: bytes="));
    let etag = if changed.load(Ordering::SeqCst) {
        "v2"
    } else {
        "v1"
    };
    let Some(range) = range else {
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"{etag}\"\r\nContent-Disposition: attachment; filename=split.bin\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(header.as_bytes());
        while running.load(Ordering::SeqCst) && !release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        return;
    };
    let Some((start, end)) = range.split_once('-') else {
        return;
    };
    let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else {
        return;
    };
    if start > end || end >= TOTAL {
        return;
    }
    log.lock()
        .unwrap()
        .push((start, end, release.load(Ordering::SeqCst)));
    let length = end - start + 1;
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {length}\r\nContent-Range: bytes {start}-{end}/{TOTAL}\r\nETag: \"{etag}\"\r\nConnection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    let mut pos = start;
    while pos <= end && running.load(Ordering::SeqCst) {
        let count = (end - pos + 1).min(64 * 1024);
        let bytes: Vec<u8> = (pos..pos + count).map(|n| (n % 251) as u8).collect();
        if stream.write_all(&bytes).is_err() {
            return;
        }
        pos += count;
        if start != 0 || end != 0 {
            while running.load(Ordering::SeqCst) && !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn split_connections_are_validated_acknowledged_and_persisted() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    for count in [0, 33] {
        let response = f.request(ClientRequest::SetConnections {
            task_id: server.url.clone(),
            count,
        });
        assert!(
            matches!(response, DaemonEvent::TaskFailed { .. }),
            "{response:?}"
        );
    }
    let response = f.request(ClientRequest::SetConnections {
        task_id: server.url.clone(),
        count: 1,
    });
    assert!(
        matches!(
            response,
            DaemonEvent::ConnectionsChanged { requested: 1, .. }
        ),
        "{response:?}"
    );
    assert_eq!(f.task(&server.url).requested_connections, Some(1));
    let response = f.request(ClientRequest::Pause {
        task_id: server.url.clone(),
    });
    assert!(
        matches!(response, DaemonEvent::TaskStateChanged { .. }),
        "{response:?}"
    );
    f.wait(&server.url, "paused");
    let response = f.request(ClientRequest::SetConnections {
        task_id: server.url.clone(),
        count: 3,
    });
    assert!(
        matches!(
            response,
            DaemonEvent::ConnectionsChanged { requested: 3, .. }
        ),
        "{response:?}"
    );
    f.stop();
    f.start();
    let task = f.wait(&server.url, "paused");
    assert_eq!(task.requested_connections, Some(3));
    assert!(task.live_connections_supported);
    let response = f.request(ClientRequest::Resume {
        task_id: server.url.clone(),
    });
    assert!(
        matches!(response, DaemonEvent::TaskStateChanged { .. }),
        "{response:?}"
    );
    server.release.store(true, Ordering::SeqCst);
    f.wait(&server.url, "completed");
    let bytes = std::fs::read(f.root.join("Downloads/split.bin")).unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, b)| *b == (i as u64 % 251) as u8));
}

#[test]
fn single_stream_resize_is_explicitly_rejected() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 1);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let task = f.task(&server.url);
        if task.active_connections == Some(1) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "single stream did not start: {task:?}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    let response = f.request(ClientRequest::SetConnections {
        task_id: server.url.clone(),
        count: 4,
    });
    assert!(
        matches!(response, DaemonEvent::TaskFailed { .. }),
        "{response:?}"
    );
    assert_eq!(f.task(&server.url).requested_connections, Some(1));
}

#[test]
fn segment_subscription_streams_progress_without_queue_requests() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    let mut stream = UnixStream::connect(f.root.join("run/rget.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    FrameEncoder::write_frame(&mut stream, &ClientRequest::SubscribeSegments).unwrap();
    let reset: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
    assert!(matches!(reset, DaemonEvent::SegmentsReset));
    let initial: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
    let (generation, mut segments) = match initial {
        DaemonEvent::SegmentsSnapshot {
            task_id,
            generation,
            segments,
        } => {
            assert_eq!(task_id, server.url);
            (generation, segments)
        }
        other => panic!("missing initial layout: {other:?}"),
    };
    let before: u64 = segments.iter().map(|segment| segment.downloaded).sum();
    assert_eq!(segments.len(), 2);
    server.release.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut advanced = false;
    loop {
        let event: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
        match event {
            DaemonEvent::SegmentsSnapshot {
                task_id,
                segments: new,
                ..
            } if task_id == server.url => segments = new,
            DaemonEvent::SegmentUpdate {
                task_id,
                segment_id,
                downloaded,
                generation: Some(gen),
                ..
            } if task_id == server.url && gen == generation => {
                if let Some(segment) = segments
                    .iter_mut()
                    .find(|s| s.segment_id.to_string() == segment_id)
                {
                    segment.downloaded = downloaded;
                }
            }
            DaemonEvent::SegmentsRemoved { task_id, .. } if task_id == server.url => break,
            _ => {}
        }
        advanced |= segments
            .iter()
            .map(|segment| segment.downloaded)
            .sum::<u64>()
            > before;
        assert!(Instant::now() < deadline, "segment stream stalled");
    }
    assert!(advanced, "no live progress event arrived");
    f.wait(&server.url, "completed");
}

#[test]
fn policy_subscription_reports_current_values_and_refreshes_without_queue_polling() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    let mut stream = UnixStream::connect(f.root.join("run/rget.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    FrameEncoder::write_frame(&mut stream, &ClientRequest::SubscribeSegmentPolicies).unwrap();
    let reset: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
    assert!(matches!(reset, DaemonEvent::SegmentPoliciesReset));
    let initial: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
    let (generation, mut previous) = match initial {
        DaemonEvent::SegmentPolicyUpdate {
            task_id,
            generation,
            target_size,
            steal_floor,
            reason,
        } => {
            assert_eq!(task_id, server.url);
            (generation, (target_size, steal_floor, reason))
        }
        other => panic!("missing initial policy: {other:?}"),
    };
    assert!((1024 * 1024..=128 * 1024 * 1024).contains(&previous.0));
    assert!(previous.1 >= 1024 * 1024);
    let deadline = Instant::now() + Duration::from_secs(18);
    loop {
        let event: DaemonEvent = FrameDecoder::read_frame(&mut stream).unwrap();
        if let DaemonEvent::SegmentPolicyUpdate {
            task_id,
            generation: current,
            target_size,
            steal_floor,
            reason,
        } = event
        {
            assert_eq!(task_id, server.url);
            assert_eq!(current, generation);
            let current = (target_size, steal_floor, reason);
            if current == previous {
                break;
            }
            previous = current;
        }
        assert!(
            Instant::now() < deadline,
            "periodic policy refresh not received"
        );
    }
    server.release.store(true, Ordering::SeqCst);
    f.wait(&server.url, "completed");
}

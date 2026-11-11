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
    changed: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<(u64, u64, bool)>>>,
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
            changed,
            running,
            requests,
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
fn fixed_splits_survive_daemon_kill_and_resume_verified_ranges() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    let checkpoint = f.wait_checkpoint(&server.url);
    let telemetry = f.task(&server.url);
    assert_eq!(telemetry.requested_connections, Some(2));
    assert!(telemetry
        .active_connections
        .is_some_and(|count| (1..=2).contains(&count)));
    assert_eq!(telemetry.segment_count, Some(2));
    f.stop();
    server.release.store(true, Ordering::SeqCst);
    f.start();
    f.wait(&server.url, "completed");
    let file = std::fs::read(f.root.join("Downloads/split.bin")).unwrap();
    assert_eq!(file.len() as u64, TOTAL);
    assert!(file
        .iter()
        .enumerate()
        .all(|(i, byte)| *byte == (i as u64 % 251) as u8));
    let requests = server.requests.lock().unwrap();
    for segment in checkpoint["segments"].as_array().unwrap() {
        let min = segment["start"].as_u64().unwrap() + segment["downloaded"].as_u64().unwrap();
        let end = segment["end"].as_u64().unwrap();
        assert!(
            requests.iter().any(|&(start, last, after)| after
                && start >= min
                && start < end
                && last + 1 == end),
            "missing resumed range: {requests:?}"
        );
    }
}

#[test]
fn changed_etag_after_restart_fails_without_publishing() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    let partial = f.task(&server.url).partial_name.unwrap();
    f.stop();
    server.changed.store(true, Ordering::SeqCst);
    server.release.store(true, Ordering::SeqCst);
    f.start();
    let failed = f.wait(&server.url, "failed:");
    assert!(failed.state.contains("validator"), "{}", failed.state);
    assert!(f.root.join("Downloads").join(partial).exists());
    assert!(!f.root.join("Downloads/split.bin").exists());
}

#[test]
fn cancel_waits_for_split_workers_before_cleanup() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    let partial = f.task(&server.url).partial_name.unwrap();
    assert!(matches!(
        f.request(ClientRequest::Cancel {
            task_id: server.url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    f.wait(&server.url, "canceled");
    assert!(!f.root.join("Downloads").join(&partial).exists());
    assert!(!f
        .root
        .join("Downloads")
        .join(format!("{partial}.meta"))
        .exists());
    assert!(!f.root.join("Downloads/split.bin").exists());
}

#[test]
fn non_range_server_falls_back_to_single_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Disposition: attachment; filename=fallback.bin\r\nConnection: close\r\n\r\nabc").unwrap();
    });
    let mut f = Fixture::new();
    f.start();
    f.add(&url, 8);
    let task = f.wait(&url, "completed");
    assert!(task
        .connection_note
        .as_deref()
        .is_some_and(|note| note.contains("one stream") && note.contains("byte-range")));
    assert_eq!(
        std::fs::read(f.root.join("Downloads/fallback.bin")).unwrap(),
        b"abc"
    );
    server.join().unwrap();
}

#[test]
fn server_ignoring_range_falls_back_without_claiming_resume_support() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            workers.push(std::thread::spawn(move || {
                let mut request = [0u8; 4096]; let _ = stream.read(&mut request);
                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=fallback-large.bin\r\nConnection: close\r\n\r\n", 2 * 1024 * 1024);
                if stream.write_all(header.as_bytes()).is_ok() { let _ = stream.write_all(&vec![42; 2 * 1024 * 1024]); }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let mut f = Fixture::new();
    f.start();
    f.add(&url, 2);
    let task = f.wait(&url, "completed");
    assert!(!task.resume_supported);
    assert!(task.connection_note.as_deref().is_some_and(|note| note
        .contains("ignored or rejected")
        && note.contains("resume is unavailable")));
    assert_eq!(
        std::fs::read(f.root.join("Downloads/fallback-large.bin")).unwrap(),
        vec![42; 2 * 1024 * 1024]
    );
    server.join().unwrap();
}

#[test]
fn rejects_connection_counts_outside_the_supported_bounds() {
    let mut f = Fixture::new();
    f.start();
    for connections in [0, 33] {
        let event = f.request(ClientRequest::AddTask {
            url: "https://example.test/file".into(),
            mirrors: vec![],
            connections,
            extract: false,
            category: None,
            not_before: None,
        });
        assert!(matches!(event, DaemonEvent::TaskFailed { .. }), "{event:?}");
    }
}

#[test]
fn paused_split_survives_restart_and_resumes_verified_ranges() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    assert!(f.task(&server.url).resume_supported);
    assert!(matches!(
        f.request(ClientRequest::Pause {
            task_id: server.url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let paused = f.wait(&server.url, "paused");
    assert_eq!(paused.active_connections, Some(0));
    let partial = f
        .root
        .join("Downloads")
        .join(paused.partial_name.as_ref().unwrap());
    let before = std::fs::read(&partial).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        std::fs::read(&partial).unwrap(),
        before,
        "bytes changed after pause barrier"
    );
    f.stop();
    server.release.store(true, Ordering::SeqCst);
    f.start();
    let restored = f.wait(&server.url, "paused");
    assert_eq!(restored.downloaded, paused.downloaded);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        std::fs::read(&partial).unwrap(),
        before,
        "paused recovery wrote bytes"
    );
    assert!(matches!(
        f.request(ClientRequest::Resume {
            task_id: server.url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    f.wait(&server.url, "completed");
    let file = std::fs::read(f.root.join("Downloads/split.bin")).unwrap();
    assert_eq!(file.len() as u64, TOTAL);
    assert!(file
        .iter()
        .enumerate()
        .all(|(i, byte)| *byte == (i as u64 % 251) as u8));
}

#[test]
fn cancel_paused_split_joins_before_removing_files() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start();
    f.add(&server.url, 2);
    f.wait_checkpoint(&server.url);
    assert!(matches!(
        f.request(ClientRequest::Pause {
            task_id: server.url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let paused = f.wait(&server.url, "paused");
    let partial = paused.partial_name.unwrap();
    assert!(matches!(
        f.request(ClientRequest::Cancel {
            task_id: server.url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    f.wait(&server.url, "canceled");
    assert!(!f.root.join("Downloads").join(&partial).exists());
    assert!(!f
        .root
        .join("Downloads")
        .join(format!("{partial}.meta"))
        .exists());
    assert!(!f.root.join("Downloads/split.bin").exists());
}

#[cfg(feature = "checkpoint-test-hooks")]
#[path = "support/split_checkpoint_crash_cases.rs"]
mod checkpoint_crash_cases;

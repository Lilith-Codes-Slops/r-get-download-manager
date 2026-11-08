#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder};
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, os::unix::net::UnixStream,
    path::PathBuf, process::{Child, Command, Stdio}, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    time::{Duration, Instant}};
// Large enough for stealing even in the highest latency bucket.
const TOTAL: u64 = 32 * 1024 * 1024;

struct Daemon { root: PathBuf, process: Option<Child> }
impl Daemon {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-steal-crash-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        Self { root, process: None }
    }
    fn start(&mut self) {
        self.process = Some(Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &self.root).env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let end = Instant::now() + Duration::from_secs(15);
        while UnixStream::connect(self.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < end, "daemon did not start");
            if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() { panic!("daemon exited: {status}"); }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self) { if let Some(mut child) = self.process.take() { let _ = child.kill(); let _ = child.wait(); } }
    fn request(&self, req: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(25))).unwrap();
        FrameEncoder::write_frame(&mut stream, &req).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn state(&self, id: &str) -> String {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks.into_iter().find(|task| task.url == id).unwrap().state,
            other => panic!("unexpected queue: {other:?}"),
        }
    }
    fn wait_stolen_checkpoint(&self, id: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = self.state(id);
            assert!(!state.starts_with("failed:"), "{state}");
            let db = self.root.join("state/rget/queue.sqlite3");
            if let Ok(conn) = rusqlite::Connection::open(db) {
                if let Ok(json) = conn.query_row("SELECT info FROM rget_split_checkpoints_v2 WHERE url = ?1", [id], |row| row.get::<_, String>(0)) {
                    let record: serde_json::Value = serde_json::from_str(&json).unwrap();
                    if record["segments"].as_array().unwrap().len() >= 3 { return record; }
                }
            }
            assert!(Instant::now() < deadline, "no checkpoint after a steal; state: {state}");
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    fn wait_complete(&self, id: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = self.state(id);
            if state == "completed" { return; }
            assert!(!state.starts_with("failed:"), "{state}");
            assert!(Instant::now() < deadline, "download did not finish: {state}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
impl Drop for Daemon { fn drop(&mut self) { self.stop(); let _ = std::fs::remove_dir_all(&self.root); } }

struct Server {
    url: String, release: Arc<AtomicBool>, running: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<(u64, u64, bool)>>>, thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let release = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let started = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (rel, run, slow, log) = (release.clone(), running.clone(), started.clone(), requests.clone());
        let thread = std::thread::spawn(move || {
            while run.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (rel, run, slow, log) = (rel.clone(), run.clone(), slow.clone(), log.clone());
                        std::thread::spawn(move || serve(stream, rel, run, slow, log));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        Self { url, release, running, requests, thread: Some(thread) }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst); self.release.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

fn send_body(stream: &mut TcpStream, mut start: u64, end: u64) -> bool {
    while start < end {
        let next = (start + 64 * 1024).min(end);
        let bytes: Vec<u8> = (start..next).map(|n| (n % 251) as u8).collect();
        if stream.write_all(&bytes).is_err() { return false; }
        start = next;
    }
    true
}

fn serve(mut stream: TcpStream, release: Arc<AtomicBool>, running: Arc<AtomicBool>, slow_started: Arc<AtomicBool>, log: Arc<Mutex<Vec<(u64, u64, bool)>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let mut req = Vec::new(); let mut byte = [0u8; 1];
    while !req.ends_with(b"\r\n\r\n") && req.len() < 16_384 {
        if !matches!(stream.read(&mut byte), Ok(1)) { return; }
        req.push(byte[0]);
    }
    let req = String::from_utf8_lossy(&req).to_ascii_lowercase();
    let range = req.lines().find_map(|line| line.strip_prefix("range: bytes="));
    let Some(range) = range else {
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=stolen.bin\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(header.as_bytes());
        while running.load(Ordering::SeqCst) && !release.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
        return;
    };
    let Some((start, end)) = range.split_once('-') else { return };
    let (Ok(start), Ok(last)) = (start.parse::<u64>(), end.parse::<u64>()) else { return };
    if start > last || last >= TOTAL { return; }
    let end = last + 1;
    let after_restart = release.load(Ordering::SeqCst);
    log.lock().unwrap().push((start, end, after_restart));
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{last}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", end - start);
    if stream.write_all(header.as_bytes()).is_err() { return; }
    if start == 0 && end == 1 { let _ = send_body(&mut stream, start, end); return; }
    if !after_restart && start == TOTAL / 2 && end == TOTAL / 2 + 8 * 1024 * 1024 { slow_started.store(true, Ordering::SeqCst); }
    if !after_restart && start < TOTAL / 2 && end <= TOTAL / 2 {
        while running.load(Ordering::SeqCst) && !slow_started.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
        let _ = send_body(&mut stream, start, end);
    } else if !after_restart {
        let first = (start + 64 * 1024).min(end);
        if !send_body(&mut stream, start, first) { return; }
        while running.load(Ordering::SeqCst) && !release.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
        let _ = send_body(&mut stream, first, end);
    } else { let _ = send_body(&mut stream, start, end); }
}

#[test]
fn daemon_recovers_after_work_stealing_changes_the_checkpoint_map() {
    let server = Server::new();
    let mut daemon = Daemon::new(); daemon.start();
    let event = daemon.request(ClientRequest::AddTask { url: server.url.clone(), mirrors: vec![], connections: 2, extract: false, category: None, not_before: None });
    assert!(matches!(event, DaemonEvent::TaskStateChanged { .. }), "{event:?}");
    let checkpoint = daemon.wait_stolen_checkpoint(&server.url);
    assert!(checkpoint["segments"].as_array().unwrap().len() >= 3);
    daemon.stop();
    server.release.store(true, Ordering::SeqCst);
    daemon.start(); daemon.wait_complete(&server.url);
    let bytes = std::fs::read(daemon.root.join("Downloads/stolen.bin")).unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes.iter().enumerate().all(|(index, byte)| *byte == (index as u64 % 251) as u8));
    let requests = server.requests.lock().unwrap();
    assert!(requests.iter().any(|&(start, _, after)| start > TOTAL / 2 && !after), "no steal before restart: {requests:?}");
    assert!(requests.iter().any(|&(start, _, after)| start > TOTAL / 2 && after), "no resumed stolen range: {requests:?}");
}

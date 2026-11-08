#![cfg(target_os = "linux")]
use downloader_ipc::{ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder};
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, os::unix::net::UnixStream, path::PathBuf,
    process::{Child, Command, Stdio}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

const MIB: u64 = 1024 * 1024;
const TOTAL: u64 = 64 * MIB;
struct Daemon { root: PathBuf, child: Option<Child> }
impl Daemon {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-host-ipc-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap(); Self { root, child: None }
    }
    fn start(&mut self) {
        self.child = Some(Command::new(env!("CARGO_BIN_EXE_downloader-core")).env("HOME", &self.root)
            .env("XDG_RUNTIME_DIR", self.root.join("run")).env("XDG_STATE_HOME", self.root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(self.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon startup timed out");
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() { panic!("daemon exited: {status}"); }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self) { if let Some(mut child) = self.child.take() { let _ = child.kill(); let _ = child.wait(); } }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap(); FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn add(&self, url: &str) {
        assert!(matches!(self.request(ClientRequest::AddTask { url: url.into(), mirrors: vec![], connections: 2, extract: false, category: None, not_before: None }), DaemonEvent::TaskStateChanged { .. }));
    }
    fn wait_canceled(&self, url: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let DaemonEvent::DetailedQueue { tasks } = self.request(ClientRequest::GetDetailedQueue) {
                if tasks.iter().any(|task| task.url == url && task.state == "canceled") { return; }
            }
            assert!(Instant::now() < deadline, "cancel timed out"); std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Daemon { fn drop(&mut self) { self.stop(); let _ = std::fs::remove_dir_all(&self.root); } }

type Requests = Arc<Mutex<Vec<(String, u64)>>>;
fn serve(mut stream: TcpStream, running: Arc<AtomicBool>, requests: Requests) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut bytes = Vec::new(); let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
        if !matches!(stream.read(&mut byte), Ok(1)) { return; } bytes.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    let path = text.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("");
    let range = text.lines().find_map(|line| line.strip_prefix("range: bytes="));
    let Some(range) = range else {
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(header.as_bytes()); return;
    };
    let Some((start, end)) = range.split_once('-') else { return };
    let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else { return };
    if start > end || end >= TOTAL { return; }
    let len = end - start + 1;
    if len > 1 { requests.lock().unwrap().push((path.to_owned(), len)); }
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {len}\r\nContent-Range: bytes {start}-{end}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n");
    if stream.write_all(header.as_bytes()).is_err() { return; }
    let _ = stream.write_all(&vec![7u8; len.min(64 * 1024) as usize]);
    if len > 1 { while running.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(20)); } }
}

fn wait_range(requests: &Requests, path: &str, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if requests.lock().unwrap().iter().any(|(name, length)| name == path && *length == expected) { return; }
        assert!(Instant::now() < deadline, "no {expected}-byte request for {path}: {:?}", requests.lock().unwrap());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn split_tasks_load_saved_host_target_after_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap(); let origin = format!("http://{address}");
    let requests: Requests = Arc::new(Mutex::new(Vec::new())); let running = Arc::new(AtomicBool::new(true));
    let (run, log) = (running.clone(), requests.clone());
    let server = std::thread::spawn(move || {
        while run.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => { let (run, log) = (run.clone(), log.clone()); std::thread::spawn(move || serve(stream, run, log)); }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    });
    let mut daemon = Daemon::new(); daemon.start();
    let first = format!("{origin}/first"); daemon.add(&first); wait_range(&requests, "/first", 8 * MIB);
    daemon.request(ClientRequest::Cancel { task_id: first.clone() }); daemon.wait_canceled(&first);
    daemon.stop();
    let db = rusqlite::Connection::open(daemon.root.join("state/rget/queue.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE IF NOT EXISTS rget_host_sizing_v1 (origin TEXT PRIMARY KEY NOT NULL, target INTEGER NOT NULL, bucket INTEGER NOT NULL, observed_ms INTEGER NOT NULL)").unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    db.execute("INSERT OR REPLACE INTO rget_host_sizing_v1(origin,target,bucket,observed_ms) VALUES (?1, ?2, 0, ?3)", rusqlite::params![origin, (12*MIB) as i64, now]).unwrap();
    drop(db); daemon.start();
    let second = format!("http://{address}/second"); daemon.add(&second); wait_range(&requests, "/second", 12 * MIB);
    daemon.request(ClientRequest::Cancel { task_id: second.clone() }); daemon.wait_canceled(&second);
    running.store(false, Ordering::SeqCst); server.join().unwrap();
}

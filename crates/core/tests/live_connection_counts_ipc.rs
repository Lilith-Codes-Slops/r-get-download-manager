#![cfg(target_os = "linux")]
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, os::unix::net::UnixStream, path::PathBuf,
    process::{Child, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}, time::{Duration, Instant}};

const TOTAL: u64 = 16 * 1024 * 1024;
struct Daemon { root: PathBuf, process: Child }
impl Daemon {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-counts-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        let mut process = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root).env("XDG_RUNTIME_DIR", root.join("run")).env("XDG_STATE_HOME", root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while UnixStream::connect(root.join("run/rget.sock")).is_err() {
            assert!(process.try_wait().unwrap().is_none(), "daemon exited");
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        Self { root, process }
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn task(&self, url: &str) -> DownloadInfo {
        let DaemonEvent::DetailedQueue { tasks } = self.request(ClientRequest::GetDetailedQueue) else { panic!("missing queue") };
        tasks.into_iter().find(|task| task.url == url).unwrap()
    }
    fn wait(&self, url: &str, predicate: impl Fn(&DownloadInfo) -> bool) -> DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let task = self.task(url);
            assert!(!task.state.starts_with("failed:"), "{}", task.state);
            if predicate(&task) { return task; }
            assert!(Instant::now() < deadline, "timeout: {task:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn add(&self, url: &str) {
        assert!(matches!(self.request(ClientRequest::AddTask { url: url.into(), mirrors: vec![], connections: 2, extract: false, category: None, not_before: None }), DaemonEvent::TaskStateChanged { .. }));
    }
    fn set(&self, url: &str, count: u8) {
        assert!(matches!(self.request(ClientRequest::SetConnections { task_id: url.into(), count }), DaemonEvent::ConnectionsChanged { requested, .. } if requested == count));
    }
}
impl Drop for Daemon { fn drop(&mut self) { let _ = self.process.kill(); let _ = self.process.wait(); let _ = std::fs::remove_dir_all(&self.root); } }

struct Server {
    url: String, high: Arc<AtomicBool>, release: Arc<AtomicBool>, running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(reject_extra: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap(); listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let high = Arc::new(AtomicBool::new(false)); let release = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true)); let accepted = Arc::new(AtomicUsize::new(0));
        let (h, r, run) = (high.clone(), release.clone(), running.clone());
        let thread = std::thread::spawn(move || {
            while run.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (h, r, run, accepted) = (h.clone(), r.clone(), run.clone(), accepted.clone());
                        std::thread::spawn(move || serve(stream, h, r, run, accepted, reject_extra));
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        Self { url, high, release, running, thread: Some(thread) }
    }
}
impl Drop for Server {
    fn drop(&mut self) { self.running.store(false, Ordering::SeqCst); self.release.store(true, Ordering::SeqCst); if let Some(thread) = self.thread.take() { let _ = thread.join(); } }
}

fn serve(mut stream: TcpStream, high: Arc<AtomicBool>, release: Arc<AtomicBool>, running: Arc<AtomicBool>, accepted: Arc<AtomicUsize>, reject_extra: bool) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let mut req = Vec::new(); let mut byte = [0u8; 1];
    while !req.ends_with(b"\r\n\r\n") && req.len() < 16_384 {
        if !matches!(stream.read(&mut byte), Ok(1)) { return; } req.push(byte[0]);
    }
    let req = String::from_utf8_lossy(&req).to_ascii_lowercase();
    let Some(range) = req.lines().find_map(|line| line.strip_prefix("range: bytes=")) else {
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=counts.bin\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(header.as_bytes()); return;
    };
    let Some((start, last)) = range.split_once('-') else { return };
    let (Ok(start), Ok(last)) = (start.parse::<u64>(), last.parse::<u64>()) else { return };
    if start > last || last >= TOTAL { return; }
    let probe = start == 0 && last == 0;
    if !probe && reject_extra && accepted.fetch_add(1, Ordering::SeqCst) > 0 && !release.load(Ordering::SeqCst) {
        let _ = stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); return;
    }
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{last}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", last - start + 1);
    if stream.write_all(header.as_bytes()).is_err() { return; }
    let mut offset = start;
    while offset <= last && running.load(Ordering::SeqCst) {
        let count = (last + 1 - offset).min(64 * 1024);
        let bytes: Vec<u8> = (offset..offset + count).map(|n| (n % 251) as u8).collect();
        if stream.write_all(&bytes).is_err() { return; }
        offset += count;
        while !probe && running.load(Ordering::SeqCst) && !release.load(Ordering::SeqCst)
            && !(start == TOTAL / 2 && high.load(Ordering::SeqCst)) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn resize_changes_requested_and_owned_worker_counts_mid_download() {
    let server = Server::new(false); let daemon = Daemon::new(); daemon.add(&server.url);
    daemon.wait(&server.url, |task| task.state == "downloading" && task.active_connections == Some(2));
    daemon.set(&server.url, 1);
    assert_eq!(daemon.task(&server.url).requested_connections, Some(1));
    // The runner forwards the requested limit on its 250 ms control tick.
    // Keep both responses blocked long enough for the drain request to arrive.
    std::thread::sleep(Duration::from_millis(1500));
    // Workers drain their current ranges; allow only the second range to finish.
    server.high.store(true, Ordering::SeqCst);
    daemon.wait(&server.url, |task| task.state == "downloading" && task.requested_connections == Some(1) && task.active_connections == Some(1));
    daemon.set(&server.url, 2);
    let task = daemon.wait(&server.url, |task| task.state == "downloading" && task.requested_connections == Some(2) && task.active_connections == Some(2));
    assert!(task.downloaded < TOTAL);
    server.release.store(true, Ordering::SeqCst);
    daemon.wait(&server.url, |task| task.state == "completed");
    let bytes = std::fs::read(daemon.root.join("Downloads/counts.bin")).unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes.iter().enumerate().all(|(i, byte)| *byte == (i as u64 % 251) as u8));
}

#[test]
fn rejected_extra_range_is_visible_with_lower_actual_count() {
    let server = Server::new(true); let daemon = Daemon::new(); daemon.add(&server.url);
    let task = daemon.wait(&server.url, |task| task.state == "downloading" && task.active_connections == Some(1)
        && task.connection_note.as_deref().is_some_and(|note| note.contains("HTTP 429")));
    assert_eq!(task.requested_connections, Some(2));
    server.release.store(true, Ordering::SeqCst);
    daemon.wait(&server.url, |task| task.state == "completed");
}

#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Fixture { root: PathBuf, daemon: Option<Child> }
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-ipc-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        Self { root, daemon: None }
    }
    fn socket(&self) -> PathBuf { self.root.join("run/rget.sock") }
    fn start(&mut self) {
        self.daemon = Some(Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &self.root).env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(self.socket()).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() { panic!("daemon exited: {status}"); }
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() { let _ = daemon.kill(); let _ = daemon.wait(); }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { self.stop(); let _ = std::fs::remove_dir_all(&self.root); }
}

fn request(socket: &Path, req: &ClientRequest) -> DaemonEvent {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    FrameEncoder::write_frame(&mut stream, req).unwrap();
    FrameDecoder::read_frame(&mut stream).unwrap()
}

#[test]
fn completed_task_and_limits_survive_daemon_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = stream.read(&mut buffer).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Disposition: attachment; filename=smoke.bin\r\nConnection: close\r\n\r\nabc").unwrap();
    });
    let mut f = Fixture::new();
    f.start();
    let response = request(&f.socket(), &ClientRequest::AddTask {
        url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None,
    });
    assert!(matches!(response, DaemonEvent::TaskStateChanged { .. }), "{response:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let DaemonEvent::DetailedQueue { tasks } = request(&f.socket(), &ClientRequest::GetDetailedQueue) {
            if tasks.iter().any(|task| task.url == url && task.state == "completed") { break; }
            assert!(!tasks.iter().any(|task| task.state.starts_with("failed:")), "{tasks:?}");
        }
        assert!(Instant::now() < deadline, "download did not finish");
        thread::sleep(Duration::from_millis(20));
    }
    server.join().unwrap();
    let verify = ClientRequest::VerifyChecksum {
        task_id: url.clone(), algorithm: "sha256".into(),
        expected: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
    };
    assert!(matches!(request(&f.socket(), &verify), DaemonEvent::ChecksumVerified { .. }));
    assert!(matches!(request(&f.socket(), &ClientRequest::SetLimit { task_id: None, kbps: 128 }), DaemonEvent::TaskStateChanged { .. }));
    f.stop();
    f.start();
    let restored = request(&f.socket(), &ClientRequest::GetDetailedQueue);
    assert!(matches!(restored, DaemonEvent::DetailedQueue { tasks } if tasks.iter().any(|task| task.url == url && task.state == "completed")));
    let db = rusqlite::Connection::open(f.root.join("state/rget/queue.sqlite3")).unwrap();
    let rate: String = db.query_row("SELECT kbps FROM rget_rates_v1 WHERE id = ''", [], |row| row.get(0)).unwrap();
    assert_eq!(rate, "128");
    assert_eq!(std::fs::read(f.root.join("Downloads/smoke.bin")).unwrap(), b"abc");
}

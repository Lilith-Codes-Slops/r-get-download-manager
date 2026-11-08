#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture { root: PathBuf, daemon: Option<Child> }
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-recovery-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        Self { root, daemon: None }
    }
    fn start(&mut self) {
        self.daemon = Some(Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &self.root).env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(self.root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() { panic!("daemon exited: {status}"); }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self) { if let Some(mut child) = self.daemon.take() { let _ = child.kill(); let _ = child.wait(); } }
    fn request(&self, req: &ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        FrameEncoder::write_frame(&mut stream, req).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn wait_for(&self, id: &str, expected: &str) -> downloader_ipc::DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let DaemonEvent::DetailedQueue { tasks } = self.request(&ClientRequest::GetDetailedQueue) {
                if let Some(task) = tasks.into_iter().find(|t| t.url == id) {
                    if task.state == expected || (expected == "failed:" && task.state.starts_with(expected)) { return task; }
                    assert!(!task.state.starts_with("failed:"), "{}", task.state);
                }
            }
            assert!(Instant::now() < deadline, "task did not reach {expected}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { self.stop(); let _ = std::fs::remove_dir_all(&self.root); }
}

fn headers(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    while !out.ends_with(b"\r\n\r\n") && out.len() < 16384 {
        if stream.read(&mut byte).unwrap() == 0 { break; }
        out.push(byte[0]);
    }
    String::from_utf8(out).unwrap().to_ascii_lowercase()
}

fn run_recovery(mode: u8) {
    let changed_etag = mode != 0;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        headers(&mut first);
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\nabcd").unwrap();
        let (mut second, _) = listener.accept().unwrap();
        let request = headers(&mut second);
        assert!(request.contains("range: bytes=4-"), "{request}");
        assert!(request.contains("if-range: \"v1\""), "{request}");
        let etag = if changed_etag { "v2" } else { "v1" };
        let response = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 4-11/12\r\nETag: \"{etag}\"\r\n\r\nefghijkl");
        second.write_all(response.as_bytes()).unwrap();
        if changed_etag {
            let (mut third, _) = listener.accept().unwrap();
            let request = headers(&mut third);
            if mode == 1 {
                assert!(request.contains("range: bytes=4-"), "{request}");
                assert!(request.contains("if-range: \"v1\""), "{request}");
                third.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 4-11/12\r\nETag: \"v1\"\r\n\r\nefghijkl").unwrap();
            } else {
                assert!(!request.contains("range: bytes="), "{request}");
                third.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\nabcdefghijkl").unwrap();
            }
        }
    });
    let mut f = Fixture::new();
    f.start();
    assert!(matches!(f.request(&ClientRequest::AddTask { url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None }), DaemonEvent::TaskStateChanged { .. }));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(&ClientRequest::GetDetailedQueue) {
            if tasks.iter().any(|t| t.url == url && t.downloaded == 4 && t.resume_supported) { break; }
        }
        assert!(Instant::now() < deadline, "initial bytes not received");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(f.request(&ClientRequest::Pause { task_id: url.clone() }), DaemonEvent::TaskStateChanged { .. }));
    let paused = f.wait_for(&url, "paused");
    assert_eq!(paused.downloaded, 4);
    let partial = f.root.join("Downloads").join(paused.partial_name.unwrap());
    f.stop();
    // Simulate writes after the last durable sidecar checkpoint.
    std::fs::OpenOptions::new().append(true).open(&partial).unwrap().write_all(b"uncheckpointed").unwrap();
    f.start();
    let restored = f.wait_for(&url, "paused");
    assert_eq!(restored.downloaded, 4);
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
    assert!(matches!(f.request(&ClientRequest::Resume { task_id: url.clone() }), DaemonEvent::TaskStateChanged { .. }));
    if changed_etag {
        let failed = f.wait_for(&url, "failed:");
        assert!(failed.state.contains("validator"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
        assert!(!f.root.join("Downloads/file.bin").exists());
        let sidecar = PathBuf::from(format!("{}.meta", partial.display()));
        if mode == 2 { std::fs::write(&sidecar, b"corrupt").unwrap(); }
        if mode == 3 { std::fs::remove_file(&sidecar).unwrap(); }
        assert!(matches!(f.request(&ClientRequest::AddTask { url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None }), DaemonEvent::TaskStateChanged { .. }));
        f.wait_for(&url, "completed");
        let name = if mode == 1 { "file.bin" } else { "file (1).bin" };
        assert_eq!(std::fs::read(f.root.join("Downloads").join(name)).unwrap(), b"abcdefghijkl");
        if mode == 1 { assert!(!f.root.join("Downloads/file (1).bin").exists()); }
        else { assert_eq!(std::fs::read(&partial).unwrap(), b"abcd"); }
    } else {
        f.wait_for(&url, "completed");
        assert_eq!(std::fs::read(f.root.join("Downloads/file.bin")).unwrap(), b"abcdefghijkl");
    }
    server.join().unwrap();
}

#[test]
fn paused_checkpoint_survives_crash_and_resumes_from_verified_prefix() { run_recovery(0); }

#[test]
fn changed_server_validator_rejects_recovered_partial() { run_recovery(1); }

#[test]
fn corrupt_checkpoint_retry_starts_renamed_without_touching_old_partial() { run_recovery(2); }

#[test]
fn missing_checkpoint_retry_starts_renamed_without_touching_old_partial() { run_recovery(3); }

#[test]
fn corrupt_checkpoint_at_restart_requeues_paused_then_starts_renamed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap(); headers(&mut first);
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\nabcd").unwrap();
        let (mut second, _) = listener.accept().unwrap();
        let request = headers(&mut second); assert!(!request.contains("range: bytes="));
        second.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\nabcdefghijkl").unwrap();
    });
    let mut f = Fixture::new(); f.start();
    f.request(&ClientRequest::AddTask { url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let DaemonEvent::DetailedQueue { tasks } = f.request(&ClientRequest::GetDetailedQueue) {
            if tasks.iter().any(|t| t.url == url && t.downloaded == 4 && t.resume_supported) { break; }
        }
        assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20));
    }
    f.request(&ClientRequest::Pause { task_id: url.clone() });
    let paused = f.wait_for(&url, "paused");
    let partial = f.root.join("Downloads").join(paused.partial_name.unwrap());
    f.stop(); std::fs::write(format!("{}.meta", partial.display()), b"corrupt").unwrap();
    f.start(); f.wait_for(&url, "paused");
    f.request(&ClientRequest::Resume { task_id: url.clone() });
    f.wait_for(&url, "completed");
    assert_eq!(std::fs::read(f.root.join("Downloads/file (1).bin")).unwrap(), b"abcdefghijkl");
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
    server.join().unwrap();
}

#[test]
fn failed_initial_stream_preserves_its_checkpoint_for_same_url_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap(); headers(&mut first);
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=file.bin\r\nConnection: close\r\n\r\nabcd").unwrap();
        drop(first);
        let (mut second, _) = listener.accept().unwrap(); let request = headers(&mut second);
        assert!(request.contains("range: bytes=4-"), "{request}");
        second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 4-11/12\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nefghijkl").unwrap();
    });
    let mut f = Fixture::new(); f.start();
    f.request(&ClientRequest::AddTask { url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None });
    let failed = f.wait_for(&url, "failed:");
    let partial = f.root.join("Downloads").join(failed.partial_name.unwrap());
    assert_eq!(std::fs::read(&partial).unwrap(), b"abcd");
    assert!(PathBuf::from(format!("{}.meta", partial.display())).exists());
    f.request(&ClientRequest::AddTask { url: url.clone(), mirrors: vec![], connections: 1, extract: false, category: None, not_before: None });
    f.wait_for(&url, "completed");
    assert_eq!(std::fs::read(f.root.join("Downloads/file.bin")).unwrap(), b"abcdefghijkl");
    server.join().unwrap();
}

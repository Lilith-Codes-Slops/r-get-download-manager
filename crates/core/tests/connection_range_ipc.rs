#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder};
use std::{io::{Read, Write}, net::TcpListener, os::unix::net::UnixStream,
    process::{Child, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

struct Daemon { process: Child, root: std::path::PathBuf }
impl Drop for Daemon {
    fn drop(&mut self) { let _ = self.process.kill(); let _ = self.process.wait(); let _ = std::fs::remove_dir_all(&self.root); }
}
impl Daemon {
    fn start() -> Self {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-count-range-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        let process = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root).env("XDG_RUNTIME_DIR", root.join("run")).env("XDG_STATE_HOME", root.join("state"))
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let mut daemon = Self { process, root };
        let deadline = Instant::now() + Duration::from_secs(15);
        while UnixStream::connect(daemon.root.join("run/rget.sock")).is_err() {
            assert!(daemon.process.try_wait().unwrap().is_none(), "daemon exited before listening");
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
}

#[test]
fn accepts_every_connection_count_and_falls_back_for_small_non_range_files() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let active = running.clone();
    let server = std::thread::spawn(move || {
        while active.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => { std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut request = Vec::new(); let mut byte = [0u8; 1];
                    while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
                        if !matches!(stream.read(&mut byte), Ok(1)) { return; }
                        request.push(byte[0]);
                    }
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc");
                }); },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    });
    let daemon = Daemon::start();
    for count in 1..=32 {
        let url = format!("http://{address}/file-{count}");
        let response = daemon.request(ClientRequest::AddTask { url, mirrors: vec![], connections: count,
            extract: false, category: None, not_before: None });
        assert!(matches!(response, DaemonEvent::TaskStateChanged { ref state, .. } if state == "downloading"), "count {count}: {response:?}");
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let DaemonEvent::DetailedQueue { tasks } = daemon.request(ClientRequest::GetDetailedQueue) else { panic!("missing queue") };
        assert!(tasks.iter().all(|task| !task.state.starts_with("failed:")), "{tasks:?}");
        if tasks.len() == 32 && tasks.iter().all(|task| task.state == "completed") { break; }
        assert!(Instant::now() < deadline, "downloads did not complete: {tasks:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
    for count in 1..=32 { assert_eq!(std::fs::read(daemon.root.join("Downloads").join(format!("file-{count}"))).unwrap(), b"abc"); }
    running.store(false, Ordering::SeqCst);
    server.join().unwrap();
}

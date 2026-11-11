#![cfg(target_os = "linux")]

use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
            std::env::temp_dir().join(format!("rget-actions-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        Self { root, daemon: None }
    }
    fn start(&mut self) {
        self.daemon = Some(
            Command::new(env!("CARGO_BIN_EXE_downloader-core"))
                .env("HOME", &self.root)
                .env("XDG_RUNTIME_DIR", self.root.join("run"))
                .env("XDG_STATE_HOME", self.root.join("state"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
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
            .set_read_timeout(Some(Duration::from_secs(25)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, &req).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }
    fn add(&self, url: &str) {
        let event = self.request(ClientRequest::AddTask {
            url: url.into(),
            mirrors: vec![],
            connections: 1,
            extract: false,
            category: None,
            not_before: None,
        });
        assert!(
            matches!(event, DaemonEvent::TaskStateChanged { .. }),
            "{event:?}"
        );
    }
    fn task(&self, url: &str) -> Option<DownloadInfo> {
        match self.request(ClientRequest::GetDetailedQueue) {
            DaemonEvent::DetailedQueue { tasks } => tasks.into_iter().find(|t| t.url == url),
            other => panic!("unexpected queue: {other:?}"),
        }
    }
    fn wait(&self, url: &str, state: &str) -> DownloadInfo {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(task) = self.task(url) {
                if task.state == state || (state == "failed:" && task.state.starts_with(state)) {
                    return task;
                }
            }
            assert!(Instant::now() < deadline, "task did not reach {state}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn delete(&self, url: &str) {
        let event = self.request(ClientRequest::RemoveTask {
            task_id: url.into(),
            delete_file: true,
        });
        assert!(
            matches!(event, DaemonEvent::TaskStateChanged { .. }),
            "{event:?}"
        );
        assert!(self.task(url).is_none());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn serve(
    held: bool,
    filename: &'static str,
) -> (
    String,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut data = [0u8; 4096];
        let _ = stream.read(&mut data).unwrap();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename={filename}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", if held { 100 } else { 3 });
        stream.write_all(header.as_bytes()).unwrap();
        let _ = stream.write_all(b"abc");
        if held {
            let _ = rx.recv_timeout(Duration::from_secs(15));
        }
    });
    (url, tx, server)
}

#[test]
fn cancel_keeps_history_and_delete_active_cleans_partial() {
    let mut f = Fixture::new();
    f.start();
    let (url, release, server) = serve(true, "held.bin");
    f.add(&url);
    let deadline = Instant::now() + Duration::from_secs(10);
    let partial = loop {
        if let Some(t) = f.task(&url) {
            if t.downloaded > 0 {
                if let Some(partial) = t.partial_name {
                    break partial;
                }
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    let task = f.task(&url).unwrap();
    assert!(task.final_path.is_none());
    assert!(task
        .destination_path
        .as_deref()
        .is_some_and(|path| path.ends_with("held.bin")));
    let event = f.request(ClientRequest::Cancel {
        task_id: url.clone(),
    });
    assert!(
        matches!(event, DaemonEvent::TaskStateChanged { .. }),
        "{event:?}"
    );
    f.wait(&url, "canceled");
    assert!(!f.root.join("Downloads").join(&partial).exists());
    assert!(!f
        .root
        .join("Downloads")
        .join(format!("{partial}.meta"))
        .exists());
    f.stop();
    f.start();
    f.wait(&url, "canceled");
    f.delete(&url);
    assert!(!f.root.join("Downloads/held.bin").exists());
    drop(release);
    server.join().unwrap();

    let (url, release, server) = serve(true, "delete-active.bin");
    f.add(&url);
    let deadline = Instant::now() + Duration::from_secs(10);
    let partial = loop {
        if let Some(t) = f.task(&url) {
            if t.downloaded > 0 {
                if let Some(partial) = t.partial_name {
                    break partial;
                }
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    f.delete(&url);
    assert!(!f.root.join("Downloads").join(&partial).exists());
    assert!(!f
        .root
        .join("Downloads")
        .join(format!("{partial}.meta"))
        .exists());
    assert!(!f.root.join("Downloads/delete-active.bin").exists());
    drop(release);
    server.join().unwrap();
}

#[test]
fn delete_completed_or_missing_file_and_failed_collision() {
    let mut f = Fixture::new();
    f.start();
    let (url, release, server) = serve(false, "complete.bin");
    f.add(&url);
    f.wait(&url, "completed");
    assert!(f.root.join("Downloads/complete.bin").exists());
    f.delete(&url);
    assert!(!f.root.join("Downloads/complete.bin").exists());
    drop(release);
    server.join().unwrap();
    f.stop();
    f.start();
    assert!(f.task(&url).is_none());

    let (url, release, server) = serve(false, "missing.bin");
    f.add(&url);
    f.wait(&url, "completed");
    std::fs::remove_file(f.root.join("Downloads/missing.bin")).unwrap();
    f.delete(&url);
    drop(release);
    server.join().unwrap();

    std::fs::write(f.root.join("Downloads/existing.bin"), b"other file").unwrap();
    let (url, release, server) = serve(false, "existing.bin");
    f.add(&url);
    let task = f.wait(&url, "completed");
    assert_eq!(task.filename.as_deref(), Some("existing (1).bin"));
    assert_eq!(task.suggested_filename.as_deref(), Some("existing.bin"));
    assert_eq!(task.destination_path.as_deref(), task.final_path.as_deref());
    let state = f.request(ClientRequest::GetSnapshot {
        task_id: Some(url.clone()),
    });
    assert!(
        matches!(state, DaemonEvent::TaskStateChanged { destination_path: Some(ref path), .. } if path.ends_with("existing (1).bin"))
    );
    assert_eq!(
        std::fs::read(f.root.join("Downloads/existing (1).bin")).unwrap(),
        b"abc"
    );
    f.delete(&url);
    assert!(!f.root.join("Downloads/existing (1).bin").exists());
    assert_eq!(
        std::fs::read(f.root.join("Downloads/existing.bin")).unwrap(),
        b"other file"
    );
    drop(release);
    server.join().unwrap();
}

#[test]
fn add_after_completed_starts_another_download_without_overwriting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for body in [b"one", b"two"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Disposition: attachment; filename=again.bin\r\nConnection: close\r\n\r\n").unwrap();
            stream.write_all(body).unwrap();
        }
    });
    let mut f = Fixture::new();
    f.start();
    f.add(&url);
    f.wait(&url, "completed");
    f.add(&url);
    f.wait(&url, "completed");
    assert_eq!(
        std::fs::read(f.root.join("Downloads/again.bin")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(f.root.join("Downloads/again (1).bin")).unwrap(),
        b"two"
    );
    server.join().unwrap();
}

fn serve_delayed_name(
    name: &'static str,
    body: &'static [u8],
) -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<()>,
) {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let release = Arc::new(AtomicBool::new(false));
    let ready = release.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut data = [0u8; 4096];
        let _ = stream.read(&mut data).unwrap();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename={name}\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(header.as_bytes()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = stream.write_all(body);
    });
    (url, release, server)
}

fn wait_for_filename(f: &Fixture, url: &str, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(task) = f.task(url) {
            if task.filename.as_deref() == Some(name) {
                return;
            }
            assert!(!task.state.starts_with("failed:"), "{}", task.state);
        }
        assert!(Instant::now() < deadline, "task did not select {name}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn concurrent_adds_with_same_name_publish_distinct_files() {
    use std::sync::atomic::Ordering;
    let mut f = Fixture::new();
    f.start();
    let (a, release_a, server_a) = serve_delayed_name("same.bin", b"first");
    let (b, release_b, server_b) = serve_delayed_name("same.bin", b"second");
    f.add(&a);
    f.add(&b);
    wait_for_filename(&f, &a, "same.bin");
    wait_for_filename(&f, &b, "same.bin");
    release_a.store(true, Ordering::SeqCst);
    release_b.store(true, Ordering::SeqCst);
    let ta = f.wait(&a, "completed");
    let tb = f.wait(&b, "completed");
    assert_eq!(ta.suggested_filename.as_deref(), Some("same.bin"));
    assert_eq!(tb.suggested_filename.as_deref(), Some("same.bin"));
    let pa = ta.final_path.unwrap();
    let pb = tb.final_path.unwrap();
    assert_ne!(pa, pb);
    assert_eq!(std::fs::read(&pa).unwrap(), b"first");
    assert_eq!(std::fs::read(&pb).unwrap(), b"second");
    assert!(f.root.join("Downloads/same.bin").exists());
    assert!(f.root.join("Downloads/same (1).bin").exists());
    server_a.join().unwrap();
    server_b.join().unwrap();
}

#[test]
fn late_collision_on_first_suffix_advances_to_second_suffix() {
    use std::sync::atomic::Ordering;
    let mut f = Fixture::new();
    f.start();
    std::fs::create_dir_all(f.root.join("Downloads")).unwrap();
    std::fs::write(f.root.join("Downloads/late.bin"), b"original").unwrap();
    let (url, release, server) = serve_delayed_name("late.bin", b"download");
    f.add(&url);
    wait_for_filename(&f, &url, "late (1).bin");
    std::fs::write(f.root.join("Downloads/late (1).bin"), b"other").unwrap();
    release.store(true, Ordering::SeqCst);
    let task = f.wait(&url, "completed");
    assert_eq!(task.filename.as_deref(), Some("late (2).bin"));
    assert_eq!(task.suggested_filename.as_deref(), Some("late.bin"));
    assert_eq!(
        std::fs::read(f.root.join("Downloads/late.bin")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(f.root.join("Downloads/late (1).bin")).unwrap(),
        b"other"
    );
    assert_eq!(
        std::fs::read(f.root.join("Downloads/late (2).bin")).unwrap(),
        b"download"
    );
    assert!(!f.root.join("Downloads/late (1) (1).bin").exists());
    server.join().unwrap();
}

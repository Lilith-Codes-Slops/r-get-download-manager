#![cfg(target_os = "linux")]
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, FrameDecoder, FrameEncoder};
use std::{
    io::{Read, Write},
    net::TcpListener,
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Daemon {
    root: PathBuf,
    process: Child,
}
impl Daemon {
    fn start() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rget-progress-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        let mut process = Command::new(env!("CARGO_BIN_EXE_downloader-core"))
            .env("HOME", &root)
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .env("XDG_STATE_HOME", root.join("state"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(root.join("run/rget.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            if let Some(status) = process.try_wait().unwrap() {
                panic!("daemon exited: {status}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Self { root, process }
    }
    fn stream(&self, request: &ClientRequest) -> UnixStream {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, request).unwrap();
        stream
    }
    fn request(&self, request: ClientRequest) -> DaemonEvent {
        FrameDecoder::read_frame(&mut self.stream(&request)).unwrap()
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn next_task(
    stream: &mut UnixStream,
    id: &str,
    test: impl Fn(&DownloadInfo) -> bool,
) -> DownloadInfo {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "progress update timed out");
        match FrameDecoder::read_frame::<_, DaemonEvent>(stream).unwrap() {
            DaemonEvent::ProgressUpdate { task_id, task }
                if task_id == task.url && task.url == id && test(task.as_ref()) =>
            {
                return *task
            }
            DaemonEvent::ProgressUpdate { task_id, task }
                if task_id == task.url && task.url == id && task.state.starts_with("failed:") =>
            {
                panic!("{}", task.state)
            }
            _ => {}
        }
    }
}

#[test]
fn progress_stream_reports_data_completion_reconnect_and_removal() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let (release, wait) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Disposition: attachment; filename=stream.bin\r\nConnection: close\r\n\r\nabc").unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        stream.write_all(b"def").unwrap();
    });
    let daemon = Daemon::start();
    let mut stream = daemon.stream(&ClientRequest::SubscribeProgress);
    assert!(matches!(
        FrameDecoder::read_frame::<_, DaemonEvent>(&mut stream).unwrap(),
        DaemonEvent::ProgressReset
    ));
    assert!(matches!(
        daemon.request(ClientRequest::AddTask {
            url: url.clone(),
            mirrors: vec![],
            connections: 1,
            extract: false,
            category: None,
            not_before: None
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let active = next_task(&mut stream, &url, |task| task.downloaded == 3);
    assert_eq!(active.total, Some(6));
    assert_eq!(active.filename.as_deref(), Some("stream.bin"));
    assert_eq!(active.suggested_filename.as_deref(), Some("stream.bin"));
    assert!(active
        .destination_path
        .as_deref()
        .is_some_and(|path| path.ends_with("stream.bin")));
    assert!(active.final_path.is_none());
    let mut second = daemon.stream(&ClientRequest::SubscribeProgress);
    assert!(matches!(
        FrameDecoder::read_frame::<_, DaemonEvent>(&mut second).unwrap(),
        DaemonEvent::ProgressReset
    ));
    let replay = next_task(&mut second, &url, |task| task.downloaded == 3);
    assert_eq!(replay.partial_name, active.partial_name);
    release.send(()).unwrap();
    let completed = next_task(&mut stream, &url, |task| task.state == "completed");
    assert_eq!(completed.downloaded, 6);
    assert!(completed
        .final_path
        .as_deref()
        .is_some_and(|path| path.ends_with("stream.bin")));
    assert!(matches!(
        daemon.request(ClientRequest::RemoveTask {
            task_id: url.clone(),
            delete_file: false
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "removal not emitted");
        if matches!(FrameDecoder::read_frame::<_, DaemonEvent>(&mut stream).unwrap(), DaemonEvent::ProgressRemoved { task_id } if task_id == url)
        {
            break;
        }
    }
    server.join().unwrap();
}

#[test]
fn cancel_state_is_published_without_queue_polling() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/held", listener.local_addr().unwrap());
    let (release, wait) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Disposition: attachment; filename=held.bin\r\nConnection: close\r\n\r\nabc").unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(10));
    });
    let daemon = Daemon::start();
    let mut stream = daemon.stream(&ClientRequest::SubscribeProgress);
    assert!(matches!(
        FrameDecoder::read_frame::<_, DaemonEvent>(&mut stream).unwrap(),
        DaemonEvent::ProgressReset
    ));
    daemon.request(ClientRequest::AddTask {
        url: url.clone(),
        mirrors: vec![],
        connections: 1,
        extract: false,
        category: None,
        not_before: None,
    });
    next_task(&mut stream, &url, |task| task.downloaded == 3);
    assert!(matches!(
        daemon.request(ClientRequest::Cancel {
            task_id: url.clone()
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let canceled = next_task(&mut stream, &url, |task| task.state == "canceled");
    assert_eq!(canceled.speed_bps, 0);
    release.send(()).unwrap();
    server.join().unwrap();
}

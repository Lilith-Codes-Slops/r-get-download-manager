#![cfg(target_os = "linux")]

use downloader_ipc::{
    ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder, PostDownloadSettings,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
            std::env::temp_dir().join(format!("rget-gui-settings-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::fs::create_dir_all(root.join("Downloads")).unwrap();
        Self { root, daemon: None }
    }

    fn start(&mut self) {
        self.daemon = Some(
            Command::new(env!("CARGO_BIN_EXE_downloader-core"))
                .env("HOME", &self.root)
                .env("XDG_RUNTIME_DIR", self.root.join("run"))
                .env("XDG_STATE_HOME", self.root.join("state"))
                .env("XDG_CONFIG_HOME", self.root.join("config"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
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

    fn request(&self, request: ClientRequest) -> DaemonEvent {
        let mut stream = UnixStream::connect(self.root.join("run/rget.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(25)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, &request).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }

    fn directory(&self) -> PathBuf {
        match self.request(ClientRequest::GetDownloadDirectory) {
            DaemonEvent::DownloadDirectory { path } => path.into(),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn settings(&self) -> BTreeMap<String, Value> {
        match self.request(ClientRequest::GetGuiSettings) {
            DaemonEvent::GuiSettings { overrides } => overrides,
            other => panic!("unexpected response: {other:?}"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn confirmed_start_fails_closed_until_publication_is_integrated() {
    let mut fixture = Fixture::new();
    let one_time = fixture.root.join("one-time");
    std::fs::create_dir(&one_time).unwrap();
    fixture.start();
    let original = fixture.directory();
    for (url, filename) in [
        ("https://example.test/file", "file.txt"),
        (
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "file.txt",
        ),
        ("https://example.test/no-name", ""),
    ] {
        let response = fixture.request(ClientRequest::AddConfirmedTask {
            url: url.into(),
            mirrors: vec![],
            connections: 1,
            extract: false,
            category: None,
            not_before: None,
            filename: filename.into(),
            folder: one_time.to_str().unwrap().into(),
        });
        match response {
            DaemonEvent::TaskFailed { task_id, error } => {
                assert_eq!(task_id, url);
                if filename.is_empty() {
                    assert!(error.contains("filename is unavailable"));
                } else {
                    assert!(error.contains("publication confirmation"));
                }
            }
            other => panic!("confirmed request must be rejected: {other:?}"),
        }
    }
    assert_eq!(fixture.directory(), original);
    assert_eq!(std::fs::read_dir(&one_time).unwrap().count(), 0);
    let conn = rusqlite::Connection::open(fixture.root.join("state/rget/queue.sqlite3")).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rget_queue_v1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn retry_and_restart_commands_fail_closed_without_touching_files() {
    let mut fixture = Fixture::new();
    let partial = fixture.root.join("Downloads/.rget-test.part");
    std::fs::write(&partial, b"partial data").unwrap();
    fixture.start();
    for request in [
        ClientRequest::Retry {
            task_id: "test".into(),
        },
        ClientRequest::RestartFromBeginning {
            task_id: "test".into(),
            confirmed: false,
        },
        ClientRequest::RestartFromBeginning {
            task_id: "test".into(),
            confirmed: true,
        },
    ] {
        match fixture.request(request) {
            DaemonEvent::TaskFailed { error, .. } => {
                assert!(error.contains("partial data was not changed"))
            }
            other => panic!("retry/restart must fail closed: {other:?}"),
        }
        assert_eq!(std::fs::read(&partial).unwrap(), b"partial data");
    }
}

#[test]
fn gui_overrides_survive_restart_and_reset_is_isolated() {
    let mut fixture = Fixture::new();
    std::fs::create_dir_all(fixture.root.join("config")).unwrap();
    std::fs::write(
        fixture.root.join("config/user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Preferred Downloads\"\n",
    )
    .unwrap();
    fixture.start();
    let conn = rusqlite::Connection::open(fixture.root.join("state/rget/queue.sqlite3")).unwrap();
    let default: String = conn
        .query_row(
            "SELECT default_path FROM rget_download_directory_v1 WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        PathBuf::from(default),
        fixture.root.join("Preferred Downloads")
    );
    drop(conn);
    assert_eq!(
        fixture.directory(),
        fixture.root.join("Preferred Downloads")
    );
    let selected = fixture.root.join("new downloads");
    std::fs::create_dir(&selected).unwrap();
    std::fs::write(selected.join("keep.txt"), b"keep").unwrap();
    assert!(matches!(
        fixture.request(ClientRequest::SetDownloadDirectory {
            path: selected.to_str().unwrap().into()
        }),
        DaemonEvent::DownloadDirectory { .. }
    ));
    assert_eq!(fixture.directory(), selected.canonicalize().unwrap());
    let one_time = fixture.root.join("one-time folder");
    std::fs::create_dir(&one_time).unwrap();
    assert!(matches!(
        fixture.request(ClientRequest::ValidateDownloadDirectory {
            path: one_time.to_str().unwrap().into(),
        }),
        DaemonEvent::DownloadDirectory { path } if PathBuf::from(path) == one_time.canonicalize().unwrap()
    ));
    assert_eq!(fixture.directory(), selected.canonicalize().unwrap());
    assert!(matches!(
        fixture.request(ClientRequest::ValidateDownloadDirectory {
            path: "relative".into(),
        }),
        DaemonEvent::TaskFailed { .. }
    ));
    let url = "http://127.0.0.1:1/example".to_string();
    assert!(matches!(
        fixture.request(ClientRequest::AddTask {
            url: url.clone(),
            mirrors: vec![],
            connections: 1,
            extract: false,
            category: None,
            not_before: None,
        }),
        DaemonEvent::TaskStateChanged { .. }
    ));
    let conn = rusqlite::Connection::open(fixture.root.join("state/rget/queue.sqlite3")).unwrap();
    let bound: String = conn
        .query_row(
            "SELECT root FROM rget_task_directory_v1 WHERE task_id=?1",
            [&url],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(PathBuf::from(bound), selected.canonicalize().unwrap());
    drop(conn);
    assert!(matches!(
        fixture.request(ClientRequest::SetDownloadDirectory {
            path: "relative".into()
        }),
        DaemonEvent::TaskFailed { .. }
    ));
    std::fs::write(
        fixture.root.join("config/user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Changed\"\n",
    )
    .unwrap();
    assert!(fixture.settings().is_empty());
    let response = fixture.request(ClientRequest::SetGuiSetting {
        key: "theme".into(),
        value: Some(json!("dark")),
    });
    assert!(matches!(response, DaemonEvent::GuiSettings { .. }));
    fixture.stop();
    fixture.start();
    assert_eq!(fixture.directory(), selected.canonicalize().unwrap());
    assert_eq!(fixture.settings().get("theme"), Some(&json!("dark")));
    let changes = [
        ("theme".into(), None),
        ("compact".into(), Some(json!(true))),
    ]
    .into_iter()
    .collect();
    assert!(matches!(
        fixture.request(ClientRequest::UpdateGuiSettings { changes }),
        DaemonEvent::GuiSettings { .. }
    ));
    assert_eq!(fixture.settings().get("compact"), Some(&json!(true)));
    assert!(!fixture.settings().contains_key("theme"));
    assert!(matches!(
        fixture.request(ClientRequest::SetGuiSetting {
            key: "max_concurrent".into(),
            value: Some(json!(0)),
        }),
        DaemonEvent::TaskFailed { .. }
    ));
    assert_eq!(fixture.settings().len(), 1);
    let post_download = PostDownloadSettings {
        open_containing_folder: true,
    };
    assert!(matches!(
        fixture.request(ClientRequest::SetPostDownloadSettings {
            settings: post_download
        }),
        DaemonEvent::PostDownloadSettings { .. }
    ));
    assert!(matches!(
        fixture.request(ClientRequest::ResetGuiSettings),
        DaemonEvent::GuiSettings { .. }
    ));
    assert!(fixture.settings().is_empty());
    assert_eq!(
        fixture.directory(),
        fixture.root.join("Preferred Downloads")
    );
    assert_eq!(std::fs::read(selected.join("keep.txt")).unwrap(), b"keep");
    assert!(
        matches!(fixture.request(ClientRequest::GetPostDownloadSettings), DaemonEvent::PostDownloadSettings { settings } if settings == post_download)
    );
    fixture.stop();
    fixture.start();
    assert!(fixture.settings().is_empty());
    let conn = rusqlite::Connection::open(fixture.root.join("state/rget/queue.sqlite3")).unwrap();
    let default: String = conn
        .query_row(
            "SELECT default_path FROM rget_download_directory_v1 WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        PathBuf::from(default),
        fixture.root.join("Preferred Downloads")
    );
    let bound: String = conn
        .query_row(
            "SELECT root FROM rget_task_directory_v1 WHERE task_id=?1",
            [&url],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(PathBuf::from(bound), selected.canonicalize().unwrap());
}

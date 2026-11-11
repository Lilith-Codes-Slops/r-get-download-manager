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

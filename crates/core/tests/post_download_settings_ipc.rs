#![cfg(target_os = "linux")]

use downloader_ipc::{
    ClientRequest, DaemonEvent, FrameDecoder, FrameEncoder, PostDownloadSettings,
};
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
        let root = std::env::temp_dir().join(format!(
            "rget-post-download-settings-{}-{nonce}",
            std::process::id()
        ));
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
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        FrameEncoder::write_frame(&mut stream, &req).unwrap();
        FrameDecoder::read_frame(&mut stream).unwrap()
    }

    fn settings(&self) -> PostDownloadSettings {
        match self.request(ClientRequest::GetPostDownloadSettings) {
            DaemonEvent::PostDownloadSettings { settings } => settings,
            other => panic!("unexpected settings response: {other:?}"),
        }
    }

    fn set(&self, settings: PostDownloadSettings) {
        let response = self.request(ClientRequest::SetPostDownloadSettings { settings });
        assert!(
            matches!(
                &response,
                DaemonEvent::PostDownloadSettings { settings: current } if *current == settings
            ),
            "unexpected set response: {response:?}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn global_post_download_setting_roundtrips_and_survives_restart() {
    let mut fixture = Fixture::new();
    fixture.start();
    assert_eq!(fixture.settings(), PostDownloadSettings::default());

    let enabled = PostDownloadSettings {
        open_containing_folder: true,
    };
    fixture.set(enabled);
    assert_eq!(fixture.settings(), enabled);

    fixture.stop();
    fixture.start();
    assert_eq!(fixture.settings(), enabled);

    let disabled = PostDownloadSettings::default();
    fixture.set(disabled);
    assert_eq!(fixture.settings(), disabled);
}

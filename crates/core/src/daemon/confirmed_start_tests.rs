//! Tests of the staged admission path; IPC remains fail-closed until all paths
//! and the real daemon have been verified.
use super::confirmed_start;
use super::{
    bandwidth::BandwidthLimiter,
    control::{Controls, Tasks},
    persistence::Persistence,
};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

async fn wait_for_durable_state(store: &Persistence, id: &str, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let saved = store.load().await.unwrap();
            if saved
                .tasks
                .get(id)
                .is_some_and(|task| task.state.contains(expected))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

fn request(url: &str, filename: &str, folder: &std::path::Path) -> ClientRequest {
    ClientRequest::AddConfirmedTask {
        url: url.into(),
        mirrors: vec![],
        connections: 1,
        extract: false,
        category: None,
        not_before: None,
        filename: filename.into(),
        folder: folder.to_str().unwrap().into(),
    }
}

#[tokio::test]
async fn staged_http_admission_persists_choice_and_rejects_duplicates() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rget-confirmed-admission-{}-{unique}",
        std::process::id()
    ));
    let selected = root.join("one-time");
    let other = root.join("other");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let store = Arc::new(Persistence::new(root.join("state/queue.sqlite3")));
    let global = store.load_download_directory().await.unwrap();
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::new()));
    let controls: Controls = Arc::new(Mutex::new(BTreeMap::new()));
    let limiter = Arc::new(BandwidthLimiter::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/source", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .await
            .unwrap();
    });
    let (a, b) = tokio::join!(
        confirmed_start::start(
            request(&url, "chosen.txt", &selected),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        ),
        confirmed_start::start(
            request(&url, "other.txt", &other),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        ),
    );
    let (accepted, rejected) = match (a, b) {
        (DaemonEvent::TaskStateChanged { .. }, DaemonEvent::TaskFailed { error, .. }) => {
            (("chosen.txt", selected.clone()), error)
        }
        (DaemonEvent::TaskFailed { error, .. }, DaemonEvent::TaskStateChanged { .. }) => {
            (("other.txt", other.clone()), error)
        }
        result => panic!("expected one admitted task and one duplicate: {result:?}"),
    };
    assert!(rejected.contains("remove the existing task first"));
    let bound = store.task_directory(&url).await.unwrap();
    assert_eq!(bound, accepted.1.canonicalize().unwrap());
    assert_eq!(store.load_download_directory().await.unwrap(), global);
    let conn = super::persistence::open(&store.database_path()).unwrap();
    let json: String = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&url],
            |row| row.get(0),
        )
        .unwrap();
    let durable: DownloadInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(durable.confirmed_filename.as_deref(), Some(accepted.0));
    drop(conn);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = tasks.lock().await.get(&url).unwrap().state.clone();
            if state == "completed" {
                break;
            }
            assert!(!state.starts_with("failed:"), "transfer failed: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(
        std::fs::read(accepted.1.join(accepted.0)).unwrap(),
        b"hello"
    );
    // Observe the worker's own terminal checkpoint before removing the fixture.
    wait_for_durable_state(&store, &url, "completed").await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn staged_http_retry_uses_original_root_name_and_range() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rget-confirmed-retry-{}-{unique}",
        std::process::id()
    ));
    let selected = root.join("selected");
    let other = root.join("other");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let store = Arc::new(Persistence::new(root.join("state/queue.sqlite3")));
    store.load_download_directory().await.unwrap();
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::new()));
    let controls: Controls = Arc::new(Mutex::new(BTreeMap::new()));
    let limiter = Arc::new(BandwidthLimiter::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for part in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let count = socket.read(&mut buf).await.unwrap();
                assert!(count > 0);
                data.extend_from_slice(&buf[..count]);
                if data.windows(4).any(|v| v == b"\r\n\r\n") {
                    break;
                }
                assert!(data.len() < 16384);
            }
            if part == 0 {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nConnection: close\r\n\r\nhello").await.unwrap();
            } else {
                assert!(String::from_utf8(data)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("range: bytes=5-"));
                socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 6\r\nContent-Range: bytes 5-10/11\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n world").await.unwrap();
            }
        }
    });
    assert!(matches!(
        confirmed_start::start(
            request(&url, "chosen.txt", &selected),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        )
        .await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if tasks
                .lock()
                .await
                .get(&url)
                .unwrap()
                .state
                .starts_with("failed:")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let before = tasks.lock().await.get(&url).unwrap().clone();
    let partial = before.partial_name.as_ref().unwrap();
    assert_eq!(std::fs::read(selected.join(partial)).unwrap(), b"hello");
    assert!(matches!(
        confirmed_start::start(
            request(&url, "other.txt", &other),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        )
        .await,
        DaemonEvent::TaskFailed { .. }
    ));
    assert!(matches!(
        confirmed_start::retry_http(
            url.clone(),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        )
        .await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = tasks.lock().await.get(&url).unwrap().state.clone();
            if state == "completed" {
                break;
            }
            assert!(!state.starts_with("failed:"), "retry failed: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(
        std::fs::read(selected.join("chosen.txt")).unwrap(),
        b"hello world"
    );
    assert!(!other.join("other.txt").exists());
    assert_eq!(
        store.task_directory(&url).await.unwrap(),
        selected.canonicalize().unwrap()
    );
    wait_for_durable_state(&store, &url, "completed").await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn staged_admission_rejects_invalid_input_without_creating_a_task() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rget-confirmed-invalid-{}-{unique}",
        std::process::id()
    ));
    let selected = root.join("selected");
    std::fs::create_dir_all(&selected).unwrap();
    let store = Arc::new(Persistence::new(root.join("state/queue.sqlite3")));
    let global = store.load_download_directory().await.unwrap();
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::new()));
    let controls: Controls = Arc::new(Mutex::new(BTreeMap::new()));
    let limiter = Arc::new(BandwidthLimiter::new());
    let url = "http://example.test/file";
    let mut unsupported = request(url, "ok", &selected);
    if let ClientRequest::AddConfirmedTask { mirrors, .. } = &mut unsupported {
        mirrors.push("http://example.test/mirror".into());
    }
    for input in [
        request(url, "../escape", &selected),
        request(url, "ok", &root.join("missing")),
        unsupported,
    ] {
        assert!(matches!(
            confirmed_start::start(
                input,
                tasks.clone(),
                controls.clone(),
                limiter.clone(),
                store.clone()
            )
            .await,
            DaemonEvent::TaskFailed { .. }
        ));
    }
    assert!(tasks.lock().await.is_empty());
    assert_eq!(store.load_download_directory().await.unwrap(), global);
    let conn = super::persistence::open(&store.database_path()).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rget_queue_v1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    drop(conn);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unsafe_http_retry_fails_closed_without_deleting_partial() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rget-unsafe-retry-{}-{unique}", std::process::id()));
    let selected = root.join("selected");
    std::fs::create_dir_all(&selected).unwrap();
    let store = Arc::new(Persistence::new(root.join("state/queue.sqlite3")));
    store.load_download_directory().await.unwrap();
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::new()));
    let controls: Controls = Arc::new(Mutex::new(BTreeMap::new()));
    let limiter = Arc::new(BandwidthLimiter::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        // No ETag or range support; truncate the response mid-body.
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello")
            .await
            .unwrap();
    });
    assert!(matches!(
        confirmed_start::start(
            request(&url, "chosen.txt", &selected),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        )
        .await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if tasks
                .lock()
                .await
                .get(&url)
                .unwrap()
                .state
                .starts_with("failed:")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    let partial = tasks
        .lock()
        .await
        .get(&url)
        .unwrap()
        .partial_name
        .clone()
        .unwrap();
    assert_eq!(std::fs::read(selected.join(&partial)).unwrap(), b"hello");
    assert!(matches!(
        confirmed_start::retry_http(
            url.clone(),
            tasks.clone(),
            controls.clone(),
            limiter.clone(),
            store.clone()
        )
        .await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = tasks.lock().await.get(&url).unwrap().state.clone();
            if state.starts_with("failed:") {
                assert!(state.contains("cannot resume automatically"), "{state}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(selected.join(&partial)).unwrap(), b"hello");
    assert!(!selected.join("chosen.txt").exists());
    wait_for_durable_state(&store, &url, "cannot resume automatically").await;
    std::fs::remove_dir_all(root).unwrap();
}

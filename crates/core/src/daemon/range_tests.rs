use super::bandwidth::BandwidthLimiter;
use super::control::{set_paused, Controls, Tasks, TransferControl};
use super::{range, transfer_body};
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, Mutex};

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut byte = [0; 1];
    while !data.ends_with(b"\r\n\r\n") && data.len() < 16384 {
        if stream.read(&mut byte).await.unwrap() == 0 {
            break;
        }
        data.push(byte[0]);
    }
    String::from_utf8(data).unwrap().to_ascii_lowercase()
}

async fn run_resume_test(mode: u8) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        read_request(&mut first).await;
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\n\r\nabcd").await.unwrap();
        let (mut second, _) = listener.accept().await.unwrap();
        let request = read_request(&mut second).await;
        assert!(request.contains("range: bytes=4-"), "{request}");
        assert!(request.contains("if-range: \"v1\""), "{request}");
        let etag = if mode == 1 { "v2" } else { "v1" };
        let status = if mode == 2 {
            "200 OK"
        } else {
            "206 Partial Content"
        };
        let range = if mode == 3 {
            "bytes 3-10/12"
        } else {
            "bytes 4-11/12"
        };
        let header = format!("HTTP/1.1 {status}\r\nContent-Length: 8\r\nContent-Range: {range}\r\nETag: \"{etag}\"\r\n\r\nefghijkl");
        second.write_all(header.as_bytes()).await.unwrap();
    });
    let client = reqwest::Client::new();
    let initial = client.get(&url).send().await.unwrap();
    let resume = range::detect(&initial).expect("range support detected");
    let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::new()));
    tasks.lock().await.insert(
        url.clone(),
        DownloadInfo {
            url: url.clone(),
            state: "downloading".into(),
            filename: None,
            downloaded: 0,
            total: Some(12),
            speed_bps: 0,
            final_path: None,
            destination_path: None,
            suggested_filename: None,
            confirmed_filename: None,
            torrent: None,
            resume_supported: true,
            partial_name: None,
            requested_connections: None,
            active_connections: None,
            segment_count: None,
            connection_note: None,
            live_connections_supported: false,
        },
    );
    let controls: Controls = Arc::new(Mutex::new(BTreeMap::new()));
    let (tx, mut rx) = watch::channel(TransferControl::Running);
    controls.lock().await.insert(url.clone(), tx);
    let path = std::env::temp_dir().join(format!("rget-resume-{}-{mode}.tmp", std::process::id()));
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let task_map = tasks.clone();
    let task_url = url.clone();
    let transfer = tokio::spawn(async move {
        let limiter = Arc::new(BandwidthLimiter::new());
        let result = transfer_body::receive(
            &client,
            Some(initial),
            &task_url,
            &mut file,
            &task_map,
            &limiter,
            &mut rx,
            Some(&resume),
            Some(12),
            None,
            0,
        )
        .await;
        file.flush().await.unwrap();
        result.map_err(|e| e.to_string())
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.lock().await.get(&url).unwrap().downloaded != 4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        set_paused(&tasks, &controls, url.clone(), true).await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.lock().await.get(&url).unwrap().state != "paused" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        set_paused(&tasks, &controls, url.clone(), false).await,
        DaemonEvent::TaskStateChanged { .. }
    ));
    let result = tokio::time::timeout(Duration::from_secs(5), transfer)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let bytes = tokio::fs::read(&path).await.unwrap();
    if mode != 0 {
        assert!(result.is_err());
        assert_eq!(bytes, b"abcd");
    } else {
        assert_eq!(result.unwrap(), 12);
        assert_eq!(bytes, b"abcdefghijkl");
    }
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn resumes_with_a_validated_range() {
    run_resume_test(0).await;
}

#[tokio::test]
async fn rejects_changed_validator_before_appending() {
    run_resume_test(1).await;
}

#[tokio::test]
async fn rejects_server_ignoring_range() {
    run_resume_test(2).await;
}

#[tokio::test]
async fn rejects_incorrect_content_range() {
    run_resume_test(3).await;
}

#[tokio::test]
async fn last_modified_resume_requires_a_strong_date_and_exact_match() {
    async fn response(headers: &str, status: &str) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let wire = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket.write_all(wire.as_bytes()).await.unwrap();
        });
        reqwest::Client::new().get(url).send().await.unwrap()
    }
    let modified = "Sun, 06 Nov 1994 08:49:37 GMT";
    let initial = response(&format!("Content-Length: 12\r\nAccept-Ranges: bytes\r\nLast-Modified: {modified}\r\nDate: Sun, 06 Nov 1994 08:51:37 GMT\r\n"), "200 OK").await;
    let mut info = range::detect(&initial).expect("valid Last-Modified");
    assert_eq!(info.etag, modified);
    let partial = response(
        &format!(
            "Content-Length: 8\r\nContent-Range: bytes 4-11/12\r\nLast-Modified: {modified}\r\n"
        ),
        "206 Partial Content",
    )
    .await;
    info.url = partial.url().clone();
    assert!(range::validate(&partial, &info, 4).is_ok());
    let changed = response("Content-Length: 8\r\nContent-Range: bytes 4-11/12\r\nLast-Modified: Sun, 06 Nov 1994 08:49:38 GMT\r\n", "206 Partial Content").await;
    info.url = changed.url().clone();
    assert!(range::validate(&changed, &info, 4).is_err());
    let recent = response(&format!("Content-Length: 12\r\nAccept-Ranges: bytes\r\nLast-Modified: {modified}\r\nDate: Sun, 06 Nov 1994 08:50:00 GMT\r\n"), "200 OK").await;
    assert!(range::detect(&recent).is_none());
    let weak = response(
        "Content-Length: 12\r\nAccept-Ranges: bytes\r\nETag: W/\"v1\"\r\n",
        "200 OK",
    )
    .await;
    assert!(range::detect(&weak).is_none());
}

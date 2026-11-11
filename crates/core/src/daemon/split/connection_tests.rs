//! Measure shared-client HTTP/1.1 reuse against a local keep-alive server.
use super::download_fixed;
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOTAL: u64 = 8 * 1024 * 1024;

async fn serve(mut stream: tokio::net::TcpStream, requests: Arc<AtomicUsize>) {
    loop {
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
            if !matches!(stream.read(&mut byte).await, Ok(1)) {
                return;
            }
            request.push(byte[0]);
        }
        let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
        let Some(range) = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
        else {
            return;
        };
        let Some((start, end)) = range.split_once('-') else {
            return;
        };
        let (Ok(start), Ok(last)) = (start.parse::<u64>(), end.parse::<u64>()) else {
            return;
        };
        if start > last || last >= TOTAL {
            return;
        }
        requests.fetch_add(1, Ordering::SeqCst);
        let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{last}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: keep-alive\r\n\r\n", last - start + 1);
        if stream.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        let mut pos = start;
        while pos <= last {
            let next = (pos + 64 * 1024).min(last + 1);
            let bytes: Vec<u8> = (pos..next).map(|i| (i % 251) as u8).collect();
            if stream.write_all(&bytes).await.is_err() {
                return;
            }
            pos = next;
        }
    }
}

#[tokio::test]
async fn reports_connection_reuse() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let accepts = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (a, r) = (accepts.clone(), requests.clone());
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            a.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(serve(stream, r.clone()));
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("rget-reuse-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap(),
    );
    let info = ResumeInfo {
        etag: "\"v1\"".into(),
        total: TOTAL,
        url: reqwest::Url::parse(&url).unwrap(),
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        download_fixed(
            reqwest::Client::new(),
            url,
            info,
            4,
            file.clone(),
            Arc::new(BandwidthLimiter::new()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    drop(file);
    let data = std::fs::read(&path).unwrap();
    assert_eq!(data.len() as u64, TOTAL);
    assert!(data
        .iter()
        .enumerate()
        .all(|(i, b)| *b == (i as u64 % 251) as u8));
    let (requests, accepts) = (
        requests.load(Ordering::SeqCst),
        accepts.load(Ordering::SeqCst),
    );
    eprintln!("keep-alive measurement: {requests} range requests over {accepts} TCP connections (includes probe)");
    assert!(requests >= 5, "expected at least four ranges plus a probe");
    assert!(accepts > 0);
    std::fs::remove_file(path).unwrap();
    server.abort();
}

//! Exercise changing targets against real HTTP requests, without changing a lease in flight.
use super::{segments::SegmentMap, worker};
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch, Mutex},
};

const MIB: u64 = 1024 * 1024;
const TOTAL: u64 = 80 * MIB;

#[tokio::test]
async fn successive_http_requests_grow_and_shrink_after_policy_changes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let (ranges_tx, mut ranges_rx) = mpsc::unbounded_channel();
    let (permit, mut permits) = watch::channel(0usize);
    let server = tokio::spawn(async move {
        for index in 1..=3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let range = request
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                })
                .unwrap();
            let (start, last) = range.split_once('-').unwrap();
            let start: u64 = start.parse().unwrap();
            let last: u64 = last.parse().unwrap();
            let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{last}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", last - start + 1);
            stream.write_all(header.as_bytes()).await.unwrap();
            ranges_tx.send((start, last + 1)).unwrap();
            while *permits.borrow() < index {
                if permits.changed().await.is_err() {
                    return;
                }
            }
            let mut left = last - start + 1;
            let bytes = vec![42u8; 64 * 1024];
            while left > 0 {
                let n = left.min(bytes.len() as u64) as usize;
                if stream.write_all(&bytes[..n]).await.is_err() {
                    return;
                }
                left -= n as u64;
            }
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("rget-target-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap(),
    );
    let map = Arc::new(Mutex::new(SegmentMap::new(TOTAL, 2).unwrap()));
    map.lock().await.note_rtt(10.0);
    let (stop, _) = watch::channel(false);
    let (_retire, retire) = watch::channel(false);
    let (_drain, drain) = watch::channel(false);
    let info = ResumeInfo {
        etag: "\"v1\"".into(),
        total: TOTAL,
        url: reqwest::Url::parse(&url).unwrap(),
    };
    let context = worker::Context {
        client: reqwest::Client::new(),
        url,
        info,
        file: file.clone(),
        map: map.clone(),
        limiter: Arc::new(BandwidthLimiter::new()),
        stop: stop.subscribe(),
        retire,
        drain,
        desired: None,
        owner: 0,
    };
    let worker = tokio::spawn(worker::run(context));
    let first = tokio::time::timeout(Duration::from_secs(10), ranges_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, (0, 8 * MIB));
    map.lock().await.note_goodput(2.0 * MIB as f64);
    permit.send_replace(1);
    let second = tokio::time::timeout(Duration::from_secs(10), ranges_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second, (8 * MIB, 20 * MIB));
    map.lock().await.note_rejection(429);
    permit.send_replace(2);
    let third = tokio::time::timeout(Duration::from_secs(10), ranges_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third, (20 * MIB, 26 * MIB));
    assert_eq!(map.lock().await.records().len(), 2);
    stop.send_replace(true);
    assert!(tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    server.abort();
    drop(file);
    std::fs::remove_file(path).unwrap();
}

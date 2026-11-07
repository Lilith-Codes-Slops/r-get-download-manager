//! An uneven local server forces an idle worker to steal an active range.
use super::download_fixed;
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

const TOTAL: u64 = 8 * 1024 * 1024;

async fn body(stream: &mut tokio::net::TcpStream, mut start: u64, end: u64) -> bool {
    while start < end {
        let limit = (start + 64 * 1024).min(end);
        let bytes: Vec<u8> = (start..limit).map(|n| (n % 251) as u8).collect();
        if stream.write_all(&bytes).await.is_err() { return false; }
        start = limit;
    }
    true
}

async fn handle(mut stream: tokio::net::TcpStream, started: watch::Sender<bool>, stolen: watch::Sender<bool>, log: Arc<Mutex<Vec<(u64, u64)>>>) {
    let mut bytes = Vec::new(); let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16_384 {
        if !matches!(stream.read(&mut byte).await, Ok(1)) { return; }
        bytes.push(byte[0]);
    }
    let req = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
    let range = req.lines().find_map(|line| line.strip_prefix("range: bytes=")).unwrap();
    let (start, end) = range.split_once('-').unwrap();
    let start: u64 = start.parse().unwrap();
    let end: u64 = end.parse::<u64>().unwrap() + 1;
    assert!(req.contains("if-range: \"v1\""));
    log.lock().unwrap().push((start, end));
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", end - start, end - 1);
    if stream.write_all(header.as_bytes()).await.is_err() { return; }
    if start == 0 && end == 1 { let _ = body(&mut stream, 0, 1).await; return; }
    if start == TOTAL / 2 && end == TOTAL {
        started.send_replace(true);
        let first_end = (start + 64 * 1024).min(end);
        if !body(&mut stream, start, first_end).await { return; }
        let mut ready = stolen.subscribe();
        loop {
            let is_ready = *ready.borrow_and_update();
            if is_ready { break; }
            if ready.changed().await.is_err() { return; }
        }
        let _ = body(&mut stream, first_end, end).await;
    } else if start == 0 && end == TOTAL / 2 {
        let mut ready = started.subscribe();
        loop {
            let is_ready = *ready.borrow_and_update();
            if is_ready { break; }
            if ready.changed().await.is_err() { return; }
        }
        let _ = body(&mut stream, start, end).await;
    } else {
        stolen.send_replace(true);
        let _ = body(&mut stream, start, end).await;
    }
}

#[tokio::test]
async fn idle_worker_steals_without_corrupting_the_file() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let (started, _) = watch::channel(false);
    let (stolen, _) = watch::channel(false);
    let records = log.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            tokio::spawn(handle(stream, started.clone(), stolen.clone(), records.clone()));
        }
    });
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rget-steal-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap());
    let info = ResumeInfo { etag: "\"v1\"".into(), total: TOTAL, url: reqwest::Url::parse(&url).unwrap() };
    tokio::time::timeout(std::time::Duration::from_secs(20), download_fixed(
        reqwest::Client::new(), url, info, 2, file.clone(), Arc::new(BandwidthLimiter::new()),
    )).await.expect("work-stealing timed out").unwrap();
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes.iter().enumerate().all(|(index, byte)| *byte == (index as u64 % 251) as u8));
    let requests = log.lock().unwrap();
    assert!(requests.iter().any(|&(start, end)| start > TOTAL / 2 && end == TOTAL), "no stolen range: {requests:?}");
    std::fs::remove_file(path).unwrap();
    server.abort();
}

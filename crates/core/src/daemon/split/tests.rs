use super::download_fixed;
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) const TOTAL: u64 = 2 * 1024 * 1024;

// mode 0 = valid, 1 = ignored probe, 2 = wrong worker ETag,
// 3 = wrong worker Content-Range, 4 = wrong length, 5 = encoded response,
// 6 = wait for both workers, 7 = truncated body, 8-14 = bad headers/status,
// 15 = ambiguous probe validator.
pub(super) async fn server(mode: u8) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            let barrier = barrier.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
                    if !matches!(stream.read(&mut byte).await, Ok(1)) { return; }
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                let range = request.lines().find_map(|line| line.strip_prefix("range: bytes=")).unwrap();
                let (start, end) = range.split_once('-').unwrap();
                let start: u64 = start.parse().unwrap();
                let end: u64 = end.parse().unwrap();
                assert!(request.contains("if-range: \"v1\""), "{request}");
                assert!(request.contains("accept-encoding: identity"), "{request}");
                let is_probe = start == 0 && end == 0;
                if mode == 6 && !is_probe { barrier.wait().await; }
                let bad = !is_probe;
                let status = if (mode == 1 && is_probe) || (mode == 14 && bad) { "200 OK" } else { "206 Partial Content" };
                let etag = if mode == 9 && bad { String::new() } else if (mode == 8 && bad) || (mode == 15 && is_probe) {
                    "ETag: \"v1\"\r\nETag: \"v2\"\r\n".to_owned()
                } else { format!("ETag: \"{}\"\r\n", if mode == 2 && bad { "v2" } else { "v1" }) };
                let header_start = if mode == 3 && bad { start + 1 } else { start };
                let range_total = if mode == 11 && bad { TOTAL + 1 } else { TOTAL };
                let length = end - start + 1;
                let header_length = if mode == 4 && bad { length - 1 } else { length };
                let extra = match mode {
                    5 if bad => "Content-Encoding: gzip\r\n".to_owned(),
                    10 if bad => format!("Content-Range: bytes {start}-{end}/{TOTAL}\r\n"),
                    12 if bad => "Content-Encoding: identity\r\nContent-Encoding: gzip\r\n".into(),
                    13 if bad => "Content-Type: multipart/byteranges; boundary=test\r\n".into(),
                    _ => String::new(),
                };
                let header = format!("HTTP/1.1 {status}\r\nContent-Length: {header_length}\r\nContent-Range: bytes {header_start}-{end}/{range_total}\r\n{etag}{extra}Connection: close\r\n\r\n");
                if stream.write_all(header.as_bytes()).await.is_err() { return; }
                let body_end = if mode == 7 && !is_probe { start + length / 2 } else { end + 1 };
                let body: Vec<u8> = (start..body_end).map(|n| (n % 251) as u8).collect();
                let _ = stream.write_all(&body).await;
            });
        }
    });
    (url, handle)
}

async fn run(mode: u8) -> (Result<(), String>, Vec<u8>) {
    let (url, server) = server(mode).await;
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rget-split-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap());
    let info = ResumeInfo { etag: "\"v1\"".into(), total: TOTAL, url: reqwest::Url::parse(&url).unwrap() };
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), download_fixed(
        reqwest::Client::new(), url, info, 2, file.clone(), Arc::new(BandwidthLimiter::new()),
    )).await.expect("split transfer timed out");
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    server.abort();
    (result, bytes)
}

#[tokio::test]
async fn downloads_fixed_ranges_into_correct_offsets() {
    let (result, bytes) = run(0).await;
    result.unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes.iter().enumerate().all(|(index, byte)| *byte == (index as u64 % 251) as u8));
}

#[tokio::test]
async fn rejects_probe_before_writing() {
    for mode in [1, 15] {
        let (result, bytes) = run(mode).await;
        assert!(result.is_err());
        assert!(bytes.is_empty());
    }
}

#[tokio::test]
async fn rejects_invalid_worker_responses() {
    for mode in [2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14] {
        let (result, bytes) = run(mode).await;
        assert!(result.is_err(), "accepted invalid response mode {mode}");
        if mode != 7 { assert!(bytes.is_empty(), "wrote bytes for invalid headers in mode {mode}"); }
    }
}

#[tokio::test]
async fn starts_multiple_range_requests_in_parallel() {
    let (result, bytes) = run(6).await;
    result.unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes.iter().enumerate().all(|(index, byte)| *byte == (index as u64 % 251) as u8));
}

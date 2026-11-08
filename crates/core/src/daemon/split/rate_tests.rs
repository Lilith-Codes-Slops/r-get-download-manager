//! End-to-end aggregate task-rate check with two live range workers.
use super::{download_fixed, tests};
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn task_limit_is_shared_by_split_workers() {
    let (url, server) = tests::server(6).await; // both requests must arrive before serving
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rget-split-rate-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap());
    let info = ResumeInfo { etag: "\"v1\"".into(), total: tests::TOTAL, url: reqwest::Url::parse(&url).unwrap() };
    let limiter = Arc::new(BandwidthLimiter::new());
    limiter.set(Some(&url), 8_000).await.unwrap(); // 1 MB/s across both workers
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(15), download_fixed(
        reqwest::Client::new(), url, info, 2, file.clone(), limiter,
    )).await.unwrap().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(1900), "split workers exceeded their shared rate");
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, tests::TOTAL);
    assert!(bytes.iter().enumerate().all(|(i, b)| *b == (i as u64 % 251) as u8));
    std::fs::remove_file(path).unwrap();
    server.abort();
}

//! Exercise actual worker scaling and explicit server capacity rejection.
use super::{adaptive, segments::SegmentMap};
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

struct Counter(Arc<AtomicUsize>);
impl Drop for Counter {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve(
    mut stream: tokio::net::TcpStream,
    total: u64,
    limit: usize,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    rejected: watch::Sender<bool>,
) {
    let mut request = Vec::new();
    let mut byte = [0; 1];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
        if !matches!(stream.read(&mut byte).await, Ok(1)) {
            return;
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
    let Some(range) = request
        .lines()
        .find_map(|line| line.strip_prefix("range: bytes="))
    else {
        return;
    };
    let (start, end) = range.split_once('-').unwrap();
    let start: u64 = start.parse().unwrap();
    let end: u64 = end.parse::<u64>().unwrap() + 1;
    let probe = start == 0 && end == 1;
    let mut guard = None;
    if !probe {
        let previous = active.fetch_add(1, Ordering::SeqCst);
        if previous >= limit {
            active.fetch_sub(1, Ordering::SeqCst);
            rejected.send_replace(true);
            let _ = stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            return;
        }
        guard = Some(Counter(active.clone()));
        peak.fetch_max(previous + 1, Ordering::SeqCst);
    }
    let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", end-start, end-1);
    if stream.write_all(header.as_bytes()).await.is_err() {
        return;
    }
    if !probe && limit == 1 {
        let mut rx = rejected.subscribe();
        loop {
            let ready = *rx.borrow_and_update();
            if ready {
                break;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
    let mut offset = start;
    while offset < end {
        let next = (offset + 64 * 1024).min(end);
        let bytes: Vec<u8> = (offset..next).map(|n| (n % 251) as u8).collect();
        if stream.write_all(&bytes).await.is_err() {
            return;
        }
        offset = next;
        if !probe && limit > 1 && offset < end {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    }
    drop(guard);
}

async fn run(limit: usize, total: u64) -> (usize, bool, usize) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (rejected, _) = watch::channel(false);
    let (a, p, r) = (active.clone(), peak.clone(), rejected.clone());
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(stream, total, limit, a.clone(), p.clone(), r.clone()));
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("rget-adaptive-{}-{nonce}.part", std::process::id()));
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
        total,
        url: reqwest::Url::parse(&url).unwrap(),
    };
    let worker_peak = Arc::new(AtomicUsize::new(0));
    let (stop, _) = watch::channel(false);
    let map = Arc::new(tokio::sync::Mutex::new(SegmentMap::new(total, 4).unwrap()));
    let inputs = adaptive::Inputs {
        client: reqwest::Client::new(),
        url,
        info,
        file: file.clone(),
        map: map.clone(),
        limiter: Arc::new(BandwidthLimiter::new()),
        stop,
        max: 4,
        desired: None,
        peak_workers: Some(worker_peak.clone()),
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        adaptive::supervise(inputs),
    )
    .await
    .expect("adaptive transfer timed out")
    .unwrap();
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, total);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, b)| *b == (i as u64 % 251) as u8));
    std::fs::remove_file(path).unwrap();
    server.abort();
    let was_rejected = *rejected.borrow();
    assert_eq!(map.lock().await.was_rejected(), was_rejected);
    (
        peak.load(Ordering::SeqCst),
        was_rejected,
        worker_peak.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn scales_up_when_throughput_is_healthy() {
    // A server handler can linger after the client drops a stolen response, so
    // its handler count is not a reliable count of active client workers.
    let (_server_peak, rejected, worker_peak) = run(usize::MAX, 32 * 1024 * 1024).await;
    assert!(!rejected);
    assert!(
        (3..=4).contains(&worker_peak),
        "worker count did not scale within the configured maximum: {worker_peak}"
    );
}

#[tokio::test]
async fn continues_after_server_rejects_extra_connections() {
    let (peak, rejected, worker_peak) = run(1, 8 * 1024 * 1024).await;
    assert!(worker_peak <= 4);
    assert!(rejected);
    assert_eq!(peak, 1);
}

#[tokio::test]
async fn retirement_releases_progress_for_another_worker() {
    use super::{segments::SegmentMap, worker};
    use tokio::sync::Mutex;
    let total = 4 * 1024 * 1024;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (rejected, _) = watch::channel(false);
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(
                stream,
                total,
                8,
                active.clone(),
                peak.clone(),
                rejected.clone(),
            ));
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("rget-retire-{}-{nonce}.part", std::process::id()));
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
        total,
        url: reqwest::Url::parse(&url).unwrap(),
    };
    let map = Arc::new(Mutex::new(SegmentMap::new(total, 2).unwrap()));
    let limiter = Arc::new(BandwidthLimiter::new());
    let (stop, _) = watch::channel(false);
    let (retire, rx) = watch::channel(false);
    let (_drain, drain_rx) = watch::channel(false);
    let context = worker::Context {
        client: reqwest::Client::new(),
        url: url.clone(),
        info: info.clone(),
        file: file.clone(),
        map: map.clone(),
        limiter: limiter.clone(),
        stop: stop.subscribe(),
        retire: rx,
        drain: drain_rx,
        desired: None,
        owner: 0,
    };
    let handle = tokio::spawn(worker::run(context));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if map.lock().await.records()[0].downloaded > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker did not start writing");
    retire.send_replace(true);
    assert!(matches!(
        handle.await.unwrap().unwrap(),
        worker::Exit::Retired
    ));
    let before = map.lock().await.records()[0].downloaded;
    assert!(before > 0 && before < total / 2);
    assert_eq!(map.lock().await.pending_count(), 2);
    let (_retire2, rx2) = watch::channel(false);
    let (_drain2, drain_rx2) = watch::channel(false);
    let context = worker::Context {
        client: reqwest::Client::new(),
        url,
        info,
        file: file.clone(),
        map: map.clone(),
        limiter,
        stop: stop.subscribe(),
        retire: rx2,
        drain: drain_rx2,
        desired: None,
        owner: 1,
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), worker::run(context))
        .await
        .unwrap()
        .unwrap();
    assert!(map.lock().await.is_complete());
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, total);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, b)| *b == (i as u64 % 251) as u8));
    std::fs::remove_file(path).unwrap();
    server.abort();
}

#[tokio::test]
async fn user_resize_changes_running_split_workers_and_preserves_bytes() {
    let total = 32 * 1024 * 1024;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (rejected, _) = watch::channel(false);
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(
                stream,
                total,
                32,
                active.clone(),
                peak.clone(),
                rejected.clone(),
            ));
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("rget-resize-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap(),
    );
    let map = Arc::new(tokio::sync::Mutex::new(SegmentMap::new(total, 2).unwrap()));
    let (stop, _) = watch::channel(false);
    let (desired, rx) = watch::channel(2u8);
    let info = ResumeInfo {
        etag: "\"v1\"".into(),
        total,
        url: reqwest::Url::parse(&url).unwrap(),
    };
    let inputs = adaptive::Inputs {
        client: reqwest::Client::new(),
        url,
        info,
        file: file.clone(),
        map: map.clone(),
        limiter: Arc::new(BandwidthLimiter::new()),
        stop,
        max: 2,
        desired: Some(rx),
        peak_workers: None,
    };
    let run = tokio::spawn(adaptive::supervise(inputs));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while map.lock().await.active_count() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("workers did not start");
    desired.send_replace(4);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while map.lock().await.active_count() < 4 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("workers did not increase");
    desired.send_replace(1);
    tokio::time::timeout(std::time::Duration::from_secs(30), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(map.lock().await.is_complete());
    drop(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, total);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, b)| *b == (i as u64 % 251) as u8));
    std::fs::remove_file(path).unwrap();
    server.abort();
}

#[tokio::test]
async fn user_drain_finishes_the_owned_segment_before_retiring() {
    use super::worker;
    let total = 4 * 1024 * 1024;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let (rejected, _) = watch::channel(false);
    let server = tokio::spawn(async move {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(
                stream,
                total,
                32,
                active.clone(),
                peak.clone(),
                rejected.clone(),
            ));
        }
    });
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("rget-drain-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap(),
    );
    let map = Arc::new(tokio::sync::Mutex::new(SegmentMap::new(total, 2).unwrap()));
    let (stop, _) = watch::channel(false);
    let (_retire, rx) = watch::channel(false);
    let (drain, drain_rx) = watch::channel(false);
    let info = ResumeInfo {
        etag: "\"v1\"".into(),
        total,
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
        retire: rx,
        drain: drain_rx,
        desired: None,
        owner: 0,
    };
    let handle = tokio::spawn(worker::run(context));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while map.lock().await.records()[0].downloaded == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drain.send_replace(true);
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        worker::Exit::Retired
    ));
    assert_eq!(map.lock().await.records()[0].downloaded, total / 2);
    drop(file);
    std::fs::remove_file(path).unwrap();
    server.abort();
}

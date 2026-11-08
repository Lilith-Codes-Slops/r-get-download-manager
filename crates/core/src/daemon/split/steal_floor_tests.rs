//! Verify latency floors against the real ownership map and reservations.
use super::segments::SegmentMap;

const MIB: u64 = 1024 * 1024;

#[test]
fn measured_latency_sets_minimum_transferred_range() {
    for (rtt, floor) in [(10.0, 2 * MIB), (50.0, 4 * MIB), (200.0, 8 * MIB)] {
        let mut map = SegmentMap::new(64 * MIB, 2).unwrap();
        map.note_rtt(rtt);
        let (_, _, original_end) = map.claim(0).unwrap();
        let (_, stolen_start, stolen_end) = map.steal(1).expect("eligible donor");
        assert_eq!(stolen_end, original_end);
        assert!(stolen_end - stolen_start >= floor);
        assert!(stolen_start >= MIB);
    }
}

#[test]
fn donor_at_floor_is_not_stolen_but_rejection_unblocks_it() {
    for status in [429, 503] {
        let mut map = SegmentMap::new(6 * MIB, 2).unwrap();
        map.note_rtt(250.0);
        let (index, start, _) = map.claim(0).unwrap();
        assert_eq!(map.stealable_slots(), 0);
        assert!(map.steal(1).is_none());
        assert_eq!(map.reserve_write(index, 0, start, 128).unwrap(), 128);
        map.note_rejection(status);
        assert!(map.stealable_slots() > 0);
        let (_, stolen_start, stolen_end) = map.steal(1).expect("rejection should relax the floor");
        assert!(stolen_start >= start + 128);
        assert!(stolen_end - stolen_start >= MIB);
        map.commit_write(index, 0, start, &[1; 128]).unwrap();
        assert_eq!(map.records()[0].downloaded, 128);
    }
}

#[test]
fn sharp_slowdown_temporarily_relaxes_floor() {
    let mut map = SegmentMap::new(6 * MIB, 2).unwrap();
    map.note_goodput(10.0 * MIB as f64);
    map.note_rtt(250.0);
    map.claim(0).unwrap();
    assert_eq!(map.stealable_slots(), 0);
    map.note_goodput(5.0 * MIB as f64);
    assert!(map.stealable_slots() > 0);
    assert!(map.steal(1).is_some());
}

#[tokio::test]
async fn live_response_latency_drives_the_floor() {
    use super::worker;
    use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
    use std::{sync::Arc, time::Duration};
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::{watch, Mutex}};
    let total = 6 * MIB;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new(); let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") { stream.read_exact(&mut byte).await.unwrap(); request.push(byte[0]); }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{total}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", total / 2, total / 2 - 1);
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(&[1]).await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rget-floor-{}-{nonce}.part", std::process::id()));
    let file = Arc::new(std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap());
    let map = Arc::new(Mutex::new(SegmentMap::new(total, 2).unwrap()));
    let (stop, _) = watch::channel(false); let (_retire, retire) = watch::channel(false);
    let (_drain, drain) = watch::channel(false);
    let info = ResumeInfo { etag: "\"v1\"".into(), total, url: reqwest::Url::parse(&url).unwrap() };
    let context = worker::Context { client: reqwest::Client::new(), url, info, file: file.clone(), map: map.clone(),
        limiter: Arc::new(BandwidthLimiter::new()), stop: stop.subscribe(), retire, drain, desired: None, owner: 0 };
    let handle = tokio::spawn(worker::run(context));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let guard = map.lock().await;
            if guard.active_count() == 1 && guard.policy().reason == "rtt" && guard.stealable_slots() == 0 { break; }
            drop(guard);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("worker response latency did not raise the floor");
    stop.send_replace(true);
    assert!(tokio::time::timeout(Duration::from_secs(5), handle).await.unwrap().unwrap().is_err());
    server.abort(); drop(file); std::fs::remove_file(path).unwrap();
}

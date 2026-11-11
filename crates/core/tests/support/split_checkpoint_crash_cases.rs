//! Kill a real daemon at each opt-in, deterministic split checkpoint boundary.
use super::*;

fn crash_at(point: &str) {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start_with_hook(Some(point));
    f.add(&server.url, 2);
    let marker = f.root.join("marker");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        let task = f.task(&server.url);
        assert!(!task.state.starts_with("failed:"), "{}", task.state);
        assert!(
            Instant::now() < deadline,
            "checkpoint hook {point} was not reached: {task:?}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    let seq: u64 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    let task = f.task(&server.url);
    let sidecar = f
        .root
        .join("Downloads")
        .join(format!("{}.meta", task.partial_name.unwrap()));
    let side: serde_json::Value = serde_json::from_slice(&std::fs::read(sidecar).unwrap()).unwrap();
    let conn = rusqlite::Connection::open(f.root.join("state/rget/queue.sqlite3")).unwrap();
    let json: String = conn
        .query_row(
            "SELECT info FROM rget_split_checkpoints_v2 WHERE url = ?1",
            [&server.url],
            |row| row.get(0),
        )
        .unwrap();
    let db: serde_json::Value = serde_json::from_str(&json).unwrap();
    let side_seq = side["sequence"].as_u64().unwrap();
    let db_seq = db["sequence"].as_u64().unwrap();
    match point {
        "data_synced" => {
            assert_eq!(side_seq + 1, seq);
            assert_eq!(db_seq + 1, seq);
        }
        "sidecar_renamed" | "sidecar_committed" => {
            assert_eq!(side_seq, seq);
            assert_eq!(db_seq + 1, seq);
        }
        "sqlite_committed" => {
            assert_eq!(side_seq, seq);
            assert_eq!(db_seq, seq);
        }
        _ => unreachable!(),
    }
    drop(conn);
    if point == "data_synced" {
        // While the checkpoint is deliberately held, workers must keep writing.
        server.release.store(true, Ordering::SeqCst);
        let partial = f
            .root
            .join("Downloads")
            .join(f.task(&server.url).partial_name.unwrap());
        let deadline = Instant::now() + Duration::from_secs(15);
        while std::fs::metadata(&partial).unwrap().len() < TOTAL {
            assert!(
                Instant::now() < deadline,
                "workers stalled behind checkpoint I/O"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    f.stop();
    server.release.store(true, Ordering::SeqCst);
    f.start();
    f.wait(&server.url, "completed");
    let bytes = std::fs::read(f.root.join("Downloads/split.bin")).unwrap();
    assert_eq!(bytes.len() as u64, TOTAL);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, byte)| *byte == (i as u64 % 251) as u8));
}

#[test]
fn kill_after_data_sync() {
    crash_at("data_synced");
}
#[test]
fn kill_after_sidecar_rename() {
    crash_at("sidecar_renamed");
}
#[test]
fn kill_after_sidecar_commit() {
    crash_at("sidecar_committed");
}
#[test]
fn kill_after_sqlite_commit() {
    crash_at("sqlite_committed");
}

#[test]
fn cancel_waits_for_a_blocking_positioned_write() {
    let server = HttpServer::new();
    let mut f = Fixture::new();
    f.start_with_hook(Some("write_block"));
    f.add(&server.url, 2);
    let marker = f.root.join("marker");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !marker.exists() {
        let task = f.task(&server.url);
        assert!(!task.state.starts_with("failed:"), "{}", task.state);
        assert!(
            Instant::now() < deadline,
            "write hook was not reached: {task:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let part = f.task(&server.url).partial_name.unwrap();
    let event = f.request(ClientRequest::Cancel {
        task_id: server.url.clone(),
    });
    assert!(
        matches!(event, DaemonEvent::TaskStateChanged { ref state, .. } if state == "canceling"),
        "{event:?}"
    );
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(f.task(&server.url).state, "canceling");
    assert!(f.root.join("Downloads").join(&part).exists());
    std::fs::write(marker.with_extension("release"), b"go").unwrap();
    f.wait(&server.url, "canceled");
    assert!(!f.root.join("Downloads").join(part).exists());
    assert!(!f.root.join("Downloads/split.bin").exists());
}

#[test]
fn server_reset_mid_range_never_publishes_a_partial_file() {
    use std::os::fd::AsRawFd;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let server = std::thread::spawn(move || {
        while flag.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut req = Vec::new();
                let mut byte = [0u8; 1];
                while !req.ends_with(b"\r\n\r\n") && req.len() < 16_384 {
                    if !matches!(stream.read(&mut byte), Ok(1)) {
                        return;
                    }
                    req.push(byte[0]);
                }
                let req = String::from_utf8_lossy(&req).to_ascii_lowercase();
                let range = req
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="));
                let Some(range) = range else {
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=reset.bin\r\nConnection: close\r\n\r\n");
                    let _ = stream.write_all(header.as_bytes());
                    return;
                };
                let Some((start, end)) = range.split_once('-') else {
                    return;
                };
                let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else {
                    return;
                };
                let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{TOTAL}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n", end-start+1);
                if stream.write_all(header.as_bytes()).is_err() {
                    return;
                }
                if start == 0 && end == 0 {
                    let _ = stream.write_all(&[0]);
                    return;
                }
                let _ = stream.write_all(&vec![7u8; 16 * 1024]);
                // Abortive close generates TCP RST rather than a clean EOF.
                let linger = libc::linger {
                    l_onoff: 1,
                    l_linger: 0,
                };
                unsafe {
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        &linger as *const _ as *const libc::c_void,
                        std::mem::size_of_val(&linger) as libc::socklen_t,
                    );
                }
            });
        }
    });
    let mut f = Fixture::new();
    f.start();
    f.add(&url, 2);
    f.wait(&url, "failed:");
    assert!(!f.root.join("Downloads/reset.bin").exists());
    running.store(false, Ordering::SeqCst);
    server.join().unwrap();
}

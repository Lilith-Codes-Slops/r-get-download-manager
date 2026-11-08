use super::*;
use crate::daemon::split::segments::SegmentMap;
const MIB: u64 = 1024 * 1024;

fn path() -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rget-host-memory-{}-{nonce}.sqlite3", std::process::id()))
}
fn observation(target: u64, bucket: Bucket, time: i64) -> Observation {
    Observation { memory: Remembered { target, bucket }, observed_ms: time }
}
#[test]
fn origins_normalize_and_isolate_scheme_and_port() {
    let key = |s| origin(&reqwest::Url::parse(s).unwrap()).unwrap();
    assert_eq!(key("https://Example.COM/a"), key("https://example.com:443/b"));
    assert_ne!(key("https://example.com/a"), key("http://example.com/a"));
    assert_ne!(key("https://example.com/a"), key("https://example.com:8443/a"));
    assert_eq!(key("http://[::1]:80/a"), key("http://[::1]/b"));
}

#[test]
fn stable_window_requires_real_samples_and_resets_after_congestion() {
    let start = Instant::now(); let mut stable = Stability::default();
    stable.success(start); assert!(stable.observation(start + STABLE, 8*MIB, Some(25.0)).is_none());
    stable.sample(Some(2.0 * MIB as f64), Some(25.0), false);
    stable.success(start); stable.success(start + STABLE - Duration::from_secs(1));
    assert!(stable.observation(start + STABLE - Duration::from_millis(1), 8*MIB, Some(25.0)).is_none());
    stable.success(start + STABLE);
    let observed = stable.observation(start + STABLE, 12*MIB, Some(25.0)).unwrap();
    assert_eq!(observed.memory, Remembered { target: 12*MIB, bucket: Bucket::Low });
    assert!(stable.observation(start + STABLE + FRESH + Duration::from_millis(1), 12*MIB, Some(25.0)).is_none());
    stable.sample(None, None, true);
    assert!(stable.observation(start + STABLE, 12*MIB, Some(25.0)).is_none());
    stable.sample(Some(0.0), Some(25.0), false);
    assert!(stable.observation(start + STABLE, 12*MIB, Some(25.0)).is_none());
}

#[tokio::test]
async fn second_task_and_restart_load_same_origin_unknown_host_uses_default() {
    let path = path(); let first = "https://example.test:443".to_string();
    let memory = Remembered { target: 12*MIB, bucket: Bucket::Medium };
    assert_eq!(load(path.clone(), first.clone()).await, None);
    let mut unknown = SegmentMap::new(64*MIB, 2).unwrap();
    let (i, start, _) = unknown.claim(0).unwrap();
    assert_eq!(unknown.begin_request(i, 0, start).unwrap() - start, 8*MIB);
    save(path.clone(), first.clone(), observation(memory.target, memory.bucket, wall_ms().unwrap())).await;
    // A fresh SQLite connection represents a later task, including after restart.
    let remembered = load(path.clone(), first.clone()).await.unwrap(); assert_eq!(remembered, memory);
    let mut next = SegmentMap::new(64*MIB, 2).unwrap(); next.seed_policy(remembered);
    assert_eq!(next.policy().reason, "remembered");
    let (i, start, _) = next.claim(0).unwrap();
    assert_eq!(next.begin_request(i, 0, start).unwrap() - start, 12*MIB);
    assert_eq!(load(path.clone(), "https://other.test:443".into()).await, None);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn invalid_rows_are_ignored_and_older_observations_cannot_overwrite_newer() {
    let path = path(); let key = "http://example.test:80".to_string(); let now = wall_ms().unwrap();
    save(path.clone(), key.clone(), observation(16*MIB, Bucket::Low, now)).await;
    save(path.clone(), key.clone(), observation(4*MIB, Bucket::High, now - 1)).await;
    assert_eq!(load(path.clone(), key.clone()).await.unwrap().target, 16*MIB);
    let conn = open(path.clone()).unwrap();
    conn.execute("UPDATE rget_host_sizing_v1 SET target = 0 WHERE origin = ?1", [&key]).unwrap();
    assert!(load(path.clone(), key.clone()).await.is_none());
    conn.execute("UPDATE rget_host_sizing_v1 SET target = ?1, bucket = 99 WHERE origin = ?2", params![8*MIB as i64, &key]).unwrap();
    assert!(load(path.clone(), key.clone()).await.is_none());
    conn.execute("UPDATE rget_host_sizing_v1 SET bucket = 0, observed_ms = ?1 WHERE origin = ?2", params![now + 1_000_000, &key]).unwrap();
    assert!(load(path.clone(), key.clone()).await.is_none());
    drop(conn);
    save(path.clone(), key.clone(), observation(4*MIB, Bucket::High, now + 1)).await;
    assert_eq!(load(path.clone(), key).await.unwrap().target, 4*MIB);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn writer_persists_only_eligible_observations_and_handles_unavailable_database() {
    let path = path(); let key = "https://example.test:443".to_string();
    let mut writer = Writer::new(path.clone(), Some(key.clone()));
    writer.consider(None, Instant::now()); writer.finish().await;
    assert!(load(path.clone(), key.clone()).await.is_none());
    let candidate = observation(6*MIB, Bucket::High, wall_ms().unwrap());
    writer.consider(Some(candidate), Instant::now()); writer.finish().await;
    assert_eq!(load(path.clone(), key).await, Some(candidate.memory));
    let missing_parent = std::env::temp_dir().join(format!("no-such-rget-directory-{}", wall_ms().unwrap())).join("db.sqlite");
    let mut failed = Writer::new(missing_parent, Some("https://example.test:443".into()));
    failed.consider(Some(candidate), Instant::now()); failed.finish().await;
    let _ = std::fs::remove_file(path);
}

#[test]
fn remembered_floor_stays_bounded_and_rejection_clears_stability() {
    let mut state = super::super::sizing_state::SizingState::remembered(Remembered { target: MIB, bucket: Bucket::High });
    assert_eq!(state.snapshot(Instant::now()).steal_floor, MIB);
    assert_eq!(state.snapshot(Instant::now()).reason, "remembered");
    let now = Instant::now();
    state.observe(Some(MIB as f64), Some(250.0), now);
    state.request_completed(now);
    state.reject(now + Duration::from_secs(1));
    state.request_completed(now + Duration::from_secs(20));
    assert!(state.good_memory(now + Duration::from_secs(20)).is_none());
}

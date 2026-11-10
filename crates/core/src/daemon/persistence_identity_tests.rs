use super::*;

#[test]
fn torrent_identity_and_seed_survive_queue_reload_without_http_recovery() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!("rget-torrent-state-{}-{nonce}", std::process::id()))
        .join("queue.sqlite3");
    let task: DownloadInfo = serde_json::from_str(r#"{"url":"torrent-unique","state":"downloading","filename":null,"downloaded":5,"total":10,"speed_bps":1,"final_path":null,"torrent":{"source":"magnet:?xt=test","info_hash":"abcdef","seed":true,"created_at":123,"stage_name":".rget-torrent-1","metrics":{"stalled":true,"stall_count":2,"current_stall_ms":123,"total_stall_ms":456}}}"#).unwrap();
    let mut state = SavedState::default();
    state.tasks.insert(task.url.clone(), task);
    save(&path, &state).unwrap();
    let restored = load(&path).unwrap();
    let task = &restored.tasks["torrent-unique"];
    let identity = task.torrent.as_ref().unwrap();
    assert!(identity.seed);
    let metrics = identity.metrics.as_ref().unwrap();
    assert!(!metrics.stalled);
    assert_eq!(metrics.current_stall_ms, 0);
    assert_eq!(metrics.stall_count, 2);
    assert_eq!(identity.info_hash.as_deref(), Some("abcdef"));
    assert_eq!(identity.stage_name.as_deref(), Some(".rget-torrent-1"));
    assert!(task.state.starts_with("failed: interrupted"));
    #[cfg(target_os = "linux")]
    assert!(!restored.prior_states.contains_key("torrent-unique"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

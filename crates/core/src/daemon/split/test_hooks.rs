//! Opt-in test-only fault injection. Never enable this feature in production.
use super::checkpoint::SplitRecord;

pub(super) async fn stop_at(point: &str, record: &SplitRecord) -> Result<(), String> {
    if std::env::var("RGET_TEST_CHECKPOINT_HOOK").ok().as_deref() != Some(point)
        || record.segments.iter().filter(|segment| segment.downloaded > 0).count() < 2 {
        return Ok(());
    }
    let marker = std::env::var_os("RGET_TEST_CHECKPOINT_MARKER").ok_or("test checkpoint marker is missing")?;
    std::fs::write(marker, record.sequence.to_string()).map_err(|e| e.to_string())?;
    // The integration test kills the daemon exactly here, then restarts it.
    std::future::pending::<()>().await;
    Ok(())
}

pub(super) fn block_after_write() -> std::io::Result<()> {
    let Some(marker) = std::env::var_os("RGET_TEST_WRITE_BLOCK") else { return Ok(()) };
    let marker = std::path::PathBuf::from(marker);
    std::fs::write(&marker, b"write completed; blocking task not joined")?;
    let release = marker.with_extension("release");
    while !release.exists() { std::thread::sleep(std::time::Duration::from_millis(10)); }
    Ok(())
}

/// Pause immediately after rename, before the containing directory is synced.
pub(super) fn stop_at_rename(record: &SplitRecord) -> std::io::Result<()> {
    if std::env::var("RGET_TEST_CHECKPOINT_HOOK").ok().as_deref() != Some("sidecar_renamed")
        || record.segments.iter().filter(|segment| segment.downloaded > 0).count() < 2 { return Ok(()); }
    let marker = std::env::var_os("RGET_TEST_CHECKPOINT_MARKER")
        .ok_or_else(|| std::io::Error::other("test checkpoint marker is missing"))?;
    std::fs::write(marker, record.sequence.to_string())?;
    loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
}

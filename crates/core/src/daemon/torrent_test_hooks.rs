//! Opt-in integration-test synchronization; excluded from normal builds.
use std::{io, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}, time::{Duration, Instant}};
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Publish a ready marker and wait for a release marker, allowing the test to
/// kill the actual daemon at a deterministic point. A timeout fails closed.
pub(super) fn pause(point: &str) -> io::Result<()> {
    if std::env::var("RGET_TORRENT_TEST_POINT").ok().as_deref() != Some(point) { return Ok(()); }
    let dir = PathBuf::from(std::env::var_os("RGET_TORRENT_TEST_DIR").ok_or_else(|| io::Error::other("test directory missing"))?);
    std::fs::write(dir.join(format!("ready-{}", NEXT.fetch_add(1, Ordering::Relaxed))), point)?;
    std::fs::write(dir.join("ready"), point)?;
    let deadline = Instant::now() + Duration::from_secs(90);
    while !dir.join("release").exists() {
        if Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "test hook timed out")); }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Record the result of an actual gated-storage call on the closed live gate.
pub(super) fn late_write(rejected: bool) -> io::Result<()> {
    if std::env::var("RGET_TORRENT_TEST_POINT").ok().as_deref() != Some("after-barrier") { return Ok(()); }
    if !rejected { return Err(io::Error::other("late storage write reached delegate")); }
    let dir = PathBuf::from(std::env::var_os("RGET_TORRENT_TEST_DIR").ok_or_else(|| io::Error::other("test directory missing"))?);
    std::fs::write(dir.join("late-write-rejected"), b"rejected")
}

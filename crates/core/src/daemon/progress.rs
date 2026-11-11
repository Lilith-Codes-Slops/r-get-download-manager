//! Consistent per-attempt aggregate telemetry for all transfer paths.
use downloader_ipc::DownloadInfo;
use std::time::{Duration, Instant};

pub(super) struct ProgressMeter {
    initial: u64,
    started: Instant,
}
impl ProgressMeter {
    pub fn new(initial: u64) -> Self {
        Self {
            initial,
            started: Instant::now(),
        }
    }
    pub fn update(&self, task: &mut DownloadInfo, downloaded: u64) {
        task.downloaded = downloaded;
        task.speed_bps = speed(downloaded, self.initial, self.started.elapsed());
    }
}
fn speed(downloaded: u64, initial: u64, elapsed: Duration) -> u64 {
    (downloaded.saturating_sub(initial) as f64 / elapsed.as_secs_f64().max(0.001)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovered_bytes_do_not_inflate_transfer_rate() {
        assert_eq!(speed(1_000_100, 1_000_000, Duration::from_secs(2)), 50);
        assert_eq!(speed(900, 1000, Duration::from_secs(1)), 0);
    }
}

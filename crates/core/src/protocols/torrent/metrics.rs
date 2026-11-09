//! Per-attempt observations; missing engine counters remain unknown.
use super::stall_detector::{StallDetector, StallEvent};
use downloader_ipc::TorrentMetrics;
use std::time::{Duration, Instant};

pub struct MetricsTracker {
    known_at: Instant,
    first_piece_at: Option<Instant>,
    detector: StallDetector,
    pub snapshot: TorrentMetrics,
}

pub fn millis(duration: Duration) -> u64 { duration.as_millis().min(u64::MAX as u128) as u64 }

impl MetricsTracker {
    pub fn new(metadata_elapsed: Duration, known_at: Instant, threshold: Duration) -> Self {
        Self { known_at, first_piece_at: None, detector: StallDetector::new(threshold),
            snapshot: TorrentMetrics { metadata_fetch_ms: Some(millis(metadata_elapsed)), ..Default::default() } }
    }

    /// Incoming bytes may include pieces later rejected; only a verified piece
    /// counter establishes time-to-first-piece. All timestamps are sampled.
    pub fn observe(&mut self, now: Instant, active: bool, fetched: Option<u64>, verified_pieces: Option<u64>, finished: bool) -> Option<(StallEvent, u64)> {
        if self.first_piece_at.is_none() && verified_pieces.is_some_and(|n| n > 0) {
            self.first_piece_at = Some(now);
            self.snapshot.time_to_first_piece_ms = Some(millis(now.saturating_duration_since(self.known_at)));
        }
        if finished && self.snapshot.first_piece_to_finished_ms.is_none() {
            if let Some(first) = self.first_piece_at { self.snapshot.first_piece_to_finished_ms = Some(millis(now.saturating_duration_since(first))); }
        }
        let event = self.detector.sample(now, active, fetched);
        if finished { self.detector.sample(now, false, None); }
        self.snapshot.stalled = self.detector.is_stalled();
        self.snapshot.stall_count = self.detector.count;
        self.snapshot.current_stall_ms = millis(self.detector.current_duration(now));
        self.snapshot.total_stall_ms = millis(self.detector.total_duration(now));
        event.map(|event| (event, if event == StallEvent::Recovered { millis(self.detector.last_episode()) } else { 0 }))
    }

    pub fn needs_recovery(&self) -> bool { self.detector.needs_recovery() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracker_emits_stall_then_recovery() {
        let start = Instant::now(); let mut tracker = MetricsTracker::new(Duration::ZERO, start, Duration::from_secs(2));
        assert_eq!(tracker.observe(start, true, Some(4), Some(0), false), None);
        assert_eq!(tracker.observe(start + Duration::from_secs(2), true, Some(4), Some(0), false), Some((StallEvent::Stalled, 0)));
        assert_eq!(tracker.observe(start + Duration::from_secs(3), true, Some(8), Some(1), false), Some((StallEvent::Recovered, 3000)));
        assert_eq!(tracker.snapshot.stall_count, 1);
        assert!(!tracker.snapshot.stalled);
    }

    #[test]
    fn first_piece_uses_verified_count_and_unknown_metrics_remain_unknown() {
        let start = Instant::now(); let mut tracker = MetricsTracker::new(Duration::from_millis(50), start, Duration::from_secs(2));
        tracker.observe(start, true, Some(100), Some(0), false);
        assert_eq!(tracker.snapshot.time_to_first_piece_ms, None);
        tracker.observe(start + Duration::from_secs(1), true, Some(200), Some(1), false);
        assert_eq!(tracker.snapshot.time_to_first_piece_ms, Some(1000));
        tracker.observe(start + Duration::from_secs(3), true, Some(300), Some(1), true);
        assert_eq!(tracker.snapshot.first_piece_to_finished_ms, Some(2000));
        assert_eq!(tracker.snapshot.duplicate_bytes, None);
        assert_eq!(tracker.snapshot.request_timeouts, None);
        assert_eq!(tracker.snapshot.peers_dropped, None);
    }
}

//! Observe incoming payload activity; this does not control peers or requests.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallEvent {
    Stalled,
    Recovered,
}

pub struct StallDetector {
    threshold: Duration,
    last_bytes: Option<u64>,
    last_progress: Option<Instant>,
    stalled_since: Option<Instant>,
    pub count: u32,
    completed_stall: Duration,
    pending_recovery: bool,
    pending_bytes: Option<u64>,
    last_episode: Duration,
}

impl StallDetector {
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            last_bytes: None,
            last_progress: None,
            stalled_since: None,
            count: 0,
            completed_stall: Duration::ZERO,
            pending_recovery: false,
            pending_bytes: None,
            last_episode: Duration::ZERO,
        }
    }

    /// `active` means live, downloading and at least one connected peer.
    /// `bytes` is librqbit's fetched payload count, not verified file progress.
    /// No-peer, unknown and paused samples reset the baseline without declaring
    /// a recovery. A counter reset establishes a new baseline as well.
    pub fn sample(&mut self, now: Instant, active: bool, bytes: Option<u64>) -> Option<StallEvent> {
        let Some(bytes) = bytes.filter(|_| active) else {
            if self.stalled_since.is_some() {
                self.pending_recovery = true;
                self.pending_bytes = self.last_bytes;
            }
            self.finish_stall(now);
            self.last_bytes = None;
            self.last_progress = None;
            return None;
        };
        let Some(previous) = self.last_bytes.replace(bytes) else {
            self.last_progress = Some(now);
            if self.pending_recovery && self.pending_bytes.is_some_and(|baseline| bytes > baseline)
            {
                self.pending_recovery = false;
                self.pending_bytes = None;
                return Some(StallEvent::Recovered);
            }
            if self.pending_bytes.is_some_and(|baseline| bytes < baseline) {
                self.pending_recovery = false;
                self.pending_bytes = None;
            }
            return None;
        };
        if bytes < previous {
            self.finish_stall(now);
            self.pending_recovery = false;
            self.pending_bytes = None;
            self.last_progress = Some(now);
            return None;
        }
        if bytes > previous {
            self.last_progress = Some(now);
            if self.stalled_since.is_some() {
                self.finish_stall(now);
                self.pending_recovery = false;
                self.pending_bytes = None;
                return Some(StallEvent::Recovered);
            }
            if self.pending_recovery && self.pending_bytes.is_some_and(|baseline| bytes > baseline)
            {
                self.pending_recovery = false;
                self.pending_bytes = None;
                return Some(StallEvent::Recovered);
            }
            return None;
        }
        let baseline = self.last_progress.get_or_insert(now);
        if self.stalled_since.is_none()
            && now.saturating_duration_since(*baseline) >= self.threshold
        {
            self.stalled_since = Some(*baseline);
            self.pending_recovery = false;
            self.pending_bytes = None;
            self.count = self.count.saturating_add(1);
            return Some(StallEvent::Stalled);
        }
        None
    }

    fn finish_stall(&mut self, now: Instant) {
        if let Some(start) = self.stalled_since.take() {
            self.last_episode = now.saturating_duration_since(start);
            self.completed_stall = self.completed_stall.saturating_add(self.last_episode);
        }
    }

    pub fn needs_recovery(&self) -> bool {
        self.stalled_since.is_some() || self.pending_recovery
    }
    pub fn last_episode(&self) -> Duration {
        self.last_episode
    }
    pub fn is_stalled(&self) -> bool {
        self.stalled_since.is_some()
    }
    pub fn current_duration(&self, now: Instant) -> Duration {
        self.stalled_since
            .map(|start| now.saturating_duration_since(start))
            .unwrap_or_default()
    }
    pub fn total_duration(&self, now: Instant) -> Duration {
        self.completed_stall
            .saturating_add(self.current_duration(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_stall_then_recovery_and_accumulated_duration() {
        let start = Instant::now();
        let mut d = StallDetector::new(Duration::from_secs(30));
        assert_eq!(d.sample(start, true, Some(0)), None);
        assert_eq!(
            d.sample(start + Duration::from_secs(29), true, Some(0)),
            None
        );
        assert_eq!(
            d.sample(start + Duration::from_secs(30), true, Some(0)),
            Some(StallEvent::Stalled)
        );
        assert_eq!(
            d.sample(start + Duration::from_secs(31), true, Some(0)),
            None
        );
        assert_eq!(
            d.sample(start + Duration::from_secs(32), true, Some(10)),
            Some(StallEvent::Recovered)
        );
        assert_eq!(d.count, 1);
        assert_eq!(
            d.total_duration(start + Duration::from_secs(32)),
            Duration::from_secs(32)
        );
    }
    #[test]
    fn no_peers_pause_unknown_and_counter_reset_do_not_create_false_stalls() {
        let start = Instant::now();
        let mut d = StallDetector::new(Duration::from_secs(2));
        d.sample(start, true, Some(100));
        d.sample(start + Duration::from_secs(1), false, Some(100));
        assert_eq!(
            d.sample(start + Duration::from_secs(50), true, Some(100)),
            None
        );
        assert_eq!(
            d.sample(start + Duration::from_secs(53), true, Some(10)),
            None
        );
        assert_eq!(d.sample(start + Duration::from_secs(54), true, None), None);
        assert_eq!(
            d.sample(start + Duration::from_secs(80), true, Some(10)),
            None
        );
        assert_eq!(d.count, 0);
    }
    #[test]
    fn recovery_after_peer_loss_waits_for_actual_new_bytes() {
        let start = Instant::now();
        let mut d = StallDetector::new(Duration::from_secs(1));
        d.sample(start, true, Some(10));
        d.sample(start + Duration::from_secs(1), true, Some(10));
        assert_eq!(d.sample(start + Duration::from_secs(2), false, None), None);
        assert_eq!(
            d.sample(start + Duration::from_secs(3), true, Some(10)),
            None
        );
        assert_eq!(
            d.sample(start + Duration::from_millis(3500), true, Some(11)),
            Some(StallEvent::Recovered)
        );
        assert_eq!(d.last_episode(), Duration::from_secs(2));
    }
    #[test]
    fn interrupted_stall_closes_without_false_recovery() {
        let start = Instant::now();
        let mut d = StallDetector::new(Duration::from_secs(1));
        d.sample(start, true, Some(0));
        d.sample(start + Duration::from_secs(1), true, Some(0));
        assert_eq!(d.sample(start + Duration::from_secs(3), false, None), None);
        assert!(!d.is_stalled());
        assert_eq!(
            d.total_duration(start + Duration::from_secs(4)),
            Duration::from_secs(3)
        );
    }
}

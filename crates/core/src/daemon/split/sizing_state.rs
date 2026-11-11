//! Per-task coupling for the sizing policy, steal floor and congestion cooldown.
use super::{
    adaptive_sizer::{AdaptiveSizer, Reason},
    steal_floor::StealFloor,
};
use std::time::{Duration, Instant};

const COOLDOWN: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon) struct PolicySnapshot {
    pub target_size: u64,
    pub steal_floor: u64,
    pub reason: &'static str,
}

pub(super) struct SizingState {
    sizer: AdaptiveSizer,
    floor: StealFloor,
    cooldown_until: Option<Instant>,
    floor_cap: Option<u64>,
    last_reason: &'static str,
    stability: super::host_memory::Stability,
}
impl Default for SizingState {
    fn default() -> Self {
        Self {
            sizer: AdaptiveSizer::new(None),
            floor: StealFloor::default(),
            cooldown_until: None,
            floor_cap: None,
            last_reason: "initial",
            stability: super::host_memory::Stability::default(),
        }
    }
}
impl SizingState {
    pub fn remembered(memory: super::host_memory::Remembered) -> Self {
        let mut state = Self {
            sizer: AdaptiveSizer::new(Some(memory.target)),
            ..Default::default()
        };
        let rtt = memory.bucket.representative_ms();
        state.sizer.recent_rtt_ms = Some(rtt);
        state.floor.observe_latency(Some(rtt), true);
        let cap = (memory.target / 2).max(super::adaptive_sizer::MIN_TARGET);
        state.floor.limit(cap);
        state.floor_cap = Some(cap);
        state.last_reason = "remembered";
        state
    }
    pub fn reset_stability(&mut self) {
        self.stability.reset();
    }
    pub fn request_completed(&mut self, now: Instant) {
        self.stability.success(now);
    }
    pub fn good_memory(&self, now: Instant) -> Option<super::host_memory::Observation> {
        self.stability
            .observation(now, self.sizer.target, self.sizer.recent_rtt_ms)
    }
    fn cooling(&self, now: Instant) -> bool {
        self.cooldown_until.is_some_and(|until| now < until)
    }
    fn couple(&mut self, old: u64) {
        let target = self.sizer.target;
        if target < old {
            self.floor_cap = Some((target / 2).max(super::adaptive_sizer::MIN_TARGET));
            self.floor.target_reduced(target);
        }
        if target > old {
            self.floor_cap =
                Some((target / 4).clamp(super::adaptive_sizer::MIN_TARGET, 8 * 1024 * 1024));
            self.floor.target_increased(target);
        }
    }
    pub fn observe(&mut self, goodput: Option<f64>, latency: Option<f64>, now: Instant) {
        let before = self.snapshot(now);
        let allow_growth = !self.cooling(now);
        let old = self.sizer.target;
        let slowdown = self
            .sizer
            .observe_with_growth(goodput, latency, allow_growth);
        if latency.is_some() {
            self.floor
                .observe_latency(self.sizer.recent_rtt_ms, allow_growth);
        }
        self.couple(old);
        if let Some(cap) = self.floor_cap {
            self.floor.limit(cap);
        }
        if slowdown || self.sizer.target < old {
            self.stability.reset();
        }
        self.stability.sample(goodput, latency, slowdown);
        if slowdown {
            self.congestion(now);
            self.last_reason = "slowdown";
        } else if self.sizer.target != old {
            self.last_reason = match self.sizer.last_change_reason {
                Reason::Goodput => "goodput",
                Reason::Rtt => "rtt",
                Reason::Rejection => "rejection",
                Reason::Slowdown => "slowdown",
                Reason::Initial => "initial",
                Reason::Remembered => "remembered",
            };
        } else if latency.is_some() && self.current_floor(now) != before.steal_floor {
            self.last_reason = "rtt";
        }
    }
    pub fn reject(&mut self, now: Instant) {
        let old = self.sizer.target;
        self.sizer.reject();
        self.stability.reset();
        self.couple(old);
        self.congestion(now);
        self.last_reason = "rejection";
    }
    pub fn snapshot(&self, now: Instant) -> PolicySnapshot {
        PolicySnapshot {
            target_size: self.sizer.target,
            steal_floor: self.current_floor(now),
            reason: self.last_reason,
        }
    }
    fn congestion(&mut self, now: Instant) {
        // Apply even at the target minimum, where the target cannot shrink.
        self.floor.target_reduced(self.sizer.target);
        self.floor_cap = Some((self.sizer.target / 2).max(super::adaptive_sizer::MIN_TARGET));
        self.floor.relax();
        self.cooldown_until = Some(now + COOLDOWN);
    }
    pub fn current_floor(&self, now: Instant) -> u64 {
        self.floor.current(self.cooling(now))
    }
    pub fn decide_floor(&mut self, now: Instant) -> u64 {
        let cooling = self.cooling(now);
        self.floor.decision(cooling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MIB: u64 = 1024 * 1024;
    #[test]
    fn target_growth_and_reduction_update_the_floor() {
        let now = Instant::now();
        let mut state = SizingState::default();
        state.observe(None, Some(30.0), now);
        assert_eq!(state.current_floor(now), 2 * MIB);
        state.observe(Some(2.0 * MIB as f64), None, now);
        assert_eq!(state.sizer.target, 12 * MIB);
        assert_eq!(state.current_floor(now), 3 * MIB);
        let mut high = SizingState::default();
        high.observe(None, Some(250.0), now);
        assert_eq!(high.sizer.target, 8 * MIB * 7 / 10);
        assert_eq!(high.current_floor(now), (8 * MIB * 7 / 10) / 2);
    }
    #[test]
    fn latency_bucket_change_respects_the_latest_coupled_cap() {
        let now = Instant::now();
        let mut state = SizingState::default();
        state.observe(None, Some(250.0), now);
        let cap = state.sizer.target / 2;
        state.observe(None, Some(15.0), now + Duration::from_secs(1));
        assert!(state.current_floor(now + Duration::from_secs(1)) <= cap);
    }
    #[test]
    fn rejection_shrinks_both_and_prevents_growth_until_cooldown() {
        let now = Instant::now();
        let mut state = SizingState::default();
        state.observe(None, Some(30.0), now);
        state.reject(now);
        assert_eq!(state.sizer.target, 4 * MIB);
        for _ in 0..4 {
            assert_eq!(state.decide_floor(now), MIB);
        }
        state.observe(
            Some(4.0 * MIB as f64),
            Some(30.0),
            now + Duration::from_secs(11),
        );
        assert_eq!(state.sizer.target, 4 * MIB);
        assert_eq!(state.current_floor(now + Duration::from_secs(11)), MIB);
        state.observe(Some(4.0 * MIB as f64), Some(30.0), now + COOLDOWN);
        assert_eq!(state.sizer.target, 6 * MIB);
        assert_eq!(state.current_floor(now + COOLDOWN), 6 * MIB / 4);
    }
    #[test]
    fn slowdown_refreshes_cooldown_and_floor_never_drops_below_one_mib() {
        let now = Instant::now();
        let mut state = SizingState::default();
        state.observe(Some(10.0 * MIB as f64), None, now);
        state.observe(Some(5.0 * MIB as f64), None, now + Duration::from_secs(1));
        assert_eq!(state.sizer.target, 4 * MIB);
        assert_eq!(state.current_floor(now + Duration::from_secs(1)), MIB);
        state.reject(now + Duration::from_secs(5));
        for _ in 0..8 {
            state.decide_floor(now + Duration::from_secs(5));
        }
        assert_eq!(state.current_floor(now + Duration::from_secs(16)), MIB);
        assert!(state.current_floor(now + Duration::from_secs(17)) >= MIB);
    }
    #[test]
    fn growth_floor_is_capped_at_eight_mib() {
        let now = Instant::now();
        let mut state = SizingState::default();
        state.observe(None, Some(30.0), now);
        for _ in 0..30 {
            state.observe(Some(4.0 * MIB as f64), None, now);
        }
        assert_eq!(state.sizer.target, 128 * MIB);
        assert_eq!(state.current_floor(now), 8 * MIB);
    }
    #[test]
    fn rtt_floor_two_mib_rule_cannot_increase_target_during_cooldown() {
        let now = Instant::now();
        let mut state = SizingState::default();
        for _ in 0..5 {
            state.reject(now);
        }
        state.observe(None, Some(250.0), now + Duration::from_secs(1));
        assert_eq!(state.sizer.target, MIB);
        assert_eq!(state.current_floor(now + Duration::from_secs(1)), MIB);
    }
}

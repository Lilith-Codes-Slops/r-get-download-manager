//! Pure sizing policy; optional host memory seeds the initial target and RTT bucket.
const MIB: u64 = 1024 * 1024;
pub(super) const MIN_TARGET: u64 = MIB;
pub(super) const MAX_TARGET: u64 = 128 * MIB;
pub(super) const INITIAL_TARGET: u64 = 8 * MIB;
// A concrete threshold for the otherwise unspecified term "high goodput".
const HIGH_GOODPUT: f64 = MIB as f64;
const EMA_WEIGHT: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason { Initial, Remembered, Rejection, Slowdown, Goodput, Rtt }

#[derive(Debug, Clone)]
pub(super) struct AdaptiveSizer {
    pub target: u64,
    pub recent_goodput: Option<f64>,
    pub recent_rtt_ms: Option<f64>,
    pub last_change_reason: Reason,
    recent_peak: f64,
}

impl AdaptiveSizer {
    pub fn new(remembered_target: Option<u64>) -> Self {
        let target = remembered_target.unwrap_or(INITIAL_TARGET).clamp(MIN_TARGET, MAX_TARGET);
        Self { target, recent_goodput: None, recent_rtt_ms: None,
            last_change_reason: if remembered_target.is_some() { Reason::Remembered } else { Reason::Initial }, recent_peak: 0.0 }
    }

    /// Accept only finite, nonnegative samples; no measurement is different from zero throughput.
    /// Call on segment completion or a measurement interval; rejections use `reject` immediately.
    #[cfg(test)]
    pub fn observe(&mut self, goodput_bps: Option<f64>, rtt_ms: Option<f64>) -> bool {
        self.observe_with_growth(goodput_bps, rtt_ms, true)
    }
    pub fn observe_with_growth(&mut self, goodput_bps: Option<f64>, rtt_ms: Option<f64>, allow_growth: bool) -> bool {
        let goodput = goodput_bps.filter(|v| v.is_finite() && *v >= 0.0);
        let rtt = rtt_ms.filter(|v| v.is_finite() && *v >= 0.0);
        if let Some(value) = rtt { self.recent_rtt_ms = Some(ema(self.recent_rtt_ms, value)); }
        let slowdown = goodput.is_some_and(|value| self.recent_peak > 0.0 && value < self.recent_peak * 0.6);
        if let Some(value) = goodput {
            self.recent_goodput = Some(ema(self.recent_goodput, value));
            self.recent_peak = self.recent_peak.max(value);
        }
        if slowdown {
            self.set(self.target / 2, Reason::Slowdown);
            // Do not repeatedly halve for one historical spike.
            self.recent_peak = goodput.unwrap_or(0.0);
        } else if allow_growth && goodput.is_some() && self.recent_goodput.is_some_and(|v| v >= HIGH_GOODPUT)
            && self.recent_rtt_ms.is_some_and(|v| v < 80.0) {
            self.set(self.target.saturating_mul(3) / 2, Reason::Goodput);
        } else if (goodput.is_some() || rtt.is_some()) && self.recent_rtt_ms.is_some_and(|v| v > 200.0) {
            let target = (self.target * 7 / 10).max(2 * MIB);
            if allow_growth || target <= self.target { self.set(target, Reason::Rtt); }
        }
        slowdown
    }

    pub fn reject(&mut self) {
        self.set(self.target / 2, Reason::Rejection);
        self.recent_peak = 0.0;
    }

    fn set(&mut self, value: u64, reason: Reason) {
        let target = value.clamp(MIN_TARGET, MAX_TARGET);
        if target != self.target { self.target = target; self.last_change_reason = reason; }
    }
}

fn ema(previous: Option<f64>, sample: f64) -> f64 {
    previous.map_or(sample, |old| old * (1.0 - EMA_WEIGHT) + sample * EMA_WEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_eight_and_clamps_remembered_values() {
        assert_eq!(AdaptiveSizer::new(None).target, 8 * MIB);
        assert_eq!(AdaptiveSizer::new(Some(0)).target, MIN_TARGET);
        assert_eq!(AdaptiveSizer::new(Some(u64::MAX)).target, MAX_TARGET);
    }

    #[test]
    fn goodput_grows_slow_rtt_reduces_and_rejection_halves() {
        let mut sizer = AdaptiveSizer::new(None);
        sizer.observe(Some(2.0 * MIB as f64), Some(30.0));
        assert_eq!(sizer.target, 12 * MIB);
        assert_eq!(sizer.last_change_reason, Reason::Goodput);
        sizer.reject();
        assert_eq!(sizer.target, 6 * MIB);
        assert_eq!(sizer.last_change_reason, Reason::Rejection);
        let mut high_rtt = AdaptiveSizer::new(None);
        high_rtt.observe(None, Some(250.0));
        assert_eq!(high_rtt.target, 8 * MIB * 7 / 10);
        assert_eq!(high_rtt.last_change_reason, Reason::Rtt);
        let mut floor = AdaptiveSizer::new(Some(MIN_TARGET));
        floor.observe(None, Some(250.0));
        assert_eq!(floor.target, 2 * MIB);
    }

    #[test]
    fn slowdown_is_immediate_and_not_repeated_for_one_spike() {
        let mut sizer = AdaptiveSizer::new(None);
        sizer.observe(Some(10.0 * MIB as f64), None);
        sizer.observe(Some(5.0 * MIB as f64), None);
        assert_eq!(sizer.target, 4 * MIB);
        assert_eq!(sizer.last_change_reason, Reason::Slowdown);
        sizer.observe(Some(5.0 * MIB as f64), None);
        assert_eq!(sizer.target, 4 * MIB);
    }

    #[test]
    fn invalid_samples_are_ignored_and_bounds_are_respected() {
        let mut sizer = AdaptiveSizer::new(None);
        sizer.observe(Some(f64::NAN), Some(f64::INFINITY));
        assert_eq!(sizer.target, INITIAL_TARGET);
        for _ in 0..100 { sizer.reject(); }
        assert_eq!(sizer.target, MIN_TARGET);
        for _ in 0..100 { sizer.observe(Some(2.0 * MIB as f64), Some(30.0)); }
        assert_eq!(sizer.target, MAX_TARGET);
    }
}

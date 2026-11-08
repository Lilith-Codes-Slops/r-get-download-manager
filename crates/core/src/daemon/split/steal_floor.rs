//! Per-task latency-based steal threshold and temporary congestion override.
use super::adaptive_sizer::MIN_TARGET;

const MIB: u64 = 1024 * 1024;
const RELAXED_DECISIONS: u8 = 4;

#[derive(Default)]
pub(super) struct StealFloor { relaxed: u8 }
impl StealFloor {
    /// With no latency observation, preserve the legacy 1 MiB threshold.
    pub fn current(&self, rtt_ms: Option<f64>) -> u64 {
        if self.relaxed > 0 { return MIN_TARGET; }
        match rtt_ms {
            Some(rtt) if rtt.is_finite() && rtt >= 0.0 && rtt < 50.0 => 2 * MIB,
            Some(rtt) if rtt.is_finite() && rtt >= 0.0 && rtt < 200.0 => 4 * MIB,
            Some(rtt) if rtt.is_finite() && rtt >= 0.0 => 8 * MIB,
            _ => MIN_TARGET,
        }.max(MIN_TARGET)
    }
    pub fn relax(&mut self) { self.relaxed = RELAXED_DECISIONS; }
    /// Inspecting capacity is not a decision; consume only on a steal attempt.
    pub fn decision(&mut self, rtt_ms: Option<f64>) -> u64 {
        let floor = self.current(rtt_ms);
        self.relaxed = self.relaxed.saturating_sub(1);
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_and_unknown_latency() {
        let floor = StealFloor::default();
        assert_eq!(floor.current(None), MIB);
        assert_eq!(floor.current(Some(0.0)), 2 * MIB);
        assert_eq!(floor.current(Some(49.99)), 2 * MIB);
        assert_eq!(floor.current(Some(50.0)), 4 * MIB);
        assert_eq!(floor.current(Some(199.99)), 4 * MIB);
        assert_eq!(floor.current(Some(200.0)), 8 * MIB);
        assert_eq!(floor.current(Some(f64::NAN)), MIB);
    }
    #[test]
    fn congestion_temporarily_relaxes_and_refreshes_budget() {
        let mut floor = StealFloor::default();
        floor.relax();
        assert_eq!(floor.current(Some(300.0)), MIB);
        for _ in 0..RELAXED_DECISIONS { assert_eq!(floor.decision(Some(300.0)), MIB); }
        assert_eq!(floor.current(Some(300.0)), 8 * MIB);
        floor.relax(); floor.decision(Some(300.0)); floor.relax();
        for _ in 0..RELAXED_DECISIONS { assert_eq!(floor.decision(Some(300.0)), MIB); }
        assert_eq!(floor.current(Some(300.0)), 8 * MIB);
    }
}

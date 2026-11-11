//! Latency floor with target-size coupling and a temporary congestion override.
use super::adaptive_sizer::MIN_TARGET;

const MIB: u64 = 1024 * 1024;
const RELAXED_DECISIONS: u8 = 4;

pub(super) struct StealFloor {
    base: u64,
    latency_bucket: u64,
    relaxed: u8,
}
impl Default for StealFloor {
    fn default() -> Self {
        Self {
            base: MIN_TARGET,
            latency_bucket: MIN_TARGET,
            relaxed: 0,
        }
    }
}
impl StealFloor {
    fn bucket(rtt_ms: Option<f64>) -> u64 {
        match rtt_ms {
            Some(rtt) if rtt.is_finite() && (0.0..50.0).contains(&rtt) => 2 * MIB,
            Some(rtt) if rtt.is_finite() && (0.0..200.0).contains(&rtt) => 4 * MIB,
            Some(rtt) if rtt.is_finite() && rtt >= 0.0 => 8 * MIB,
            _ => MIN_TARGET,
        }
    }
    /// A bucket change establishes a new latency floor; target changes can then
    /// override it. Defer any floor growth while the cooldown is in force.
    pub fn observe_latency(&mut self, rtt_ms: Option<f64>, allow_growth: bool) {
        let bucket = Self::bucket(rtt_ms);
        if bucket != self.latency_bucket && (allow_growth || bucket <= self.base) {
            self.base = bucket;
            self.latency_bucket = bucket;
        }
    }
    pub fn limit(&mut self, cap: u64) {
        self.base = self.base.min(cap.max(MIN_TARGET));
    }
    pub fn target_reduced(&mut self, target: u64) {
        self.base = self.base.min((target / 2).max(MIN_TARGET));
    }
    pub fn target_increased(&mut self, target: u64) {
        self.base = (target / 4).clamp(MIN_TARGET, 8 * MIB);
    }
    pub fn current(&self, cooling: bool) -> u64 {
        if cooling || self.relaxed > 0 {
            MIN_TARGET
        } else {
            self.base
        }
    }
    pub fn relax(&mut self) {
        self.relaxed = RELAXED_DECISIONS;
    }
    /// Inspecting capacity is not a decision; consume only on a steal attempt.
    pub fn decision(&mut self, cooling: bool) -> u64 {
        let floor = self.current(cooling);
        self.relaxed = self.relaxed.saturating_sub(1);
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_and_unknown_latency() {
        for (rtt, expected) in [
            (None, MIB),
            (Some(0.0), 2 * MIB),
            (Some(49.99), 2 * MIB),
            (Some(50.0), 4 * MIB),
            (Some(199.99), 4 * MIB),
            (Some(200.0), 8 * MIB),
            (Some(f64::NAN), MIB),
        ] {
            let mut floor = StealFloor::default();
            floor.observe_latency(rtt, true);
            assert_eq!(floor.current(false), expected);
        }
    }
    #[test]
    fn congestion_temporarily_relaxes_and_refreshes_budget() {
        let mut floor = StealFloor::default();
        floor.observe_latency(Some(300.0), true);
        floor.relax();
        for _ in 0..RELAXED_DECISIONS {
            assert_eq!(floor.decision(false), MIB);
        }
        assert_eq!(floor.current(false), 8 * MIB);
        floor.relax();
        floor.decision(false);
        floor.relax();
        for _ in 0..RELAXED_DECISIONS {
            assert_eq!(floor.decision(false), MIB);
        }
        assert_eq!(floor.current(false), 8 * MIB);
    }
}

//! Fixed-step scheduling that is independent of paint frequency or focus.
use crate::simulation::TICK_INTERVAL;
use std::time::{Duration, Instant};

pub struct SimulationClock {
    next_tick: Instant,
}
impl SimulationClock {
    /// Match the reference's initial tick followed by a 500 ms interval.
    pub fn new(now: Instant) -> Self {
        Self { next_tick: now }
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next_tick
    }
    pub fn advance(&mut self) {
        self.next_tick += TICK_INTERVAL;
    }
    pub fn retry_after_error(&mut self, now: Instant) {
        self.next_tick = now + TICK_INTERVAL;
    }
    pub fn delay(&self, now: Instant) -> Duration {
        self.next_tick.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_tick_fixed_cadence_catches_up_and_retries() {
        let start = Instant::now();
        let mut clock = SimulationClock::new(start);
        assert!(clock.due(start));
        clock.advance();
        assert_eq!(clock.delay(start), Duration::from_millis(500));
        assert!(!clock.due(start + Duration::from_millis(499)));
        let late = start + Duration::from_millis(1650);
        let mut count = 0;
        while clock.due(late) {
            count += 1;
            clock.advance();
        }
        assert_eq!(count, 3);
        assert_eq!(clock.delay(late), Duration::from_millis(350));
        clock.retry_after_error(late);
        assert_eq!(clock.delay(late), TICK_INTERVAL);
    }
}

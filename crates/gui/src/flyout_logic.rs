//! Timing for the temporary sidebar flyout, independent of rendering.
use std::time::Duration;

const CLOSE_DELAY: f64 = 0.25;

#[derive(Default)]
pub struct FlyoutTimer {
    left_at: Option<f64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Dismissal {
    KeepOpen,
    Wait(Duration),
    Close,
}

impl FlyoutTimer {
    pub fn reset(&mut self) {
        self.left_at = None;
    }

    pub fn update(&mut self, open: bool, pointer_inside: bool, now: f64) -> Dismissal {
        if !open || pointer_inside {
            self.reset();
            return Dismissal::KeepOpen;
        }
        let left_at = *self.left_at.get_or_insert(now);
        let remaining = CLOSE_DELAY - (now - left_at).max(0.0);
        if remaining <= 0.0 {
            self.reset();
            Dismissal::Close
        } else {
            Dismissal::Wait(Duration::from_secs_f64(remaining))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_after_delay_and_reentry_cancels_it() {
        let mut timer = FlyoutTimer::default();
        assert_eq!(timer.update(true, true, 1.0), Dismissal::KeepOpen);
        assert_eq!(
            timer.update(true, false, 1.0),
            Dismissal::Wait(Duration::from_millis(250))
        );
        assert!(matches!(timer.update(true, false, 1.2), Dismissal::Wait(_)));
        assert_eq!(timer.update(true, true, 1.2), Dismissal::KeepOpen);
        assert!(matches!(timer.update(true, false, 1.3), Dismissal::Wait(_)));
        assert_eq!(timer.update(true, false, 1.56), Dismissal::Close);
    }

    #[test]
    fn explicit_close_and_reopen_reset_pending_timer() {
        let mut timer = FlyoutTimer::default();
        assert!(matches!(timer.update(true, false, 2.0), Dismissal::Wait(_)));
        assert_eq!(timer.update(false, false, 2.1), Dismissal::KeepOpen);
        assert!(matches!(timer.update(true, false, 3.0), Dismissal::Wait(_)));
        timer.reset();
        assert!(matches!(timer.update(true, false, 4.0), Dismissal::Wait(_)));
    }
}

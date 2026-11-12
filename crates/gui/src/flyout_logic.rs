//! Timing for the temporary sidebar flyout, independent of rendering.
use std::time::Duration;

const CLOSE_DELAY: f64 = 0.25;
const MIN_HEIGHT: f32 = 80.0;
const MAX_HEIGHT: f32 = 380.0;

/// The flyout occupies only its content, not the full-height rail beneath it.
/// Long lists scroll within the maximum height.
pub fn flyout_height(item_count: usize, available_height: f32) -> f32 {
    let desired = 54.0 + item_count as f32 * 34.0;
    desired
        .clamp(MIN_HEIGHT, MAX_HEIGHT)
        .min(available_height.max(0.0))
}

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

    pub fn update(
        &mut self,
        open: bool,
        over_rail: bool,
        over_flyout: bool,
        now: f64,
    ) -> Dismissal {
        if !open || over_rail || over_flyout {
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
        assert_eq!(timer.update(true, true, false, 1.0), Dismissal::KeepOpen);
        assert_eq!(
            timer.update(true, false, false, 1.0),
            Dismissal::Wait(Duration::from_millis(250))
        );
        assert!(matches!(
            timer.update(true, false, false, 1.2),
            Dismissal::Wait(_)
        ));
        assert_eq!(timer.update(true, false, true, 1.2), Dismissal::KeepOpen);
        assert!(matches!(
            timer.update(true, false, false, 1.3),
            Dismissal::Wait(_)
        ));
        assert_eq!(timer.update(true, false, false, 1.56), Dismissal::Close);
    }

    #[test]
    fn panel_height_tracks_content_and_is_bounded() {
        assert_eq!(flyout_height(0, 800.0), 80.0);
        assert_eq!(flyout_height(3, 800.0), 156.0);
        assert_eq!(flyout_height(100, 800.0), 380.0);
        assert_eq!(flyout_height(100, 200.0), 200.0);
        assert_eq!(flyout_height(0, 0.0), 0.0);
    }

    #[test]
    fn explicit_close_and_reopen_reset_pending_timer() {
        let mut timer = FlyoutTimer::default();
        assert!(matches!(
            timer.update(true, false, false, 2.0),
            Dismissal::Wait(_)
        ));
        assert_eq!(timer.update(false, false, false, 2.1), Dismissal::KeepOpen);
        assert!(matches!(
            timer.update(true, false, false, 3.0),
            Dismissal::Wait(_)
        ));
        timer.reset();
        assert!(matches!(
            timer.update(true, false, false, 4.0),
            Dismissal::Wait(_)
        ));
    }
}

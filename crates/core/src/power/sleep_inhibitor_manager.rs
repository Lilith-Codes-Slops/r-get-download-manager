use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SleepInhibitorManager {
    active: AtomicUsize,
}

impl SleepInhibitorManager {
    pub fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
        }
    }

    pub fn task_started(&self) {
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    pub fn task_finished(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn is_inhibiting(&self) -> bool {
        self.active.load(Ordering::Acquire) > 0
    }
}

impl Default for SleepInhibitorManager {
    fn default() -> Self {
        Self::new()
    }
}

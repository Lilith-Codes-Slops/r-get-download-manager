use std::time::Duration;

pub struct BackoffRetryPolicy {
    base: Duration,
    max: Duration,
}

impl BackoffRetryPolicy {
    pub fn new() -> Self {
        Self {
            base: Duration::from_millis(200),
            max: Duration::from_secs(30),
        }
    }

    pub fn delay(&self, attempt: u32) -> Duration {
        let exp = self.base * 2u32.saturating_pow(attempt);
        exp.min(self.max)
    }
}

impl Default for BackoffRetryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

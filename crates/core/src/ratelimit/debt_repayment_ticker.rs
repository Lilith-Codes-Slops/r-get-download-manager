use super::token_debt_bucket::TokenDebtBucket;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

/// Periodic ticker that replenishes tokens on a monotonic clock.
pub struct DebtRepaymentTicker {
    bucket: Arc<TokenDebtBucket>,
    period: Duration,
}

impl DebtRepaymentTicker {
    pub fn new(bucket: Arc<TokenDebtBucket>, period: Duration) -> Self {
        Self { bucket, period }
    }

    /// Spawn a background task that calls replenish every `period` (default 10 ms).
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = interval(self.period);
            loop {
                tick.tick().await;
                self.bucket.replenish();
            }
        })
    }
}

impl Default for DebtRepaymentTicker {
    fn default() -> Self {
        Self {
            bucket: Arc::new(TokenDebtBucket::new(0)),
            period: Duration::from_millis(10),
        }
    }
}

//! Shared token-bucket bandwidth limits. Rates are bytes per second internally.
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

struct Bucket {
    rate: u64,
    tokens: f64,
    updated: Instant,
}

impl Bucket {
    fn new(rate: u64) -> Self { Self { rate, tokens: 0.0, updated: Instant::now() } }
    fn set_rate(&mut self, rate: u64) {
        self.rate = rate;
        self.tokens = 0.0;
        self.updated = Instant::now();
    }
    fn refill(&mut self, now: Instant) {
        if self.rate != 0 {
            let capacity = (self.rate as f64 / 4.0).max(1.0);
            self.tokens = (self.tokens + now.duration_since(self.updated).as_secs_f64() * self.rate as f64).min(capacity);
        }
        self.updated = now;
    }
    fn available(&self) -> usize {
        if self.rate == 0 { usize::MAX } else { self.tokens as usize }
    }
    fn consume(&mut self, bytes: usize) {
        if self.rate != 0 { self.tokens -= bytes as f64; }
    }
}

struct Limits { global: Bucket, tasks: HashMap<String, Bucket> }

pub(super) struct BandwidthLimiter { limits: Mutex<Limits> }

impl BandwidthLimiter {
    pub fn new() -> Self {
        Self { limits: Mutex::new(Limits { global: Bucket::new(0), tasks: HashMap::new() }) }
    }
    /// 0 means unlimited. `kbps` is decimal kilobits per second.
    pub async fn set(&self, task: Option<&str>, kbps: u64) -> Result<(), &'static str> {
        let rate = kbps.checked_mul(125).ok_or("speed limit is too large")?;
        let mut limits = self.limits.lock().await;
        if let Some(id) = task {
            limits.tasks.entry(id.to_owned()).or_insert_with(|| Bucket::new(rate)).set_rate(rate);
        } else {
            limits.global.set_rate(rate);
        }
        Ok(())
    }
    pub async fn remove(&self, task: &str) { self.limits.lock().await.tasks.remove(task); }

    /// Export decimal kilobits per second for persistence.
    pub async fn rates(&self) -> (u64, HashMap<String, u64>) {
        let limits = self.limits.lock().await;
        (limits.global.rate / 125, limits.tasks.iter().map(|(id, bucket)| (id.clone(), bucket.rate / 125)).collect())
    }

    /// Wait for the requested byte budget, shared among all active downloads.
    pub async fn acquire(&self, task: &str, mut remaining: usize) {
        while remaining > 0 {
            let taken = {
                let mut limits = self.limits.lock().await;
                let now = Instant::now();
                limits.global.refill(now);
                if let Some(bucket) = limits.tasks.get_mut(task) { bucket.refill(now); }
                let task_available = limits.tasks.get(task).map_or(usize::MAX, Bucket::available);
                let taken = remaining.min(limits.global.available()).min(task_available);
                limits.global.consume(taken);
                if let Some(bucket) = limits.tasks.get_mut(task) { bucket.consume(taken); }
                taken
            };
            remaining -= taken;
            if remaining > 0 { tokio::time::sleep(Duration::from_millis(25)).await; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unlimited_and_invalid_rates() {
        let limiter = BandwidthLimiter::new();
        tokio::time::timeout(Duration::from_millis(100), limiter.acquire("a", 10000)).await.unwrap();
        assert!(limiter.set(None, u64::MAX).await.is_err());
        limiter.set(Some("a"), 0).await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_workers_share_one_task_budget() {
        let limiter = BandwidthLimiter::new();
        limiter.set(Some("task"), 8).await.unwrap(); // 1000 bytes per second total
        let start = Instant::now();
        tokio::time::timeout(Duration::from_secs(8), async {
            tokio::join!(limiter.acquire("task", 1000), limiter.acquire("task", 1000));
        }).await.expect("shared task limiter timed out");
        assert!(start.elapsed() >= Duration::from_millis(1850), "two workers exceeded the aggregate task rate");
    }
    #[tokio::test]
    async fn global_budget_is_shared() {
        let limiter = BandwidthLimiter::new();
        limiter.set(None, 8).await.unwrap(); // 1000 bytes per second
        assert!(tokio::time::timeout(Duration::from_millis(50), limiter.acquire("a", 200)).await.is_err());
        limiter.set(None, 0).await.unwrap();
        tokio::time::timeout(Duration::from_millis(100), limiter.acquire("b", 200)).await.unwrap();
    }
}

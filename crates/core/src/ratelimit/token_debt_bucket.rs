use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

const MAX_DEBT_BYTES: i64 = 256 * 1024; // hard cap ~ few hundred ms at typical rates

/// Atomic token-bucket that permits a capped negative debt balance.
pub struct TokenDebtBucket {
    tokens: AtomicI64,
    rate_bps: AtomicI64,
    last_tick: std::sync::Mutex<Instant>,
}

impl TokenDebtBucket {
    pub fn new(rate_bps: u64) -> Self {
        Self {
            tokens: AtomicI64::new(0),
            rate_bps: AtomicI64::new(rate_bps as i64),
            last_tick: std::sync::Mutex::new(Instant::now()),
        }
    }

    /// Attempt to consume `bytes`. Returns true if the request can proceed
    /// (including when debt is taken within the hard cap).
    pub fn try_consume(&self, bytes: u64) -> bool {
        let needed = bytes as i64;
        let mut current = self.tokens.load(Ordering::Acquire);
        loop {
            if current >= needed {
                match self.tokens.compare_exchange_weak(
                    current,
                    current - needed,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return true,
                    Err(actual) => current = actual,
                }
            } else {
                let debt = needed - current;
                if debt > MAX_DEBT_BYTES {
                    return false;
                }
                match self.tokens.compare_exchange_weak(
                    current,
                    current - needed,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return true,
                    Err(actual) => current = actual,
                }
            }
        }
    }

    pub fn set_rate(&self, rate_bps: u64) {
        self.rate_bps.store(rate_bps as i64, Ordering::Release);
    }

    pub fn current_tokens(&self) -> i64 {
        self.tokens.load(Ordering::Acquire)
    }

    /// Replenish tokens based on elapsed monotonic time.
    pub fn replenish(&self) {
        let mut last = self.last_tick.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        *last = now;
        drop(last);

        let rate = self.rate_bps.load(Ordering::Acquire);
        if rate <= 0 {
            return;
        }
        let add = (elapsed.as_secs_f64() * rate as f64) as i64;
        if add > 0 {
            self.tokens.fetch_add(add, Ordering::AcqRel);
            // Soft clamp to avoid unbounded positive accumulation
            let _ = self.tokens.fetch_min(rate * 2, Ordering::AcqRel);
        }
    }
}

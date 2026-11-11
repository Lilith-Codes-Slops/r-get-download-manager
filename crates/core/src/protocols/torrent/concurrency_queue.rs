//! Fair, cancel-safe permits for active torrent downloads.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

pub struct ConcurrencyQueue {
    permits: Arc<Semaphore>,
    waiting: Mutex<VecDeque<String>>,
    changed: Notify,
}

impl ConcurrencyQueue {
    pub fn new(max_active: usize) -> Arc<Self> {
        assert!(max_active > 0);
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(max_active)),
            waiting: Mutex::new(VecDeque::new()),
            changed: Notify::new(),
        })
    }

    /// Register before spawning so queue positions follow accepted add order.
    pub fn register(self: &Arc<Self>, id: String) -> Ticket {
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(id.clone());
        Ticket {
            queue: self.clone(),
            id,
        }
    }

    fn remove(&self, id: &str) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = waiting.iter().position(|queued| queued == id) {
            waiting.remove(index);
            self.changed.notify_waiters();
        }
    }
}

pub struct Ticket {
    queue: Arc<ConcurrencyQueue>,
    id: String,
}
impl Ticket {
    pub fn position(&self) -> Option<usize> {
        self.queue
            .waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .position(|id| id == &self.id)
            .map(|n| n + 1)
    }
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, tokio::sync::AcquireError> {
        loop {
            // Enable the waiter before inspecting the queue to avoid lost wakes.
            let notified = self.queue.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.position() == Some(1) {
                let permit = self.queue.permits.clone().acquire_owned().await?;
                self.queue.remove(&self.id);
                return Ok(permit);
            }
            notified.await;
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.queue.remove(&self.id);
    }
}

/// Set once at daemon startup/first add. Limits outside 1–32 are rejected.
pub fn global() -> &'static Arc<ConcurrencyQueue> {
    static QUEUE: OnceLock<Arc<ConcurrencyQueue>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let configured = std::env::var("RGET_TORRENT_MAX_ACTIVE").ok();
        let limit = configured
            .as_deref()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| (1..=32).contains(n));
        if configured.is_some() && limit.is_none() {
            tracing::warn!("invalid RGET_TORRENT_MAX_ACTIVE; using 3");
        }
        let limit = limit.unwrap_or(3);
        tracing::info!(
            max_active_downloads = limit,
            "torrent download concurrency limit"
        );
        ConcurrencyQueue::new(limit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_tasks_wait_and_advance_when_permit_is_released() {
        let queue = ConcurrencyQueue::new(1);
        let first = queue.register("first".into());
        let permit = first.acquire().await.unwrap();
        let second = queue.register("second".into());
        let third = queue.register("third".into());
        assert_eq!(second.position(), Some(1));
        assert_eq!(third.position(), Some(2));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), second.acquire())
                .await
                .is_err()
        );
        drop(permit);
        let next = second.acquire().await.unwrap();
        assert_eq!(third.position(), Some(1));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), third.acquire())
                .await
                .is_err()
        );
        drop(next);
        let _last = third.acquire().await.unwrap();
    }

    #[tokio::test]
    async fn registration_order_wins_even_when_later_waiter_is_polled_first() {
        let queue = ConcurrencyQueue::new(1);
        let holder = queue.register("holder".into());
        let permit = holder.acquire().await.unwrap();
        let first = queue.register("first".into());
        let second = queue.register("second".into());
        let later = tokio::spawn(async move { second.acquire().await.unwrap() });
        tokio::task::yield_now().await;
        drop(permit);
        let first_permit = tokio::time::timeout(std::time::Duration::from_secs(1), first.acquire())
            .await
            .unwrap()
            .unwrap();
        assert!(!later.is_finished());
        drop(first_permit);
        let _second_permit = tokio::time::timeout(std::time::Duration::from_secs(1), later)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_queued_ticket_removes_it_without_releasing_an_active_permit() {
        let queue = ConcurrencyQueue::new(1);
        let first = queue.register("first".into());
        let permit = first.acquire().await.unwrap();
        let canceled = queue.register("cancel".into());
        let next = queue.register("next".into());
        drop(canceled);
        assert_eq!(next.position(), Some(1));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), next.acquire())
                .await
                .is_err()
        );
        drop(permit);
        let _next = next.acquire().await.unwrap();
    }
}

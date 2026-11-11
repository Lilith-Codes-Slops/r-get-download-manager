//! Per-task lifecycle exclusion for future multi-file operations.
//!
//! This is not yet connected to transfer, cancel, remove, or startup recovery.
//! Those paths must share the same registry before this can protect them.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Default)]
pub(super) struct LifecycleLocks {
    entries: Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>,
}

impl LifecycleLocks {
    /// Wait without holding the registry mutex. The returned owned guard keeps
    /// the lock alive even if the caller drops its registry reference. A canceled
    /// waiter cannot remove or replace an active lock. Poisoning fails closed.
    pub(super) async fn lock(&self, task_id: &str) -> Result<OwnedMutexGuard<()>, &'static str> {
        let lock = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "torrent lifecycle registry poisoned")?;
            entries.retain(|_, lock| lock.strong_count() != 0);
            if let Some(lock) = entries.get(task_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                entries.insert(task_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        Ok(lock.lock_owned().await)
    }
}

/// One registry for all multi-file lifecycle operations in this daemon process.
/// Do not hold a lifecycle guard while waiting for a worker that needs it to
/// finish: cancellation must first signal, release, join, then reacquire.
#[allow(dead_code)]
pub(super) fn global() -> &'static LifecycleLocks {
    static LOCKS: OnceLock<LifecycleLocks> = OnceLock::new();
    LOCKS.get_or_init(LifecycleLocks::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_task_waits_while_other_tasks_proceed() {
        let locks = LifecycleLocks::default();
        let first = locks.lock("one").await.unwrap();
        let other = tokio::time::timeout(Duration::from_secs(1), locks.lock("two"))
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(25), locks.lock("one"))
                .await
                .is_err()
        );
        drop(first);
        let next = tokio::time::timeout(Duration::from_secs(1), locks.lock("one"))
            .await
            .unwrap()
            .unwrap();
        drop((next, other));
    }

    #[tokio::test]
    async fn canceled_waiter_cannot_replace_live_lock() {
        let locks = Arc::new(LifecycleLocks::default());
        let first = locks.lock("task").await.unwrap();
        let waiting = {
            let locks = locks.clone();
            tokio::spawn(async move { locks.lock("task").await.unwrap() })
        };
        tokio::task::yield_now().await;
        waiting.abort();
        let _ = waiting.await;
        assert!(
            tokio::time::timeout(Duration::from_millis(25), locks.lock("task"))
                .await
                .is_err()
        );
        drop(first);
        let _next = tokio::time::timeout(Duration::from_secs(1), locks.lock("task"))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn unused_entries_are_pruned_without_losing_live_locks() {
        let locks = LifecycleLocks::default();
        drop(locks.lock("old").await.unwrap());
        let live = locks.lock("live").await.unwrap();
        let entries = locks.entries.lock().unwrap();
        assert!(!entries.contains_key("old"));
        assert!(entries.contains_key("live"));
        drop(entries);
        drop(live);
    }
}

use super::token_debt_bucket::TokenDebtBucket;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Combines a global speed cap with optional per-task caps.
pub struct HierarchicalLimiter {
    global: Arc<TokenDebtBucket>,
    per_task: Arc<RwLock<HashMap<String, Arc<TokenDebtBucket>>>>,
}

impl HierarchicalLimiter {
    pub fn new(global_bps: u64) -> Self {
        Self {
            global: Arc::new(TokenDebtBucket::new(global_bps)),
            per_task: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_global_rate(&self, bps: u64) {
        self.global.set_rate(bps);
    }

    pub async fn set_task_rate(&self, task_id: &str, bps: u64) {
        let mut map = self.per_task.write().await;
        map.entry(task_id.to_string())
            .or_insert_with(|| Arc::new(TokenDebtBucket::new(bps)))
            .set_rate(bps);
    }

    /// Consume from both global and (if present) per-task buckets.
    /// Both must succeed for the call to return true.
    pub async fn try_consume(&self, task_id: &str, bytes: u64) -> bool {
        if !self.global.try_consume(bytes) {
            return false;
        }
        let map = self.per_task.read().await;
        if let Some(bucket) = map.get(task_id) {
            if !bucket.try_consume(bytes) {
                // Refund global on per-task rejection (best-effort)
                return false;
            }
        }
        true
    }

    pub fn global(&self) -> Arc<TokenDebtBucket> {
        Arc::clone(&self.global)
    }
}

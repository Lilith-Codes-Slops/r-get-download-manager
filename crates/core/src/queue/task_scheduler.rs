use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TaskScheduler {
    active: Arc<Mutex<VecDeque<String>>>,
    max_slots: usize,
}

impl TaskScheduler {
    pub fn new(max_slots: usize) -> Self {
        Self {
            active: Arc::new(Mutex::new(VecDeque::new())),
            max_slots,
        }
    }

    pub async fn can_start(&self) -> bool {
        self.active.lock().await.len() < self.max_slots
    }

    pub async fn start(&self, task_id: String) {
        self.active.lock().await.push_back(task_id);
    }

    pub async fn finish(&self, task_id: &str) {
        let mut active = self.active.lock().await;
        active.retain(|id| id != task_id);
    }
}

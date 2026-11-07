use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct SegmentCoordinator {
    max_connections: usize,
    semaphore: Arc<Semaphore>,
}

impl SegmentCoordinator {
    pub fn new(max_connections: usize) -> Self {
        Self {
            max_connections,
            semaphore: Arc::new(Semaphore::new(max_connections)),
        }
    }

    pub async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.semaphore.clone().acquire_owned().await.unwrap()
    }

    pub fn max(&self) -> usize {
        self.max_connections
    }
}

use crate::IpcError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// Manages active attached client streams (CLI, GUI, nmhost).
pub struct IpcConnectionPool {
    clients: Arc<RwLock<HashMap<u64, broadcast::Sender<Vec<u8>>>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl IpcConnectionPool {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Register a new client and return its id plus a receiver for broadcasts.
    pub async fn register(&self) -> (u64, broadcast::Receiver<Vec<u8>>) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = broadcast::channel(64);
        self.clients.write().await.insert(id, tx);
        (id, rx)
    }

    /// Remove a client on disconnect.
    pub async fn unregister(&self, id: u64) {
        self.clients.write().await.remove(&id);
    }

    /// Broadcast a frame to all registered clients.
    pub async fn broadcast(&self, frame: Vec<u8>) -> Result<usize, IpcError> {
        let clients = self.clients.read().await;
        let mut sent = 0;
        for tx in clients.values() {
            if tx.send(frame.clone()).is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Number of currently attached clients.
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }
}

impl Default for IpcConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

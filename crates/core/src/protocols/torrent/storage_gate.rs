//! One-way quiescence barrier for a future librqbit storage wrapper.
//! This does not stop or join engine tasks; it only blocks storage access.
use std::sync::{Arc, RwLock, RwLockReadGuard};

#[derive(Clone)]
pub struct StorageGate {
    state: Arc<RwLock<bool>>,
}

impl Default for StorageGate {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageGate {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(true)),
        }
    }
    /// Hold this guard for the entire underlying storage call. Once closed,
    /// no operation may enter. Poisoning fails closed.
    pub fn enter(&self) -> anyhow::Result<RwLockReadGuard<'_, bool>> {
        let guard = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("storage gate poisoned"))?;
        if !*guard {
            anyhow::bail!("torrent storage gate is closed");
        }
        Ok(guard)
    }
    /// Blocking operation: waits for every admitted storage call to return,
    /// then permanently rejects new calls. Never run on an async executor.
    pub fn close_and_wait(&self) -> anyhow::Result<()> {
        let mut guard = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("storage gate poisoned"))?;
        *guard = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Barrier},
        time::Duration,
    };
    #[test]
    fn close_waits_for_inflight_and_prevents_future_entries() {
        let gate = StorageGate::new();
        let guard = gate.enter().unwrap();
        let started = Arc::new(Barrier::new(2));
        let (sent, received) = mpsc::channel();
        let other = gate.clone();
        let ready = started.clone();
        let worker = std::thread::spawn(move || {
            ready.wait();
            other.close_and_wait().unwrap();
            sent.send(()).unwrap();
        });
        started.wait();
        assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
        drop(guard);
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(gate.enter().is_err());
        gate.close_and_wait().unwrap();
    }
    #[test]
    fn poisoning_never_reopens_the_gate() {
        let gate = StorageGate::new();
        let other = gate.clone();
        let _ = std::thread::spawn(move || {
            let _guard = other.state.write().unwrap();
            panic!("injected");
        })
        .join();
        assert!(gate.enter().is_err());
        assert!(gate.close_and_wait().is_err());
    }
}

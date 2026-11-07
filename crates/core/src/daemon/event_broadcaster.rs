use downloader_ipc::DaemonEvent;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct EventBroadcaster {
    tx: broadcast::Sender<DaemonEvent>,
}

impl EventBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(128);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }

    pub fn send(&self, event: DaemonEvent) {
        let _ = self.tx.send(event);
    }
}

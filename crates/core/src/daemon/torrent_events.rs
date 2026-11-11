//! Separate opt-in transition stream; no intermediate states are coalesced.
use downloader_ipc::DaemonEvent;
use std::sync::OnceLock;
use tokio::{net::UnixStream, sync::broadcast};

fn channel() -> &'static broadcast::Sender<DaemonEvent> {
    static CHANNEL: OnceLock<broadcast::Sender<DaemonEvent>> = OnceLock::new();
    CHANNEL.get_or_init(|| broadcast::channel(256).0)
}

pub(super) fn send(event: DaemonEvent) {
    let _ = channel().send(event);
}

pub(super) async fn subscribe(mut stream: UnixStream) {
    let mut rx = channel().subscribe();
    if super::segment_events::send(&mut stream, &DaemonEvent::TorrentEventsReady)
        .await
        .is_err()
    {
        return;
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let event = tokio::select! {
            event = rx.recv() => match event { Ok(event) => event, Err(_) => return },
            _ = heartbeat.tick() => DaemonEvent::TorrentEventsHeartbeat,
        };
        if super::segment_events::send(&mut stream, &event)
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn read_event(stream: &mut UnixStream) -> DaemonEvent {
        let mut prefix = [0; 4];
        stream.read_exact(&mut prefix).await.unwrap();
        let mut payload = vec![0; u32::from_le_bytes(prefix) as usize];
        stream.read_exact(&mut payload).await.unwrap();
        serde_json::from_slice(&payload).unwrap()
    }

    #[tokio::test]
    async fn subscriber_gets_ordered_stall_and_recovery() {
        let (sender, mut reader) = UnixStream::pair().unwrap();
        let task = tokio::spawn(subscribe(sender));
        assert_eq!(
            read_event(&mut reader).await,
            DaemonEvent::TorrentEventsReady
        );
        send(DaemonEvent::TorrentStalled {
            task_id: "unit-test".into(),
            stall_count: 1,
        });
        send(DaemonEvent::TorrentRecovered {
            task_id: "unit-test".into(),
            stall_count: 1,
            duration_ms: 31000,
        });
        let mut events = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while events.len() < 2 {
                let event = read_event(&mut reader).await;
                if matches!(
                    event,
                    DaemonEvent::TorrentStalled { .. } | DaemonEvent::TorrentRecovered { .. }
                ) {
                    events.push(event);
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(&events[0], DaemonEvent::TorrentStalled { task_id, .. } if task_id == "unit-test")
        );
        assert!(
            matches!(&events[1], DaemonEvent::TorrentRecovered { task_id, duration_ms: 31000, .. } if task_id == "unit-test")
        );
        task.abort();
    }
}

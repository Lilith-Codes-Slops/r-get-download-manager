//! Bounded live split snapshots, independent of queue polling and checkpoints.
use downloader_ipc::{DaemonEvent, SegmentProgress};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
use tokio::{io::AsyncWriteExt, net::UnixStream, sync::broadcast};

struct Hub {
    latest: BTreeMap<String, DaemonEvent>,
    sender: broadcast::Sender<DaemonEvent>,
    policies: BTreeMap<String, DaemonEvent>,
    policy_sender: broadcast::Sender<DaemonEvent>,
    maps: BTreeMap<
        String,
        (
            String,
            std::sync::Weak<tokio::sync::Mutex<super::split::segments::SegmentMap>>,
        ),
    >,
}
fn hub() -> &'static Mutex<Hub> {
    static HUB: OnceLock<Mutex<Hub>> = OnceLock::new();
    HUB.get_or_init(|| {
        let (sender, _) = broadcast::channel(1024);
        let (policy_sender, _) = broadcast::channel(1024);
        Mutex::new(Hub {
            latest: BTreeMap::new(),
            sender,
            policies: BTreeMap::new(),
            policy_sender,
            maps: BTreeMap::new(),
        })
    })
}

/// A generation prevents an old transfer from clearing a newer retry's events.
pub(super) struct Publisher {
    id: String,
    generation: String,
}
impl Publisher {
    pub fn new(
        id: String,
        generation: String,
        map: &std::sync::Arc<tokio::sync::Mutex<super::split::segments::SegmentMap>>,
    ) -> Self {
        hub().lock().unwrap_or_else(|e| e.into_inner()).maps.insert(
            id.clone(),
            (generation.clone(), std::sync::Arc::downgrade(map)),
        );
        Self { id, generation }
    }
    pub fn publish(&self, segments: Vec<SegmentProgress>) {
        let event = DaemonEvent::SegmentsSnapshot {
            task_id: self.id.clone(),
            generation: self.generation.clone(),
            segments,
        };
        let mut hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        if !hub
            .maps
            .get(&self.id)
            .is_some_and(|(generation, _)| generation == &self.generation)
        {
            return;
        }
        if hub.latest.get(&self.id) == Some(&event) {
            return;
        }
        let mut deltas = Vec::new();
        if let (
            Some(DaemonEvent::SegmentsSnapshot {
                generation,
                segments: old,
                ..
            }),
            DaemonEvent::SegmentsSnapshot { segments: new, .. },
        ) = (hub.latest.get(&self.id), &event)
        {
            if generation == &self.generation
                && old.len() == new.len()
                && old.iter().zip(new).all(|(a, b)| {
                    a.segment_id == b.segment_id
                        && a.start_byte == b.start_byte
                        && a.end_byte == b.end_byte
                })
            {
                for (a, b) in old.iter().zip(new) {
                    if a != b {
                        deltas.push(DaemonEvent::SegmentUpdate {
                            task_id: self.id.clone(),
                            segment_id: b.segment_id.to_string(),
                            start_byte: b.start_byte,
                            end_byte: b.end_byte,
                            downloaded: b.downloaded,
                            mirror_id: None,
                            generation: Some(self.generation.clone()),
                            status: Some(b.status.clone()),
                        });
                    }
                }
            }
        }
        hub.latest.insert(self.id.clone(), event.clone());
        // Snapshots handle layout changes and cap bursts for large split maps.
        if deltas.is_empty() || deltas.len() > 32 {
            let _ = hub.sender.send(event);
        } else {
            for delta in deltas {
                let _ = hub.sender.send(delta);
            }
        }
    }
    /// Changed values are sent immediately; a periodic refresh is optional.
    pub fn publish_policy(
        &self,
        policy: super::split::sizing_state::PolicySnapshot,
        refresh: bool,
    ) {
        let event = DaemonEvent::SegmentPolicyUpdate {
            task_id: self.id.clone(),
            generation: self.generation.clone(),
            target_size: policy.target_size,
            steal_floor: policy.steal_floor,
            reason: policy.reason.into(),
        };
        let mut hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        if !hub
            .maps
            .get(&self.id)
            .is_some_and(|(generation, _)| generation == &self.generation)
        {
            return;
        }
        let previous = hub.policies.get(&self.id);
        let changed = previous != Some(&event);
        if changed {
            let old_target = match previous {
                Some(DaemonEvent::SegmentPolicyUpdate { target_size, .. }) => *target_size,
                _ => policy.target_size,
            };
            tracing::info!(
                "adaptive size {} → {} bytes ({}) steal_floor={} bytes, generation={}",
                old_target,
                policy.target_size,
                policy.reason,
                policy.steal_floor,
                self.generation
            );
            hub.policies.insert(self.id.clone(), event.clone());
        }
        if changed || refresh {
            let _ = hub.policy_sender.send(event);
        }
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        let mut hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        if hub
            .maps
            .get(&self.id)
            .is_some_and(|(generation, _)| generation == &self.generation)
        {
            hub.maps.remove(&self.id);
        }
        if matches!(hub.policies.get(&self.id), Some(DaemonEvent::SegmentPolicyUpdate { generation, .. }) if generation == &self.generation)
        {
            hub.policies.remove(&self.id);
            let _ = hub.policy_sender.send(DaemonEvent::SegmentPolicyRemoved {
                task_id: self.id.clone(),
                generation: self.generation.clone(),
            });
        }
        let ours = matches!(hub.latest.get(&self.id), Some(DaemonEvent::SegmentsSnapshot { generation, .. }) if generation == &self.generation);
        if ours {
            hub.latest.remove(&self.id);
            let _ = hub.sender.send(DaemonEvent::SegmentsRemoved {
                task_id: self.id.clone(),
                generation: self.generation.clone(),
            });
        }
    }
}

/// Read the authoritative ownership map on demand, not a timer sample.
pub(super) async fn active_count(id: &str, generation: Option<&str>) -> Option<u8> {
    let map = {
        let hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        let (current, map) = hub.maps.get(id)?;
        if Some(current.as_str()) != generation {
            return None;
        }
        map.upgrade()?
    };
    let count = map.lock().await.active_count();
    Some(count)
}

pub(super) async fn send(stream: &mut UnixStream, event: &DaemonEvent) -> Result<(), ()> {
    let bytes = serde_json::to_vec(event).map_err(|_| ())?;
    if bytes.len() > downloader_ipc::MAX_FRAME_PAYLOAD {
        return Err(());
    }
    let write = async {
        stream
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .await?;
        stream.write_all(&bytes).await
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), write)
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

pub(super) async fn subscribe(mut stream: UnixStream) {
    // Subscribe and capture the initial state atomically so updates cannot be lost.
    let (mut rx, initial) = {
        let hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        (
            hub.sender.subscribe(),
            hub.latest.values().cloned().collect::<Vec<_>>(),
        )
    };
    if send(&mut stream, &DaemonEvent::SegmentsReset)
        .await
        .is_err()
    {
        return;
    }
    for event in initial {
        if send(&mut stream, &event).await.is_err() {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let event = tokio::select! {
            event = rx.recv() => match event { Ok(event) => event, Err(_) => return },
            _ = heartbeat.tick() => DaemonEvent::SegmentsHeartbeat,
        };
        if send(&mut stream, &event).await.is_err() {
            return;
        }
    }
}

pub(super) async fn subscribe_policies(mut stream: UnixStream) {
    let (mut rx, initial) = {
        let hub = hub().lock().unwrap_or_else(|e| e.into_inner());
        (
            hub.policy_sender.subscribe(),
            hub.policies.values().cloned().collect::<Vec<_>>(),
        )
    };
    if send(&mut stream, &DaemonEvent::SegmentPoliciesReset)
        .await
        .is_err()
    {
        return;
    }
    for event in initial {
        if send(&mut stream, &event).await.is_err() {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let event = tokio::select! {
            event = rx.recv() => match event { Ok(event) => event, Err(_) => return },
            _ = heartbeat.tick() => DaemonEvent::SegmentsHeartbeat,
        };
        if send(&mut stream, &event).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn part(start: u64, end: u64, downloaded: u64) -> SegmentProgress {
        SegmentProgress {
            segment_id: 0,
            start_byte: start,
            end_byte: end,
            downloaded,
            status: "active".into(),
        }
    }
    fn next_for(rx: &mut broadcast::Receiver<DaemonEvent>, id: &str) -> DaemonEvent {
        loop {
            let event = rx.try_recv().unwrap();
            if matches!(&event, DaemonEvent::SegmentsSnapshot { task_id, .. } | DaemonEvent::SegmentUpdate { task_id, .. } | DaemonEvent::SegmentsRemoved { task_id, .. } if task_id == id)
            {
                return event;
            }
        }
    }
    #[tokio::test]
    async fn policy_changes_are_emitted_immediately_on_the_separate_stream() {
        let id = "segment-policy-test".to_string();
        let mut rx = hub().lock().unwrap().policy_sender.subscribe();
        let mut old_stream = hub().lock().unwrap().sender.subscribe();
        let map = std::sync::Arc::new(tokio::sync::Mutex::new(
            super::super::split::segments::SegmentMap::new(16 * 1024 * 1024, 2).unwrap(),
        ));
        let publisher = std::sync::Arc::new(Publisher::new(id.clone(), "attempt".into(), &map));
        let mut guard = map.lock().await;
        guard.attach_policy_publisher(std::sync::Arc::downgrade(&publisher));
        publisher.publish_policy(guard.policy(), false);
        let next = |rx: &mut broadcast::Receiver<DaemonEvent>| loop {
            let event = rx.try_recv().unwrap();
            if matches!(&event, DaemonEvent::SegmentPolicyUpdate { task_id, .. } if task_id == &id)
            {
                break event;
            }
        };
        assert!(
            matches!(next(&mut rx), DaemonEvent::SegmentPolicyUpdate { reason, .. } if reason == "initial")
        );
        guard.note_rtt(30.0);
        assert!(
            matches!(next(&mut rx), DaemonEvent::SegmentPolicyUpdate { steal_floor, reason, .. } if steal_floor == 2 * 1024 * 1024 && reason == "rtt")
        );
        guard.note_goodput(2.0 * 1024.0 * 1024.0);
        assert!(
            matches!(next(&mut rx), DaemonEvent::SegmentPolicyUpdate { target_size, reason, .. } if target_size == 12 * 1024 * 1024 && reason == "goodput")
        );
        guard.note_rejection(429);
        assert!(
            matches!(next(&mut rx), DaemonEvent::SegmentPolicyUpdate { steal_floor, reason, .. } if steal_floor == 1024 * 1024 && reason == "rejection")
        );
        while let Ok(event) = old_stream.try_recv() {
            assert!(!matches!(
                event,
                DaemonEvent::SegmentPolicyUpdate { .. }
                    | DaemonEvent::SegmentPoliciesReset
                    | DaemonEvent::SegmentPolicyRemoved { .. }
            ));
        }
    }

    #[test]
    fn publishes_deltas_and_replaces_changed_layouts_without_stale_removal() {
        let id = "segment-event-test".to_string();
        let mut rx = hub().lock().unwrap().sender.subscribe();
        let map = std::sync::Arc::new(tokio::sync::Mutex::new(
            super::super::split::segments::SegmentMap::new(2 * 1024 * 1024, 2).unwrap(),
        ));
        let old = Publisher::new(id.clone(), "old".into(), &map);
        old.publish(vec![part(0, 10, 1)]);
        assert!(matches!(
            next_for(&mut rx, &id),
            DaemonEvent::SegmentsSnapshot { .. }
        ));
        old.publish(vec![part(0, 10, 2)]);
        assert!(matches!(
            next_for(&mut rx, &id),
            DaemonEvent::SegmentUpdate { downloaded: 2, .. }
        ));
        old.publish(vec![part(0, 5, 2)]);
        assert!(matches!(
            next_for(&mut rx, &id),
            DaemonEvent::SegmentsSnapshot { .. }
        ));
        let new = Publisher::new(id.clone(), "new".into(), &map);
        new.publish(vec![part(0, 10, 0)]);
        assert!(matches!(
            next_for(&mut rx, &id),
            DaemonEvent::SegmentsSnapshot { .. }
        ));
        drop(old);
        drop(new);
        assert!(
            matches!(next_for(&mut rx, &id), DaemonEvent::SegmentsRemoved { generation, .. } if generation == "new")
        );
    }
}

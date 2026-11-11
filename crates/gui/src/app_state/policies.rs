//! Current per-attempt policy values from the optional policy stream.
use downloader_ipc::{DaemonEvent, DownloadInfo};
use std::collections::HashMap;

pub(super) struct Policy {
    generation: String,
    pub target_size: u64,
    pub steal_floor: u64,
    pub reason: String,
}
#[derive(Default)]
pub(super) struct Policies(HashMap<String, Policy>);
impl Policies {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn apply(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::SegmentPoliciesReset => self.clear(),
            DaemonEvent::SegmentPolicyUpdate {
                task_id,
                generation,
                target_size,
                steal_floor,
                reason,
            } => {
                self.0.insert(
                    task_id,
                    Policy {
                        generation,
                        target_size,
                        steal_floor,
                        reason,
                    },
                );
            }
            DaemonEvent::SegmentPolicyRemoved {
                task_id,
                generation,
            } if self
                .0
                .get(&task_id)
                .is_some_and(|policy| policy.generation == generation) =>
            {
                self.0.remove(&task_id);
            }
            _ => {}
        }
    }
    pub fn get(&self, task: &DownloadInfo) -> Option<&Policy> {
        if !matches!(
            task.state.as_str(),
            "downloading" | "paused" | "pausing" | "resuming"
        ) {
            return None;
        }
        let policy = self.0.get(&task.url)?;
        if task.partial_name.as_deref() != Some(policy.generation.as_str()) {
            return None;
        }
        Some(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> DownloadInfo {
        serde_json::from_str(r#"{"url":"task","state":"downloading","filename":null,"downloaded":0,"total":20,"speed_bps":0,"final_path":null,"partial_name":"attempt"}"#).unwrap()
    }
    #[test]
    fn generation_reset_and_removal_are_respected() {
        let mut policies = Policies::default();
        policies.apply(DaemonEvent::SegmentPolicyUpdate {
            task_id: "task".into(),
            generation: "attempt".into(),
            target_size: 8,
            steal_floor: 2,
            reason: "rtt".into(),
        });
        assert_eq!(policies.get(&task()).unwrap().reason, "rtt");
        let mut retry = task();
        retry.partial_name = Some("new".into());
        assert!(policies.get(&retry).is_none());
        policies.apply(DaemonEvent::SegmentPolicyRemoved {
            task_id: "task".into(),
            generation: "old".into(),
        });
        assert_eq!(policies.get(&task()).unwrap().target_size, 8);
        policies.apply(DaemonEvent::SegmentPoliciesReset);
        assert!(policies.get(&task()).is_none());
    }
}

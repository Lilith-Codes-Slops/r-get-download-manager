use serde::{Deserialize, Serialize};

/// Broadcast events sent from the daemon to subscribed clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DaemonEvent {
    SpeedTick {
        task_id: String,
        speed_bps: u64,
        eta_secs: Option<u64>,
        progress_bps: u64,
    },
    SegmentUpdate {
        task_id: String,
        segment_id: String,
        start_byte: u64,
        end_byte: u64,
        downloaded: u64,
        mirror_id: Option<String>,
    },
    TaskStateChanged {
        task_id: String,
        state: String,
    },
    TaskCompleted {
        task_id: String,
        final_path: String,
        size_bytes: u64,
    },
    TaskFailed {
        task_id: String,
        error: String,
    },
    RefreshRequired {
        task_id: String,
        reason: String,
    },
    MirrorHealth {
        task_id: String,
        mirror_id: String,
        ttfb_ms: Option<u64>,
        goodput_bps: u64,
        error_rate: f32,
        bytes_served: u64,
    },
    QueueSnapshot {
        tasks: Vec<String>,
    },
}

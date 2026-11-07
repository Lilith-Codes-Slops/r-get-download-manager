use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub url: String,
    pub state: String,
    pub filename: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: u64,
    pub final_path: Option<String>,
    /// Whether this server provided enough information for safe HTTP resume.
    #[serde(default)]
    pub resume_supported: bool,
    /// Internal identifier of an owned partial file, if one exists.
    #[serde(default)]
    pub partial_name: Option<String>,
}

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
    ChecksumVerified {
        task_id: String,
        algorithm: String,
    },
    DetailedQueue {
        tasks: Vec<DownloadInfo>,
    },
    QueueSnapshot {
        tasks: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_queue_roundtrips() {
        let event = DaemonEvent::DetailedQueue { tasks: vec![DownloadInfo {
            url: "https://example.test/file".into(), state: "downloading".into(),
            filename: Some("file".into()), downloaded: 42, total: Some(100),
            speed_bps: 12, final_path: None, resume_supported: false, partial_name: None,
        }] };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }
}

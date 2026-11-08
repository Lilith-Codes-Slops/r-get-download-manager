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
    /// Planned destination; may change before atomic publication. Not proof of ownership.
    #[serde(default)]
    pub destination_path: Option<String>,
    /// Whether this server provided enough information for safe HTTP resume.
    #[serde(default)]
    pub resume_supported: bool,
    /// Internal identifier of an owned partial file, if one exists.
    #[serde(default)]
    pub partial_name: Option<String>,
    /// Requested worker ceiling, absent for older task records or daemons.
    #[serde(default)]
    pub requested_connections: Option<u8>,
    /// Workers currently owning a range (or the single-stream transfer), not TCP sockets.
    /// An IPC response is a point-in-time observation, not a guarantee about later activity.
    #[serde(default)]
    pub active_connections: Option<u8>,
    /// Number of split segments, when known.
    #[serde(default)]
    pub segment_count: Option<u16>,
    /// Describes a fallback or explicit capacity rejection when known.
    #[serde(default)]
    pub connection_note: Option<String>,
    /// Whether the running daemon accepts live connection changes for this task.
    #[serde(default)]
    pub live_connections_supported: bool,
}

/// A half-open byte range and committed progress, not socket activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentProgress {
    pub segment_id: u16,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded: u64,
    pub status: String,
}

/// Broadcast events sent from the daemon to subscribed clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DaemonEvent {
    /// Emitted only on the opt-in progress stream; task includes current
    /// committed bytes, speed, state, planned/published destination and attempt.
    ProgressUpdate { task_id: String, task: DownloadInfo },
    ProgressRemoved { task_id: String },
    ProgressReset,
    ProgressHeartbeat,
    SpeedTick {
        task_id: String,
        speed_bps: u64,
        eta_secs: Option<u64>,
        progress_bps: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        destination_path: Option<String>,
    },
    SegmentUpdate {
        task_id: String,
        segment_id: String,
        start_byte: u64,
        end_byte: u64,
        downloaded: u64,
        mirror_id: Option<String>,
        /// Optional for compatibility with older event producers.
        #[serde(default)]
        generation: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    /// Complete replacement for a task's changing segment layout.
    SegmentsSnapshot { task_id: String, generation: String, segments: Vec<SegmentProgress> },
    SegmentsRemoved { task_id: String, generation: String },
    SegmentsReset,
    SegmentsHeartbeat,
    SegmentPoliciesReset,
    SegmentPolicyUpdate { task_id: String, generation: String, target_size: u64, steal_floor: u64, reason: String },
    SegmentPolicyRemoved { task_id: String, generation: String },
    ConnectionsChanged { task_id: String, requested: u8 },
    TaskStateChanged {
        task_id: String,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        destination_path: Option<String>,
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
    fn legacy_queue_task_deserializes_without_connection_telemetry() {
        let json = r#"{"url":"https://example.test/file","state":"paused","filename":null,"downloaded":1,"total":10,"speed_bps":0,"final_path":null}"#;
        let task: DownloadInfo = serde_json::from_str(json).unwrap();
        assert_eq!(task.requested_connections, None);
        assert_eq!(task.active_connections, None);
        assert_eq!(task.segment_count, None);
        assert_eq!(task.connection_note, None);
    }

    #[test]
    fn legacy_segment_update_deserializes_without_new_fields() {
        let json = r#"{"event":"segment_update","task_id":"a","segment_id":"0","start_byte":0,"end_byte":5,"downloaded":2,"mirror_id":null}"#;
        assert!(matches!(serde_json::from_str::<DaemonEvent>(json).unwrap(), DaemonEvent::SegmentUpdate { generation: None, status: None, .. }));
    }

    #[test]
    fn legacy_state_and_speed_events_allow_missing_destination() {
        let state = r#"{"event":"task_state_changed","task_id":"a","state":"downloading"}"#;
        assert!(matches!(serde_json::from_str::<DaemonEvent>(state).unwrap(), DaemonEvent::TaskStateChanged { destination_path: None, .. }));
        let speed = r#"{"event":"speed_tick","task_id":"a","speed_bps":1,"eta_secs":null,"progress_bps":2}"#;
        assert!(matches!(serde_json::from_str::<DaemonEvent>(speed).unwrap(), DaemonEvent::SpeedTick { destination_path: None, .. }));
        let event = DaemonEvent::TaskStateChanged { task_id: "a".into(), state: "downloading".into(), destination_path: Some("/Downloads/file (1).bin".into()) };
        assert_eq!(serde_json::from_str::<DaemonEvent>(&serde_json::to_string(&event).unwrap()).unwrap(), event);
    }

    #[test]
    fn policy_update_roundtrips() {
        let event = DaemonEvent::SegmentPolicyUpdate { task_id: "task".into(), generation: "attempt".into(),
            target_size: 8 * 1024 * 1024, steal_floor: 2 * 1024 * 1024, reason: "rtt".into() };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn progress_update_roundtrips() {
        let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"downloading","filename":"file.bin","downloaded":3,"total":10,"speed_bps":7,"final_path":null,"destination_path":"/Downloads/file.bin","partial_name":".rget-1.part"}"#).unwrap();
        let event = DaemonEvent::ProgressUpdate { task_id: task.url.clone(), task };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn detailed_queue_roundtrips() {
        let event = DaemonEvent::DetailedQueue { tasks: vec![DownloadInfo {
            url: "https://example.test/file".into(), state: "downloading".into(),
            filename: Some("file".into()), downloaded: 42, total: Some(100),
            speed_bps: 12, final_path: None, destination_path: None, resume_supported: false, partial_name: None,
         requested_connections: None, active_connections: None, segment_count: None, connection_note: None, live_connections_supported: false }] };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }
}

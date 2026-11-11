use super::post_download_settings::PostDownloadSettings;
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
    /// Original response/URL-derived name before collision handling, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_filename: Option<String>,
    /// Torrent identity; absent for HTTP tasks and older queue records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentIdentity>,
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

/// Stable task key is `DownloadInfo.url`; this record holds the original source
/// and v1 info-hash without changing the key when metadata arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentIdentity {
    pub source: String,
    pub info_hash: Option<String>,
    /// Owned staging directory component; absence never authorizes deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
    #[serde(default)]
    pub seed: bool,
    pub created_at: i64,
    /// Best-effort observation for the current attempt, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<TorrentMetrics>,
    /// Original validated root directory for a multi-file torrent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_name: Option<String>,
    /// Validated files discovered for this attempt; absent for older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<TorrentFile>>,
    /// Publication succeeded but the directory durability sync failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_warning: Option<String>,
}

/// File priorities are stored only; the current engine integration does not apply them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentFile {
    pub path: String,
    pub length: u64,
    pub priority: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentMetrics {
    pub metadata_fetch_ms: Option<u64>,
    pub time_to_first_piece_ms: Option<u64>,
    pub first_piece_to_finished_ms: Option<u64>,
    pub stall_count: u32,
    pub stalled: bool,
    pub current_stall_ms: u64,
    pub total_stall_ms: u64,
    pub duplicate_bytes: Option<u64>,
    pub request_timeouts: Option<u32>,
    pub peers_dropped: Option<u32>,
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
    /// Only sent on the opt-in torrent transition stream.
    TorrentEventsReady,
    TorrentEventsHeartbeat,
    TorrentStalled {
        task_id: String,
        stall_count: u32,
    },
    TorrentRecovered {
        task_id: String,
        stall_count: u32,
        duration_ms: u64,
    },
    /// Emitted only on the opt-in progress stream; includes current task state.
    ProgressUpdate {
        task_id: String,
        task: Box<DownloadInfo>,
    },
    ProgressRemoved {
        task_id: String,
    },
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
    SegmentsSnapshot {
        task_id: String,
        generation: String,
        segments: Vec<SegmentProgress>,
    },
    SegmentsRemoved {
        task_id: String,
        generation: String,
    },
    SegmentsReset,
    SegmentsHeartbeat,
    SegmentPoliciesReset,
    SegmentPolicyUpdate {
        task_id: String,
        generation: String,
        target_size: u64,
        steal_floor: u64,
        reason: String,
    },
    SegmentPolicyRemoved {
        task_id: String,
        generation: String,
    },
    ConnectionsChanged {
        task_id: String,
        requested: u8,
    },
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
    /// User-specified GUI settings; missing keys fall back to defaults.
    GuiSettings {
        overrides: std::collections::BTreeMap<String, serde_json::Value>,
    },
    /// Directory used for newly admitted tasks.
    DownloadDirectory {
        path: String,
    },
    /// Current daemon-wide post-download action settings.
    PostDownloadSettings {
        settings: PostDownloadSettings,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_queue_task_deserializes_without_connection_telemetry() {
        let json = r#"{"url":"https://example.test/file","state":"paused","filename":null,"downloaded":1,"total":10,"speed_bps":0,"final_path":null}"#;
        let task: DownloadInfo = serde_json::from_str(json).unwrap();
        assert_eq!(task.suggested_filename, None);
        assert_eq!(task.torrent, None);
        assert_eq!(task.requested_connections, None);
        assert_eq!(task.active_connections, None);
        assert_eq!(task.segment_count, None);
        assert_eq!(task.connection_note, None);
    }

    #[test]
    fn legacy_segment_update_deserializes_without_new_fields() {
        let json = r#"{"event":"segment_update","task_id":"a","segment_id":"0","start_byte":0,"end_byte":5,"downloaded":2,"mirror_id":null}"#;
        assert!(matches!(
            serde_json::from_str::<DaemonEvent>(json).unwrap(),
            DaemonEvent::SegmentUpdate {
                generation: None,
                status: None,
                ..
            }
        ));
    }

    #[test]
    fn legacy_state_and_speed_events_allow_missing_destination() {
        let state = r#"{"event":"task_state_changed","task_id":"a","state":"downloading"}"#;
        assert!(matches!(
            serde_json::from_str::<DaemonEvent>(state).unwrap(),
            DaemonEvent::TaskStateChanged {
                destination_path: None,
                ..
            }
        ));
        let speed = r#"{"event":"speed_tick","task_id":"a","speed_bps":1,"eta_secs":null,"progress_bps":2}"#;
        assert!(matches!(
            serde_json::from_str::<DaemonEvent>(speed).unwrap(),
            DaemonEvent::SpeedTick {
                destination_path: None,
                ..
            }
        ));
        let event = DaemonEvent::TaskStateChanged {
            task_id: "a".into(),
            state: "downloading".into(),
            destination_path: Some("/Downloads/file (1).bin".into()),
        };
        assert_eq!(
            serde_json::from_str::<DaemonEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn policy_update_roundtrips() {
        let event = DaemonEvent::SegmentPolicyUpdate {
            task_id: "task".into(),
            generation: "attempt".into(),
            target_size: 8 * 1024 * 1024,
            steal_floor: 2 * 1024 * 1024,
            reason: "rtt".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn torrent_events_and_request_roundtrip() {
        let request = crate::ClientRequest::SubscribeTorrentEvents;
        assert_eq!(
            serde_json::from_str::<crate::ClientRequest>(&serde_json::to_string(&request).unwrap())
                .unwrap(),
            request
        );
        let event = DaemonEvent::TorrentRecovered {
            task_id: "task".into(),
            stall_count: 2,
            duration_ms: 31000,
        };
        assert_eq!(
            serde_json::from_str::<DaemonEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn torrent_identity_roundtrips_and_seed_is_persisted() {
        let identity = TorrentIdentity {
            source: "magnet:?xt=test".into(),
            info_hash: Some("hash".into()),
            stage_name: Some(".rget-torrent-test".into()),
            seed: true,
            created_at: 123,
            metrics: None,
            root_name: None,
            files: None,
            publication_warning: None,
        };
        let json = serde_json::to_string(&identity).unwrap();
        assert_eq!(
            serde_json::from_str::<TorrentIdentity>(&json).unwrap(),
            identity
        );
        let mut with_files = identity.clone();
        with_files.root_name = Some("bundle".into());
        with_files.files = Some(vec![TorrentFile {
            path: "a.bin".into(),
            length: 3,
            priority: 1,
        }]);
        with_files.publication_warning = Some("directory sync failed".into());
        assert_eq!(
            serde_json::from_str::<TorrentIdentity>(&serde_json::to_string(&with_files).unwrap())
                .unwrap(),
            with_files
        );
        assert_eq!(
            serde_json::from_str::<TorrentIdentity>(&json)
                .unwrap()
                .files,
            None
        );
    }

    #[test]
    fn gui_settings_overrides_roundtrip() {
        let event = DaemonEvent::GuiSettings {
            overrides: [("theme".to_owned(), serde_json::json!("dark"))]
                .into_iter()
                .collect(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn download_directory_roundtrips() {
        let event = DaemonEvent::DownloadDirectory {
            path: "/tmp/downloads".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn post_download_settings_roundtrips() {
        let settings = PostDownloadSettings {
            open_containing_folder: true,
        };
        let event = DaemonEvent::PostDownloadSettings { settings };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"post_download_settings\""));
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn progress_update_roundtrips() {
        let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"downloading","filename":"file.bin","downloaded":3,"total":10,"speed_bps":7,"final_path":null,"destination_path":"/Downloads/file.bin","suggested_filename":"original.bin","partial_name":".rget-1.part"}"#).unwrap();
        let event = DaemonEvent::ProgressUpdate {
            task_id: task.url.clone(),
            task: Box::new(task),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }

    #[test]
    fn detailed_queue_roundtrips() {
        let event = DaemonEvent::DetailedQueue {
            tasks: vec![DownloadInfo {
                url: "https://example.test/file".into(),
                state: "downloading".into(),
                filename: Some("file".into()),
                downloaded: 42,
                total: Some(100),
                speed_bps: 12,
                final_path: None,
                destination_path: None,
                suggested_filename: None,
                torrent: None,
                resume_supported: false,
                partial_name: None,
                requested_connections: None,
                active_connections: None,
                segment_count: None,
                connection_note: None,
                live_connections_supported: false,
            }],
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<DaemonEvent>(&json).unwrap(), event);
    }
}

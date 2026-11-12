use super::post_download_settings::PostDownloadSettings;
use serde::{Deserialize, Serialize};

/// Commands sent from clients (CLI, GUI, nmhost) to the core daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientRequest {
    AddTask {
        url: String,
        mirrors: Vec<String>,
        connections: u8,
        extract: bool,
        category: Option<String>,
        not_before: Option<i64>,
    },
    /// Start only with an explicitly confirmed name and one-time folder.
    /// Servers that cannot enforce confirmed publication must reject this
    /// request rather than silently falling back to AddTask.
    AddConfirmedTask {
        url: String,
        mirrors: Vec<String>,
        connections: u8,
        extract: bool,
        category: Option<String>,
        not_before: Option<i64>,
        filename: String,
        folder: String,
    },
    Pause {
        task_id: String,
    },
    Resume {
        task_id: String,
    },
    /// Retry an existing failed task without changing its bound destination.
    Retry {
        task_id: String,
    },
    /// Explicitly discard an existing partial and restart the same task.
    /// Clients must obtain user confirmation before setting this to true.
    RestartFromBeginning {
        task_id: String,
        confirmed: bool,
    },
    /// Stop an active transfer while retaining its task history.
    Cancel {
        task_id: String,
    },
    RefreshUrl {
        task_id: String,
        new_url: String,
    },
    /// Change the active split task's worker ceiling.
    SetConnections {
        task_id: String,
        count: u8,
    },
    SetLimit {
        task_id: Option<String>,
        kbps: u64,
    },
    RemoveTask {
        task_id: String,
        delete_file: bool,
    },
    ListQueue,
    /// Structured queue with transfer telemetry for GUI clients.
    GetDetailedQueue,
    /// Open a separate connection for a stream of complete split snapshots.
    SubscribeSegments,
    /// Dedicated stream for optional adaptive policy telemetry.
    SubscribeSegmentPolicies,
    /// Ordered task progress updates, with a reset on each connection.
    SubscribeProgress,
    /// Separate, opt-in stream of torrent stall and recovery transitions.
    SubscribeTorrentEvents,
    GetSnapshot {
        task_id: Option<String>,
    },
    AddMirror {
        task_id: String,
        mirror_url: String,
    },
    SelectMediaQuality {
        task_id: String,
        video_id: Option<String>,
        audio_id: Option<String>,
    },
    VerifyChecksum {
        task_id: String,
        algorithm: String,
        expected: String,
    },
    /// Read GUI setting overrides; absent keys use the GUI's hardcoded defaults.
    GetGuiSettings,
    /// Atomically set or remove one GUI setting override. Clients remove an
    /// override when its value equals the hardcoded default. `None` removes it;
    /// JSON null is not an override value.
    SetGuiSetting {
        key: String,
        value: Option<serde_json::Value>,
    },
    /// Apply multiple override changes in one transaction. A null value removes
    /// an override and lets the client use its hardcoded default.
    UpdateGuiSettings {
        changes: std::collections::BTreeMap<String, Option<serde_json::Value>>,
    },
    /// Clear GUI setting overrides without modifying tasks or history.
    ResetGuiSettings,
    /// Read or update the daemon-wide directory for new downloads.
    GetDownloadDirectory,
    SetDownloadDirectory {
        path: String,
    },
    /// Resolve an existing one-time folder without changing the global setting.
    ValidateDownloadDirectory {
        path: String,
    },
    /// Read the daemon-wide post-download action settings.
    GetPostDownloadSettings,
    /// Update the daemon-wide post-download action settings.
    SetPostDownloadSettings {
        settings: PostDownloadSettings,
    },
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_start_round_trips_without_losing_fields() {
        let request = ClientRequest::AddConfirmedTask {
            url: "https://example.test/file".into(),
            mirrors: vec!["https://mirror.example.test/file".into()],
            connections: 4,
            extract: true,
            category: Some("documents".into()),
            not_before: Some(12345),
            filename: "confirmed name.txt".into(),
            folder: "/tmp/one-time folder".into(),
        };
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["op"], "add_confirmed_task");
        assert_eq!(encoded["filename"], "confirmed name.txt");
        assert_eq!(encoded["folder"], "/tmp/one-time folder");
        assert_eq!(
            serde_json::from_value::<ClientRequest>(encoded).unwrap(),
            request
        );
        assert!(serde_json::from_str::<ClientRequest>(
            r#"{"op":"add_confirmed_task","url":"https://example.test/file","mirrors":[],"connections":1,"extract":false,"category":null,"not_before":null,"folder":"/tmp"}"#
        ).is_err());
    }

    #[test]
    fn retry_and_explicit_restart_round_trip() {
        for request in [
            ClientRequest::Retry {
                task_id: "task".into(),
            },
            ClientRequest::RestartFromBeginning {
                task_id: "task".into(),
                confirmed: true,
            },
            ClientRequest::RestartFromBeginning {
                task_id: "task".into(),
                confirmed: false,
            },
        ] {
            let json = serde_json::to_value(&request).unwrap();
            assert_eq!(
                serde_json::from_value::<ClientRequest>(json).unwrap(),
                request
            );
        }
        // A missing confirmation must never default to approval.
        assert!(serde_json::from_str::<ClientRequest>(
            r#"{"op":"restart_from_beginning","task_id":"task"}"#
        )
        .is_err());
    }

    #[test]
    fn gui_settings_requests_round_trip() {
        for request in [
            ClientRequest::GetGuiSettings,
            ClientRequest::SetGuiSetting {
                key: "theme".into(),
                value: Some(serde_json::json!("dark")),
            },
            ClientRequest::SetGuiSetting {
                key: "theme".into(),
                value: None,
            },
            ClientRequest::UpdateGuiSettings {
                changes: [
                    ("theme".into(), Some(serde_json::json!("dark"))),
                    ("compact".into(), None),
                ]
                .into_iter()
                .collect(),
            },
            ClientRequest::ResetGuiSettings,
            ClientRequest::GetDownloadDirectory,
            ClientRequest::SetDownloadDirectory {
                path: "/tmp/downloads".into(),
            },
            ClientRequest::ValidateDownloadDirectory {
                path: "/tmp/one-time".into(),
            },
        ] {
            let json = serde_json::to_string(&request).unwrap();
            assert_eq!(
                serde_json::from_str::<ClientRequest>(&json).unwrap(),
                request
            );
        }
    }
}

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
    Pause {
        task_id: String,
    },
    Resume {
        task_id: String,
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
        ] {
            let json = serde_json::to_string(&request).unwrap();
            assert_eq!(
                serde_json::from_str::<ClientRequest>(&json).unwrap(),
                request
            );
        }
    }
}

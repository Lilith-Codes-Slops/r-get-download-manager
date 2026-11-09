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
    SetConnections { task_id: String, count: u8 },
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
    Shutdown,
}

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
    RefreshUrl {
        task_id: String,
        new_url: String,
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
    Shutdown,
}

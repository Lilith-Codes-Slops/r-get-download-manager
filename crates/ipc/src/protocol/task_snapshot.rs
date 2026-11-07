use serde::{Deserialize, Serialize};

/// Complete task state for UI/CLI synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    pub url: String,
    pub state: String,
    pub priority: i32,
    pub connections: u8,
    pub created_at: i64,
    pub not_before: Option<i64>,
    pub files: Vec<FileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub file_id: String,
    pub filename: String,
    pub target_path: String,
    pub total_size: Option<u64>,
    pub downloaded: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: Vec<SegmentSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentSnapshot {
    pub segment_id: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded: u64,
    pub mirror_id: Option<String>,
    pub state: String,
}

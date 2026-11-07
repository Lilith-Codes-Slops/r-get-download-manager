use serde::{Deserialize, Serialize};

/// Media quality choices presented for HLS/DASH/yt-dlp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaManifest {
    pub task_id: String,
    pub title: Option<String>,
    pub tracks: Vec<MediaTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub track_id: String,
    pub kind: String,
    pub qualities: Vec<QualityOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityOption {
    pub quality_id: String,
    pub label: String,
    pub bandwidth: Option<u64>,
    pub resolution: Option<String>,
    pub codecs: Option<String>,
    pub url: String,
}

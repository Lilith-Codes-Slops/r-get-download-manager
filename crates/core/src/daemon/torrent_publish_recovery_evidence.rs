//! Isolated evidence classification; never performs recovery or publication.
use super::{
    torrent_publish_inspection::{self, Inspection},
    torrent_publish_intent::Intent,
    torrent_publish_proof,
};
use crate::protocols::torrent::staging_tree::StageArea;
use downloader_ipc::DownloadInfo;
use std::{fs::File, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Evidence {
    Unproven,
    VerifiedSourcePresent,
    VerifiedDestinationLayout,
    Ambiguous,
}

/// Classify a journaled attempt only. The caller must exclude lifecycle and
/// filesystem mutations throughout the observation. No result authorizes
/// rename, deletion, adoption or completion. In particular, layout and length
/// checks cannot detect same-size content modification after verification.
/// Database/schema errors and unsafe tree walks propagate rather than becoming
/// a positive classification.
#[allow(dead_code)]
pub(super) fn classify(
    database: &Path,
    area: &StageArea,
    root: &File,
    intent: &Intent,
    task: &DownloadInfo,
) -> Result<Evidence> {
    if !torrent_publish_proof::is_verified(database, intent, task)? {
        return Ok(Evidence::Unproven);
    }
    Ok(
        match torrent_publish_inspection::inspect(area, root, intent, task)? {
            Inspection::SourcePresent => Evidence::VerifiedSourcePresent,
            Inspection::DestinationLayoutMatches => Evidence::VerifiedDestinationLayout,
            Inspection::Ambiguous => Evidence::Ambiguous,
        },
    )
}

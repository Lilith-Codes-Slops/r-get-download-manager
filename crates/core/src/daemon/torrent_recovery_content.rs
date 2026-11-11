//! Journal- and content-bound observation, not automatic reconciliation.
use super::{
    torrent_publish_inspection::{self, Inspection},
    torrent_publish_intent::Intent,
    torrent_publish_proof,
    torrent_publish_recovery_evidence::{self, Evidence},
};
use crate::protocols::torrent::{recovery_piece_verify, staging_tree::StageArea};
use downloader_ipc::DownloadInfo;
use nix::{
    fcntl::{openat, OFlag},
    sys::stat::Mode,
};
use rusqlite::OptionalExtension;
use std::{fs::File, io, os::unix::fs::MetadataExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ContentEvidence {
    Other(Evidence),
    MissingInfo,
    DestinationPiecesMatch,
}

/// Caller must hold exclusive lifecycle control, exclude filesystem writers,
/// and independently trust the opened root. A match is evidence only, not
/// permission to complete, delete or replay. Errors always fail closed.
#[allow(dead_code)]
pub(super) fn inspect(
    database: &Path,
    area: &StageArea,
    root: &File,
    intent: &Intent,
    task: &DownloadInfo,
) -> Result<ContentEvidence> {
    let layout = torrent_publish_recovery_evidence::classify(database, area, root, intent, task)?;
    if layout != Evidence::VerifiedDestinationLayout {
        return Ok(ContentEvidence::Other(layout));
    }
    let conn = super::persistence::open(database)?;
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='rget_torrent_recovery_info_v1')", [], |row| row.get(0))?;
    if !exists {
        return Ok(ContentEvidence::MissingInfo);
    }
    // Avoid materializing oversized or non-blob data even for a corrupt DB.
    let info: Option<Option<Vec<u8>>> = conn.query_row("SELECT CASE WHEN typeof(info)='blob' AND length(info)<=8388608 THEN info ELSE NULL END FROM rget_torrent_recovery_info_v1 WHERE task_id=?1",
        [&intent.stage.task_id], |row| row.get(0)).optional()?;
    let Some(info) = info else {
        return Ok(ContentEvidence::MissingInfo);
    };
    let info = info.ok_or("invalid or oversized recovery info")?;
    let manifest = torrent_publish_inspection::manifest_for(task, &intent.stage.task_id)?;
    let hash = task
        .torrent
        .as_ref()
        .and_then(|identity| identity.info_hash.as_deref())
        .ok_or("torrent info-hash missing")?;
    let fd = openat(
        root,
        intent.destination.as_str(),
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let destination = File::from(fd);
    let meta = destination.metadata()?;
    if meta.dev() != intent.payload_device || meta.ino() != intent.payload_inode {
        return Err("publication directory identity changed".into());
    }
    recovery_piece_verify::verify(&destination, intent.payload_device, &manifest, &info, hash)?;
    if !torrent_publish_proof::is_verified(database, intent, task)?
        || torrent_publish_inspection::inspect(area, root, intent, task)?
            != Inspection::DestinationLayoutMatches
    {
        return Err("publication changed during content inspection".into());
    }
    Ok(ContentEvidence::DestinationPiecesMatch)
}

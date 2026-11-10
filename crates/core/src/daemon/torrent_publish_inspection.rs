//! Read-only evidence gathering for a journaled publication; no auto-adoption.
use super::torrent_publish_intent::{self, Intent, Observation};
use crate::protocols::torrent::{
    staging_manifest::{ManifestFile, ValidatedManifest},
    staging_tree::StageArea,
    staging_verify,
};
use downloader_ipc::DownloadInfo;
use nix::{
    fcntl::{openat, OFlag},
    sys::stat::Mode,
};
use std::{fs::File, io, os::unix::fs::MetadataExt};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Inspection {
    SourcePresent,
    DestinationLayoutMatches,
    Ambiguous,
}

pub(super) fn manifest_for(task: &DownloadInfo, task_id: &str) -> Result<ValidatedManifest> {
    if task.url != task_id {
        return Err("publication task identity mismatch".into());
    }
    let torrent = task.torrent.as_ref().ok_or("torrent identity missing")?;
    let hash = torrent
        .info_hash
        .as_deref()
        .ok_or("torrent info-hash missing")?;
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid torrent info-hash".into());
    }
    let root = torrent
        .root_name
        .as_ref()
        .ok_or("torrent root name missing")?;
    let files = torrent
        .files
        .as_ref()
        .ok_or("torrent file manifest missing")?;
    let mut total = 0u64;
    let mut entries = Vec::with_capacity(files.len().min(100_001));
    if files.len() > 100_000 {
        return Err("torrent file count exceeds limit".into());
    }
    for file in files {
        if file.priority > 2 {
            return Err("invalid stored file priority".into());
        }
        if file.path.len() > 4096 || file.path.split('/').take(129).count() > 128 {
            return Err("stored torrent path exceeds limits".into());
        }
        total = total
            .checked_add(file.length)
            .ok_or("torrent length overflow")?;
        entries.push(ManifestFile {
            components: file.path.split('/').map(str::to_owned).collect(),
            length: file.length,
        });
    }
    Ok(ValidatedManifest::new(root.clone(), entries, total)?)
}

/// Confirm journal identity and the exact stored layout at the destination.
/// Caller must exclude concurrent writers; same-size content tampering is NOT
/// detectable here. This function never deletes, adopts or changes task state.
#[allow(dead_code)]
pub(super) fn inspect(
    area: &StageArea,
    root: &File,
    intent: &Intent,
    task: &DownloadInfo,
) -> Result<Inspection> {
    let manifest = manifest_for(task, &intent.stage.task_id)?;
    match torrent_publish_intent::inspect(area, root, intent)? {
        Observation::SourcePresent => return Ok(Inspection::SourcePresent),
        Observation::Ambiguous => return Ok(Inspection::Ambiguous),
        Observation::DestinationMatches => {}
    }
    let fd = openat(
        root,
        intent.destination.as_str(),
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let destination = File::from(fd);
    let before = destination.metadata()?;
    if before.dev() != intent.payload_device || before.ino() != intent.payload_inode {
        return Err("publication directory identity changed".into());
    }
    staging_verify::verify_published_readonly(&destination, &manifest, intent.payload_device)?;
    if torrent_publish_intent::inspect(area, root, intent)? != Observation::DestinationMatches {
        return Err("publication changed during inspection".into());
    }
    Ok(Inspection::DestinationLayoutMatches)
}

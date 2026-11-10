//! Verify the quiescent staged payload against its durable v1 piece hashes.
use super::{torrent_publish_inspection, torrent_stage_allocate::AllocatedStage};
use crate::protocols::torrent::recovery_piece_verify;
use downloader_ipc::DownloadInfo;
use nix::{
    fcntl::{openat, OFlag},
    sys::stat::Mode,
};
use std::{fs::File, io, os::unix::fs::MetadataExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The caller holds the lifecycle guard and has closed and drained storage.
/// Stage retention must already be durable so verification failure is kept.
pub(super) fn verify(database: &Path, stage: &AllocatedStage, task: &DownloadInfo) -> Result<()> {
    let manifest = torrent_publish_inspection::manifest_for(task, &stage.record.task_id)?;
    let hash = task
        .torrent
        .as_ref()
        .and_then(|t| t.info_hash.as_deref())
        .ok_or("torrent info-hash missing")?;
    let conn = super::persistence::open(database)?;
    let info: Option<Vec<u8>> = conn.query_row(
        "SELECT CASE WHEN typeof(info)='blob' AND length(info)<=8388608 THEN info ELSE NULL END FROM rget_torrent_recovery_info_v1 WHERE task_id=?1",
        [&stage.record.task_id], |row| row.get(0))?;
    let info = info.ok_or("invalid or oversized recovery info")?;
    let parent = stage.area.verify(&stage.record)?;
    let fd = openat(
        &parent,
        "payload",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let payload = File::from(fd);
    let observed = payload.metadata()?;
    let allocated = stage.payload.metadata()?;
    if observed.dev() != allocated.dev()
        || observed.ino() != allocated.ino()
        || observed.dev() != stage.record.device
    {
        return Err("staged payload identity changed".into());
    }
    recovery_piece_verify::verify(&payload, observed.dev(), &manifest, &info, hash)?;
    Ok(())
}

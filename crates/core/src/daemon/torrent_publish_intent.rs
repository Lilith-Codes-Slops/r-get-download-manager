//! Durable exact-destination intent for later multi-file publication.
//! These primitives are not wired into torrent downloads or automatic recovery.
use super::torrent_stage_store;
use crate::protocols::torrent::{
    multi_file_layout::validate_component,
    staging_tree::{StageArea, StageRecord},
};
use nix::{
    errno::Errno,
    fcntl::{openat, OFlag},
    sys::stat::Mode,
    unistd::geteuid,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs::File, io, os::unix::fs::MetadataExt, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Intent {
    pub stage: StageRecord,
    pub destination: String,
    pub payload_device: u64,
    pub payload_inode: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observation {
    SourcePresent,
    DestinationMatches,
    Ambiguous,
}

fn open(path: &Path) -> Result<Connection> {
    // Initialize the stage table before a conditional INSERT references it.
    torrent_stage_store::load(path)?;
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_publish_intents_v1 (
        task_id TEXT PRIMARY KEY NOT NULL, stage_name TEXT NOT NULL, stage_device TEXT NOT NULL, stage_inode TEXT NOT NULL,
        destination TEXT NOT NULL, payload_device TEXT NOT NULL, payload_inode TEXT NOT NULL);")?;
    Ok(conn)
}
fn validate(intent: &Intent) -> Result<()> {
    validate_component(&intent.stage.name)?;
    validate_component(&intent.destination)?;
    if !intent.stage.name.starts_with("task-")
        || intent.stage.task_id.is_empty()
        || intent.stage.task_id.len() > 512
    {
        return Err("invalid publication intent".into());
    }
    Ok(())
}
fn open_entry(parent: &File, name: &str) -> io::Result<Option<File>> {
    match openat(
        parent,
        name,
        OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(Errno::ENOENT) => Ok(None),
        Err(e) => Err(io::Error::from_raw_os_error(e as i32)),
    }
}

/// Record a single exact candidate only after the caller has stopped the
/// engine and verified/synced its payload. No existing intent is overwritten.
#[allow(dead_code)]
pub(super) fn begin(
    path: &Path,
    area: &StageArea,
    stage: &StageRecord,
    destination: &str,
) -> Result<Intent> {
    let stage_dir = area.verify(stage)?;
    let payload = open_entry(&stage_dir, "payload")?.ok_or("payload missing")?;
    let meta = payload.metadata()?;
    if !meta.is_dir()
        || meta.dev() != stage.device
        || meta.uid() != geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err("invalid payload identity".into());
    }
    let intent = Intent {
        stage: stage.clone(),
        destination: destination.into(),
        payload_device: meta.dev(),
        payload_inode: meta.ino(),
    };
    validate(&intent)?;
    let conn = open(path)?;
    let count = conn.execute("INSERT INTO rget_torrent_publish_intents_v1(task_id,stage_name,stage_device,stage_inode,destination,payload_device,payload_inode)
        SELECT task_id,stage_name,device,inode,?1,?2,?3 FROM rget_torrent_stages_v1
        WHERE task_id=?4 AND stage_name=?5 AND device=?6 AND inode=?7 AND state='retained'",
        params![destination, meta.dev().to_string(), meta.ino().to_string(), stage.task_id, stage.name, stage.device.to_string(), stage.inode.to_string()])?;
    if count != 1 {
        return Err("no matching retained staging record".into());
    }
    Ok(intent)
}

#[allow(dead_code)]
pub(super) fn load(path: &Path) -> Result<Vec<Intent>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare("SELECT task_id,stage_name,stage_device,stage_inode,destination,payload_device,payload_inode FROM rget_torrent_publish_intents_v1 ORDER BY task_id")?;
    let mut rows = stmt.query([])?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next()? {
        let stage = StageRecord {
            task_id: row.get(0)?,
            name: row.get(1)?,
            device: row.get::<_, String>(2)?.parse()?,
            inode: row.get::<_, String>(3)?.parse()?,
        };
        let intent = Intent {
            stage,
            destination: row.get(4)?,
            payload_device: row.get::<_, String>(5)?.parse()?,
            payload_inode: row.get::<_, String>(6)?.parse()?,
        };
        validate(&intent)?;
        entries.push(intent);
    }
    Ok(entries)
}

pub(super) fn has_intent(path: &Path, task_id: &str) -> Result<bool> {
    let conn = open(path)?;
    Ok(conn
        .query_row(
            "SELECT 1 FROM rget_torrent_publish_intents_v1 WHERE task_id=?1",
            [task_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Observation is only evidence for later reconciliation, not permission to
/// delete, adopt, or mark completed. Caller must prevent concurrent mutations.
#[allow(dead_code)]
pub(super) fn inspect(
    area: &StageArea,
    destination_root: &File,
    intent: &Intent,
) -> Result<Observation> {
    validate(intent)?;
    let stage = area.verify(&intent.stage)?;
    let source = open_entry(&stage, "payload")?;
    let dest = open_entry(destination_root, &intent.destination)?;
    let matches_payload = |file: &File| -> io::Result<bool> {
        let meta = file.metadata()?;
        Ok(meta.is_dir()
            && meta.dev() == intent.payload_device
            && meta.ino() == intent.payload_inode
            && meta.uid() == geteuid().as_raw())
    };
    let source_matches = source
        .as_ref()
        .map(matches_payload)
        .transpose()?
        .unwrap_or(false);
    let dest_matches = dest
        .as_ref()
        .map(matches_payload)
        .transpose()?
        .unwrap_or(false);
    match (source_matches, source.is_none(), dest_matches) {
        (true, false, false) => Ok(Observation::SourcePresent),
        (false, true, true) => Ok(Observation::DestinationMatches),
        _ => Ok(Observation::Ambiguous),
    }
}

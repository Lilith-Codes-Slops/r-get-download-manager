//! Durable identity of a published multi-file directory. Not yet used for removal.
use crate::protocols::torrent::multi_file_layout::validate_component;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::{fs::File, os::unix::fs::MetadataExt, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublishedRecord {
    pub task_id: String, pub name: String,
    pub root_device: u64, pub root_inode: u64,
    pub device: u64, pub inode: u64,
}

fn open(path: &Path) -> Result<Connection> {
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_published_v1 (
        task_id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, root_device TEXT NOT NULL,
        root_inode TEXT NOT NULL, device TEXT NOT NULL, inode TEXT NOT NULL)")?;
    Ok(conn)
}
pub(super) fn ensure_schema(path: &Path) -> Result<()> { open(path)?; Ok(()) }

/// Insert only, inside completion transaction. Never overwrite older ownership.
pub(super) fn insert(tx: &Transaction<'_>, root: &File, intent: &super::torrent_publish_intent::Intent) -> Result<()> {
    validate_component(&intent.destination)?;
    let meta = root.metadata()?;
    tx.execute("INSERT INTO rget_torrent_published_v1(task_id,name,root_device,root_inode,device,inode) VALUES(?1,?2,?3,?4,?5,?6)",
        params![intent.stage.task_id, intent.destination, meta.dev().to_string(), meta.ino().to_string(), intent.payload_device.to_string(), intent.payload_inode.to_string()])?;
    Ok(())
}

#[allow(dead_code)]
pub(super) fn load(path: &Path, task_id: &str) -> Result<Option<PublishedRecord>> {
    let conn = open(path)?;
    let row: Option<(String, String, String, String, String)> = conn.query_row(
        "SELECT name,root_device,root_inode,device,inode FROM rget_torrent_published_v1 WHERE task_id=?1", [task_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
    let Some((name, root_device, root_inode, device, inode)) = row else { return Ok(None); };
    validate_component(&name)?;
    if task_id.is_empty() || task_id.len() > 512 { return Err("invalid published task identity".into()); }
    Ok(Some(PublishedRecord { task_id: task_id.into(), name, root_device: root_device.parse()?, root_inode: root_inode.parse()?, device: device.parse()?, inode: inode.parse()? }))
}

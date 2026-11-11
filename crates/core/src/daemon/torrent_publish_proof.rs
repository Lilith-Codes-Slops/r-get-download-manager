//! Evidence that a journaled stage was layout-checked and synced pre-rename.
//! Not an integrity hash or an attestation against malicious local DB changes.
use super::{
    torrent_publish_inspection,
    torrent_publish_intent::{self, Intent},
};
use crate::protocols::torrent::{
    staging_tree::{StageArea, StageRecord},
    staging_verify,
};
use downloader_ipc::DownloadInfo;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn open(path: &Path) -> Result<Connection> {
    // Ensure journal and queue tables exist before creating the evidence table.
    torrent_publish_intent::load(path)?;
    let conn = super::persistence::open(path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_torrent_publish_proofs_v1 (task_id TEXT PRIMARY KEY NOT NULL, fingerprint TEXT NOT NULL)")?;
    Ok(conn)
}

pub(super) fn ensure_schema(path: &Path) -> Result<()> {
    open(path)?;
    Ok(())
}

fn fingerprint(task: &DownloadInfo, intent: &Intent) -> Result<String> {
    torrent_publish_inspection::manifest_for(task, &intent.stage.task_id)?;
    let identity = task.torrent.as_ref().ok_or("torrent identity missing")?;
    // Bind both the manifest and the exact directory identities/candidate.
    let bytes = serde_json::to_vec(&(
        "rget-torrent-publication-v1",
        &task.url,
        &identity.source,
        &identity.info_hash,
        &identity.root_name,
        &identity.files,
        &intent.stage.name,
        intent.stage.device,
        intent.stage.inode,
        &intent.destination,
        intent.payload_device,
        intent.payload_inode,
    ))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// Caller must hold the task lifecycle guard, stop/join a successfully finished
/// engine, and prevent external writes until publication. The check covers
/// layout and lengths, not piece hashes. Pre-rename sync errors retain staging.
/// If evidence insertion fails after intent creation, the unproven intent is
/// retained and must not be automatically replayed.
#[allow(dead_code)]
pub(super) fn begin_verified(
    path: &Path,
    area: &StageArea,
    stage: &StageRecord,
    destination: &str,
    task: &DownloadInfo,
) -> Result<Intent> {
    let manifest = torrent_publish_inspection::manifest_for(task, &stage.task_id)?;
    let conn = open(path)?;
    let json: String = conn.query_row(
        "SELECT info FROM rget_queue_v1 WHERE url=?1",
        [&stage.task_id],
        |row| row.get(0),
    )?;
    let durable: DownloadInfo = serde_json::from_str(&json)?;
    // Ensure the exact metadata is already durable before touching the tree.
    if task.state != "finalizing"
        || durable.state != "finalizing"
        || durable.url != task.url
        || durable
            .torrent
            .as_ref()
            .map(|t| (&t.source, &t.info_hash, &t.root_name, &t.files))
            != task
                .torrent
                .as_ref()
                .map(|t| (&t.source, &t.info_hash, &t.root_name, &t.files))
    {
        return Err("durable manifest differs from task".into());
    }
    drop(conn);
    staging_verify::verify_and_sync(area, stage, &manifest)?;
    let intent = torrent_publish_intent::begin(path, area, stage, destination)?;
    let expected = fingerprint(task, &intent)?;
    let mut conn = open(path)?;
    let tx = conn.transaction()?;
    let json: String = tx.query_row(
        "SELECT info FROM rget_queue_v1 WHERE url=?1",
        [&stage.task_id],
        |row| row.get(0),
    )?;
    let durable: DownloadInfo = serde_json::from_str(&json)?;
    if durable.state != "finalizing" || fingerprint(&durable, &intent)? != expected {
        return Err("durable task or manifest changed before proof commit".into());
    }
    let rows = tx.execute("INSERT INTO rget_torrent_publish_proofs_v1(task_id,fingerprint)
        SELECT i.task_id,?1 FROM rget_torrent_publish_intents_v1 i JOIN rget_torrent_stages_v1 s ON s.task_id=i.task_id
        WHERE i.task_id=?2 AND i.stage_name=?3 AND i.stage_device=?4 AND i.stage_inode=?5 AND i.destination=?6 AND i.payload_device=?7 AND i.payload_inode=?8
        AND s.stage_name=i.stage_name AND s.device=i.stage_device AND s.inode=i.stage_inode AND s.state='retained'",
        params![expected, stage.task_id, stage.name, stage.device.to_string(), stage.inode.to_string(), intent.destination, intent.payload_device.to_string(), intent.payload_inode.to_string()])?;
    if rows != 1 {
        return Err("publication intent or stage changed before proof commit".into());
    }
    tx.commit()?;
    Ok(intent)
}

/// Missing or stale evidence is false. A true result still requires checking
/// the real destination and stored manifest before any recovery decision.
#[allow(dead_code)]
pub(super) fn is_verified(path: &Path, intent: &Intent, task: &DownloadInfo) -> Result<bool> {
    let expected = fingerprint(task, intent)?;
    let conn = open(path)?;
    let found: Option<String> = conn
        .query_row(
            "SELECT fingerprint FROM rget_torrent_publish_proofs_v1 WHERE task_id=?1",
            [&intent.stage.task_id],
            |row| row.get(0),
        )
        .optional()?;
    if found.as_deref() != Some(expected.as_str()) {
        return Ok(false);
    }
    let json: Option<String> = conn
        .query_row(
            "SELECT info FROM rget_queue_v1 WHERE url=?1",
            [&intent.stage.task_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Ok(false);
    };
    let durable: DownloadInfo = serde_json::from_str(&json)?;
    if fingerprint(&durable, intent)? != expected {
        return Ok(false);
    }
    let stage_ok = super::torrent_stage_store::load(path)?.iter().any(|row| {
        row.state == super::torrent_stage_store::State::Retained
            && row.identity.as_ref() == Some(&intent.stage)
    });
    Ok(stage_ok
        && torrent_publish_intent::load(path)?
            .iter()
            .any(|entry| entry == intent))
}

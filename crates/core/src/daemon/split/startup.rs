//! Discover only task-owned v2 checkpoints during daemon startup.
use super::{checkpoint::SplitRecord, recovery::{self, PreparedSplit}};
use crate::daemon::persistence::SavedState;
use std::{collections::HashMap, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

pub(in crate::daemon) fn load_records(conn: &rusqlite::Connection) -> Result<HashMap<String, SplitRecord>, rusqlite::Error> {
    let mut result = HashMap::new();
    let mut statement = conn.prepare("SELECT url, info FROM rget_split_checkpoints_v2")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, json) = row?;
        if let Ok(record) = serde_json::from_str::<SplitRecord>(&json) {
            if record.task_url == id { result.insert(id, record); }
        }
    }
    Ok(result)
}

fn sidecar_version(path: &Path) -> Option<u64> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path).ok()?;
    if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > 64 * 1024 { return None; }
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 64 * 1024 { return None; }
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?.get("version")?.as_u64()
}

fn already_published(saved: &mut SavedState, root: &Path, id: &str, partial: &Path) -> bool {
    let Some(record) = saved.split_checkpoints.get(id) else { return false };
    let Some(task) = saved.tasks.get(id) else { return false };
    if task.partial_name.as_deref() != Some(record.partial_name.as_str())
        || recovery::validate_record(record, partial).is_err()
        || record.segments.iter().any(|segment| segment.downloaded != segment.end - segment.start) { return false; }
    let path = root.join(&record.filename);
    let Ok(file) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path) else { return false };
    let Ok(meta) = file.metadata() else { return false };
    if !meta.is_file() || meta.len() != record.total || recovery::verify(&file, record).is_err() { return false; }
    if let Some(task) = saved.tasks.get_mut(id) {
        task.state = "completed".into(); task.downloaded = record.total; task.total = Some(record.total);
        task.filename = Some(record.filename.clone()); task.final_path = Some(path.to_string_lossy().into_owned());
        task.partial_name = None; task.speed_bps = 0; task.resume_supported = false;
    }
    true
}

pub(in crate::daemon) fn reconcile(saved: &mut SavedState, root: &Path) -> Vec<PreparedSplit> {
    let mut result = Vec::new();
    for (id, prior) in saved.prior_states.clone() {
        let Some(name) = saved.tasks.get(&id).and_then(|t| t.partial_name.as_deref()).map(str::to_owned) else { continue };
        // Do not inspect paths that are not a single task-owned partial name.
        if name.contains('/') || name.contains('\\') || !name.starts_with(".rget-") || !name.ends_with(".part") { continue; }
        let partial = root.join(&name);
        let sidecar = root.join(format!("{name}.meta"));
        let database = saved.split_checkpoints.get(&id).filter(|record| record.partial_name == name);
        if database.is_none() && sidecar_version(&sidecar) != Some(2) { continue; }
        saved.prior_states.remove(&id);
        if already_published(saved, root, &id, &partial) { continue; }
        let prepared = recovery::prepare(&sidecar, &partial, &id, saved.split_checkpoints.get(&id).filter(|record| record.partial_name == name));
        match prepared {
            Ok(item) => {
                if let Some(task) = saved.tasks.get_mut(&id) {
                    task.state = if matches!(prior.as_str(), "paused" | "pausing") { "paused" } else { "downloading" }.into();
                    task.downloaded = item.record.segments.iter().map(|s| s.downloaded).sum();
                    task.total = Some(item.record.total); task.filename = Some(item.record.filename.clone());
                    task.speed_bps = 0; task.resume_supported = true;
                    task.requested_connections = Some(task.requested_connections.filter(|count| (1..=32).contains(count)).unwrap_or(item.record.requested_connections));
                    task.live_connections_supported = true;
                    task.active_connections = Some(0); task.segment_count = Some(item.record.segments.len() as u16);
                }
                result.push(item);
            }
            Err(error) => if let Some(task) = saved.tasks.get_mut(&id) {
                task.state = format!("failed: recovery unavailable: {error}"); task.resume_supported = false;
            },
        }
    }
    result
}

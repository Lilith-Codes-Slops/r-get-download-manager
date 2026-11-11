//! Conservative startup reconciliation for single-stream checkpoints.
use super::checkpoint::{write_atomic, RecoveryRecord};
use super::checkpoint_validation::{validate, verify_prefix};
use super::persistence::SavedState;
use std::path::{Component, Path, PathBuf};

pub(super) struct PreparedRecovery {
    pub record: RecoveryRecord,
    pub partial: PathBuf,
    pub file: std::fs::File,
    pub hash: blake3::Hasher,
    pub paused: bool,
}

fn same_resource(a: &RecoveryRecord, b: &RecoveryRecord) -> bool {
    a.version == b.version
        && a.task_url == b.task_url
        && a.resolved_url == b.resolved_url
        && a.filename == b.filename
        && a.partial_name == b.partial_name
        && a.etag == b.etag
        && a.total == b.total
}

fn choose(
    sidecar: RecoveryRecord,
    database: Option<&RecoveryRecord>,
) -> Result<RecoveryRecord, String> {
    let Some(db) = database else {
        return Ok(sidecar);
    };
    if !same_resource(&sidecar, db) {
        return Err("checkpoint identities differ".into());
    }
    if sidecar.sequence == db.sequence {
        return if sidecar == *db {
            Ok(sidecar)
        } else {
            Err("equal checkpoint sequences contain different data".into())
        };
    }
    let (newer, older) = if sidecar.sequence > db.sequence {
        (&sidecar, db)
    } else {
        (db, &sidecar)
    };
    if newer.downloaded < older.downloaded || newer.downloaded > newer.total {
        return Err("checkpoint sequence and progress disagree".into());
    }
    Ok(newer.clone())
}

/// Reconciles only task-owned partial filenames in the configured Downloads
/// directory. Orphan files are never adopted or deleted.
pub(super) fn reconcile(saved: &mut SavedState, root: &Path) -> Vec<PreparedRecovery> {
    let mut prepared = Vec::new();
    for (id, previous_state) in saved.prior_states.clone() {
        #[cfg(target_os = "linux")]
        let task_root = saved.task_roots.get(&id).cloned();
        #[cfg(target_os = "linux")]
        let root = task_root.as_deref().unwrap_or(root);
        if restore_published_file(saved, root, &id) {
            continue;
        }
        let result = prepare_one(saved, root, &id, &previous_state);
        match result {
            Ok(item) => {
                if let Some(task) = saved.tasks.get_mut(&id) {
                    task.state = if item.paused { "paused" } else { "downloading" }.into();
                    task.downloaded = item.record.downloaded;
                    task.total = Some(item.record.total);
                    task.filename = Some(item.record.filename.clone());
                    task.destination_path = Some(
                        root.join(&item.record.filename)
                            .to_string_lossy()
                            .into_owned(),
                    );
                    task.resume_supported = true;
                    task.speed_bps = 0;
                }
                prepared.push(item);
            }
            Err(error) => {
                if let Some(task) = saved.tasks.get_mut(&id) {
                    task.state = format!("failed: recovery unavailable: {error}");
                    task.resume_supported = false;
                }
            }
        }
    }
    prepared
}

/// A size match alone cannot prove a pre-existing final file is ours. Accept it
/// only if SQLite contains a complete checkpoint and its full digest matches.
fn restore_published_file(saved: &mut SavedState, root: &Path, id: &str) -> bool {
    use std::io::Read;
    let Some(record) = saved.checkpoints.get(id) else {
        return false;
    };
    let Some(task) = saved.tasks.get(id) else {
        return false;
    };
    let mut components = Path::new(&record.filename).components();
    if record.version != 1
        || record.sequence == 0
        || record.task_url != id
        || task.partial_name.as_deref() != Some(record.partial_name.as_str())
        || record.downloaded != record.total
        || record.filename.contains('\\')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return false;
    }
    let path = root.join(&record.filename);
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let Ok(mut file) = options.open(&path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() != record.total {
        return false;
    }
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let Ok(n) = file.read(&mut buffer) else {
            return false;
        };
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if hash.finalize().to_hex().as_str() != record.prefix_blake3 {
        return false;
    }
    if let Some(task) = saved.tasks.get_mut(id) {
        task.state = "completed".into();
        task.downloaded = record.total;
        task.total = Some(record.total);
        task.filename = Some(record.filename.clone());
        task.final_path = Some(path.to_string_lossy().into_owned());
        task.partial_name = None;
        task.resume_supported = false;
        task.speed_bps = 0;
    }
    true
}

pub(super) fn prepare_one(
    saved: &SavedState,
    root: &Path,
    id: &str,
    state: &str,
) -> Result<PreparedRecovery, String> {
    use std::io::{Seek, SeekFrom};
    let name = saved
        .tasks
        .get(id)
        .and_then(|t| t.partial_name.as_deref())
        .ok_or("no recorded partial file")?;
    let mut components = Path::new(name).components();
    if name.contains('\\')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !name.starts_with(".rget-")
        || !name.ends_with(".part")
    {
        return Err("unsafe partial filename".into());
    }
    let partial = root.join(name);
    let mut sidecar_name = partial.as_os_str().to_os_string();
    sidecar_name.push(".meta");
    let sidecar = PathBuf::from(sidecar_name);
    let sidecar_record =
        validate(&sidecar, &partial).map_err(|e| format!("sidecar validation failed: {e}"))?;
    if sidecar_record.task_url != id {
        return Err("sidecar belongs to a different task".into());
    }
    // A retry of the same URL owns a fresh partial file. A database record
    // from an older attempt must not be reconciled against the new sidecar.
    let database = saved
        .checkpoints
        .get(id)
        .filter(|record| record.partial_name == name);
    let record = choose(sidecar_record.clone(), database)?;
    if record.downloaded > record.total {
        return Err("checkpoint exceeds total length".into());
    }
    // A destination collision is resolved atomically when the validated partial is published.
    let (mut file, hash) =
        verify_prefix(&record, &partial).map_err(|e| format!("partial validation failed: {e}"))?;
    file.set_len(record.downloaded).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(record.downloaded))
        .map_err(|e| e.to_string())?;
    if record != sidecar_record {
        write_atomic(&sidecar, &record).map_err(|e| e.to_string())?;
    }
    Ok(PreparedRecovery {
        record,
        partial,
        file,
        hash,
        paused: matches!(state, "paused" | "pausing"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(seq: u64, count: u64) -> RecoveryRecord {
        RecoveryRecord {
            version: 1,
            sequence: seq,
            task_url: "https://example.test/file".into(),
            resolved_url: "https://example.test/file".into(),
            filename: "file".into(),
            partial_name: ".rget-1-1.part".into(),
            etag: "\"v1\"".into(),
            total: 10,
            downloaded: count,
            prefix_blake3: format!("hash-{count}"),
        }
    }
    #[test]
    fn reconciles_newer_record_and_rejects_inconsistent_sequences() {
        assert_eq!(
            choose(record(2, 5), Some(&record(1, 3))).unwrap().sequence,
            2
        );
        assert_eq!(
            choose(record(1, 3), Some(&record(2, 5))).unwrap().sequence,
            2
        );
        assert!(choose(record(2, 2), Some(&record(1, 3))).is_err());
        assert!(choose(record(1, 2), Some(&record(1, 3))).is_err());
        let mut altered = record(2, 5);
        altered.etag = "\"other\"".into();
        assert!(choose(record(1, 3), Some(&altered)).is_err());
    }
}

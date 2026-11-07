//! Validate and reconcile version 2 split checkpoints without truncating ranges.
use super::{checkpoint::SplitRecord, segments::SegmentMap};
use std::io::Read;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Component, Path};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) struct PreparedSplit {
    pub record: SplitRecord,
    pub file: Arc<std::fs::File>,
    pub map: Arc<Mutex<SegmentMap>>,
}

fn safe_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.contains('/') && !name.contains('\\') && matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

pub(super) fn validate_record(record: &SplitRecord, partial: &Path) -> Result<(), String> {
    if record.version != 2 || record.sequence == 0 || !(2..=32).contains(&record.requested_connections)
        || !crate::daemon::range::strong_etag(&record.etag) {
        return Err("invalid split checkpoint header".into());
    }
    for url in [&record.task_url, &record.resolved_url] {
        if !matches!(reqwest::Url::parse(url), Ok(ref url) if matches!(url.scheme(), "http" | "https")) { return Err("invalid checkpoint URL".into()); }
    }
    if !safe_name(&record.filename) || !safe_name(&record.partial_name)
        || !record.partial_name.starts_with(".rget-") || !record.partial_name.ends_with(".part")
        || partial.file_name().and_then(|n| n.to_str()) != Some(record.partial_name.as_str()) {
        return Err("unsafe or mismatched checkpoint filename".into());
    }
    if record.segments.is_empty() || record.segments.len() > 32 { return Err("invalid split segment count".into()); }
    let mut next = 0;
    for segment in &record.segments {
        if segment.start != next || segment.start >= segment.end || segment.end > record.total
            || segment.downloaded > segment.end - segment.start || segment.hash.len() != 64
            || !segment.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid or overlapping split segment map".into());
        }
        next = segment.end;
    }
    if next != record.total { return Err("split segment map does not cover the file".into()); }
    Ok(())
}

fn choose(sidecar: &SplitRecord, database: Option<&SplitRecord>) -> Result<SplitRecord, String> {
    let Some(db) = database else { return Ok(sidecar.clone()) };
    if sidecar.version != db.version || sidecar.task_url != db.task_url || sidecar.resolved_url != db.resolved_url
        || sidecar.filename != db.filename || sidecar.partial_name != db.partial_name || sidecar.etag != db.etag
        || sidecar.total != db.total || sidecar.requested_connections != db.requested_connections
        || sidecar.segments.len() != db.segments.len() {
        return Err("split checkpoint identities differ".into());
    }
    if sidecar.sequence == db.sequence {
        return if sidecar == db { Ok(sidecar.clone()) } else { Err("equal split sequences differ".into()) };
    }
    let (newer, older) = if sidecar.sequence > db.sequence { (sidecar, db) } else { (db, sidecar) };
    for (new, old) in newer.segments.iter().zip(&older.segments) {
        if new.start != old.start || new.end != old.end || new.downloaded < old.downloaded
            || (new.downloaded == old.downloaded && new.hash != old.hash) {
            return Err("split sequence and progress disagree".into());
        }
    }
    Ok(newer.clone())
}

fn verify(file: &std::fs::File, record: &SplitRecord) -> Result<Vec<blake3::Hasher>, String> {
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > record.total { return Err("partial file exceeds the expected size".into()); }
    let mut hashes = Vec::with_capacity(record.segments.len());
    let mut buffer = [0u8; 64 * 1024];
    for segment in &record.segments {
        if segment.downloaded > 0 && length < segment.start + segment.downloaded { return Err("checkpointed range is missing bytes".into()); }
        let mut hash = blake3::Hasher::new();
        let mut position = segment.start;
        let end = segment.start + segment.downloaded;
        while position < end {
            let size = (end - position).min(buffer.len() as u64) as usize;
            let n = file.read_at(&mut buffer[..size], position).map_err(|e| e.to_string())?;
            if n == 0 { return Err("checkpointed range was truncated".into()); }
            hash.update(&buffer[..n]);
            position += n as u64;
        }
        if hash.finalize().to_hex().as_str() != segment.hash { return Err("split range checksum mismatch".into()); }
        hashes.push(hash);
    }
    Ok(hashes)
}

/// Only a partial named by a known task may be passed here. Uncheckpointed
/// bytes are left untouched and will be overwritten by subsequent range writes.
pub(super) fn prepare(sidecar: &Path, partial: &Path, task_url: &str, database: Option<&SplitRecord>) -> Result<PreparedSplit, String> {
    let mut expected = partial.as_os_str().to_os_string();
    expected.push(".meta");
    if sidecar != Path::new(&expected) { return Err("unexpected split sidecar path".into()); }
    let mut meta = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(sidecar).map_err(|e| e.to_string())?;
    let stat = meta.metadata().map_err(|e| e.to_string())?;
    if !stat.is_file() || stat.len() > 64 * 1024 { return Err("invalid split sidecar".into()); }
    let mut data = Vec::new();
    (&mut meta).take(64 * 1024 + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() > 64 * 1024 { return Err("oversized split sidecar".into()); }
    let side: SplitRecord = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    validate_record(&side, partial)?;
    if side.task_url != task_url { return Err("sidecar belongs to another task".into()); }
    if let Some(db) = database { validate_record(db, partial)?; }
    let record = choose(&side, database)?;
    let file = std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW).open(partial).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("partial is not a regular file".into()); }
    // If a checkpoint is newer, also verify the older snapshot on the same file
    // descriptor, so a newer record cannot silently rewrite an earlier prefix.
    if record != side { verify(&file, &side)?; }
    if let Some(db) = database { if *db != record { verify(&file, db)?; } }
    let hashes = verify(&file, &record)?;
    let map = SegmentMap::restore(&record.segments, hashes)?;
    Ok(PreparedSplit { record, file: Arc::new(file), map: Arc::new(Mutex::new(map)) })
}

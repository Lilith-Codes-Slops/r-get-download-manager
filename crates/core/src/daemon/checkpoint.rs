//! Durable, single-stream checkpoints. These are not automatically replayed.
use super::range::ResumeInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CHECKPOINT_BYTES: u64 = 8 * 1024 * 1024;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RecoveryRecord {
    pub version: u32,
    pub sequence: u64,
    pub task_url: String,
    pub resolved_url: String,
    pub filename: String,
    pub partial_name: String,
    pub etag: String,
    pub total: u64,
    pub downloaded: u64,
    pub prefix_blake3: String,
}

pub(super) struct Checkpoint {
    pub sidecar: PathBuf,
    record: RecoveryRecord,
    hash: blake3::Hasher,
    last_bytes: u64,
    last_time: Instant,
    persistence: Option<(std::sync::Arc<super::persistence::Persistence>, super::control::Tasks)>,
}

impl Checkpoint {
    pub fn new(partial: &Path, task_url: &str, filename: &str, resume: &ResumeInfo) -> std::io::Result<Self> {
        let name = partial.file_name().and_then(|v| v.to_str()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid partial filename"))?;
        let mut sidecar = partial.as_os_str().to_os_string();
        sidecar.push(".meta");
        let hash = blake3::Hasher::new();
        Ok(Self {
            sidecar: sidecar.into(),
            record: RecoveryRecord {
                version: 1, sequence: 0, task_url: task_url.into(), resolved_url: resume.url.to_string(),
                filename: filename.into(), partial_name: name.into(), etag: resume.etag.clone(),
                total: resume.total, downloaded: 0, prefix_blake3: hash.finalize().to_hex().to_string(),
            },
            hash, last_bytes: 0, last_time: Instant::now(), persistence: None,
        })
    }

    pub fn restore(partial: &Path, record: RecoveryRecord, hash: blake3::Hasher) -> Self {
        let mut sidecar = partial.as_os_str().to_os_string();
        sidecar.push(".meta");
        let last_bytes = record.downloaded;
        Self { sidecar: sidecar.into(), record, hash, last_bytes, last_time: Instant::now(), persistence: None }
    }

    pub fn attach_persistence(&mut self, store: std::sync::Arc<super::persistence::Persistence>, tasks: super::control::Tasks) {
        self.persistence = Some((store, tasks));
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.hash.update(bytes);
        self.record.downloaded += bytes.len() as u64;
    }

    pub async fn save(&mut self, file: &tokio::fs::File, force: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !force && self.record.downloaded.saturating_sub(self.last_bytes) < CHECKPOINT_BYTES && self.last_time.elapsed() < CHECKPOINT_INTERVAL {
            return Ok(());
        }
        // The data must reach disk before the metadata can claim it is complete.
        file.sync_data().await?;
        self.record.sequence = self.record.sequence.checked_add(1).ok_or("checkpoint sequence exhausted")?;
        self.record.prefix_blake3 = self.hash.finalize().to_hex().to_string();
        let path = self.sidecar.clone();
        let record = self.record.clone();
        let sidecar_record = record.clone();
        tokio::task::spawn_blocking(move || write_atomic(&path, &sidecar_record)).await??;
        if let Some((store, tasks)) = &self.persistence { store.save_recovery(tasks, &record).await?; }
        self.last_bytes = self.record.downloaded;
        self.last_time = Instant::now();
        Ok(())
    }
}

pub(super) fn write_atomic(path: &Path, record: &RecoveryRecord) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let temp = PathBuf::from(temp);
    let result = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
        let mut file = opts.open(&temp)?;
        serde_json::to_writer(&mut file, record).map_err(std::io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        #[cfg(unix)]
        std::fs::File::open(path.parent().ok_or_else(|| std::io::Error::other("sidecar has no directory"))?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temp); }
    result
}

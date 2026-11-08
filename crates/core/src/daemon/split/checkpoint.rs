//! Version 2 split checkpoint: each range has its own verified prefix hash.
use super::segments::SegmentMap;
use crate::daemon::{control::Tasks, persistence::Persistence};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::daemon) struct SegmentRecord {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::daemon) struct SplitRecord {
    pub version: u32,
    pub sequence: u64,
    pub task_url: String,
    pub resolved_url: String,
    pub filename: String,
    pub partial_name: String,
    pub etag: String,
    pub total: u64,
    pub requested_connections: u8,
    pub segments: Vec<SegmentRecord>,
}

pub(super) struct SplitCheckpoint {
    pub sidecar: PathBuf,
    pub record: SplitRecord,
}

impl SplitCheckpoint {
    /// The caller owns a task-matched partial and must create an initial
    /// checkpoint before permitting writes. The snapshot includes only writes
    /// that have completed and been committed under the map lock.
    pub async fn save(&mut self, map: &Mutex<SegmentMap>, file: &Arc<std::fs::File>, store: &Persistence, tasks: &Tasks) -> Result<(), String> {
        let segments = map.lock().await.records();
        self.record.sequence = self.record.sequence.checked_add(1).ok_or("split checkpoint sequence exhausted")?;
        self.record.segments = segments;
        let handle = file.clone();
        tokio::task::spawn_blocking(move || handle.sync_data()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        #[cfg(feature = "checkpoint-test-hooks")]
        super::test_hooks::stop_at("data_synced", &self.record).await?;
        let parent = self.sidecar.parent().ok_or("split sidecar has no parent")?;
        let partial = parent.join(&self.record.partial_name);
        super::recovery::validate_record(&self.record, &partial)?;
        // Never bind metadata to a different file or a substituted symlink.
        use std::os::unix::fs::MetadataExt;
        let handle_meta = file.metadata().map_err(|e| e.to_string())?;
        let path_meta = std::fs::symlink_metadata(&partial).map_err(|e| e.to_string())?;
        if !handle_meta.is_file() || !path_meta.is_file() || handle_meta.dev() != path_meta.dev() || handle_meta.ino() != path_meta.ino() {
            return Err("split partial path no longer identifies the open file".into());
        }
        let mut expected = partial.as_os_str().to_os_string();
        expected.push(".meta");
        if self.sidecar != PathBuf::from(expected) { return Err("unexpected split sidecar path".into()); }
        let owned = tasks.lock().await.get(&self.record.task_url)
            .is_some_and(|task| task.partial_name.as_deref() == Some(self.record.partial_name.as_str()));
        if !owned { return Err("split checkpoint no longer belongs to the task".into()); }
        // Recovery deliberately bounds sidecar reads, so never publish an
        // oversized checkpoint that a restart would be unable to load.
        if serde_json::to_vec(&self.record).map_err(|e| e.to_string())?.len() + 1 > 64 * 1024 {
            return Err("split checkpoint exceeds the sidecar size limit".into());
        }
        let path = self.sidecar.clone();
        let record = self.record.clone();
        tokio::task::spawn_blocking(move || crate::daemon::checkpoint::write_atomic_with(&path, &record, || {
            #[cfg(feature = "checkpoint-test-hooks")]
            super::test_hooks::stop_at_rename(&record)?;
            Ok(())
        }))
            .await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        #[cfg(feature = "checkpoint-test-hooks")]
        super::test_hooks::stop_at("sidecar_committed", &self.record).await?;
        store.save_split_recovery(tasks, &self.record).await.map_err(|e| e.to_string())?;
        #[cfg(feature = "checkpoint-test-hooks")]
        super::test_hooks::stop_at("sqlite_committed", &self.record).await?;
        Ok(())
    }
}

//! Complete a previously published stage and clean its now-empty owned stage.
//! This operation is not wired to torrent downloads or automatic recovery.
use super::{
    control::Tasks,
    persistence::Persistence,
    torrent_publish_commit,
    torrent_publish_inspection::{self, Inspection},
    torrent_publish_proof,
    torrent_stage_publish::PublishedStage,
    torrent_stage_store,
};
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[allow(dead_code)]
pub(super) struct CompletionReport {
    pub cleanup_warning: Option<String>,
}

impl PublishedStage {
    /// Rename has already succeeded. An error means completion could not be
    /// recorded, not that the published directory should be deleted or retried.
    /// The caller must exclude external writes and all lifecycle paths must use
    /// the same registry. This moves the lifecycle, persistence and task guards
    /// into the blocking operation so cancellation cannot expose a partial commit.
    #[allow(dead_code)]
    pub(super) async fn complete(
        self,
        store: &Persistence,
        tasks: &Tasks,
        root: &Path,
    ) -> Result<CompletionReport> {
        if store.database_path() != self.stage.reservation.database {
            return Err("publication database changed".into());
        }
        let gate = store.gate.clone().lock_owned().await;
        let mut tasks = tasks.clone().lock_owned().await;
        let task = tasks
            .get(&self.intent.stage.task_id)
            .ok_or("torrent task missing after publication")?;
        if task.state != "finalizing" {
            return Err("torrent is not finalizing after publication".into());
        }
        let mut completed = task.clone();
        completed.final_path = Some(
            root.join(&self.intent.destination)
                .to_str()
                .ok_or("non-UTF-8 destination root")?
                .into(),
        );
        completed.filename = Some(self.intent.destination.clone());
        completed.state = "completed".into();
        completed.speed_bps = 0;
        completed
            .torrent
            .as_mut()
            .ok_or("torrent identity missing")?
            .publication_warning = self.sync_warning.clone();
        let database = store.database_path();
        let root = root.to_owned();
        tokio::task::spawn_blocking(move || -> Result<CompletionReport> {
            let _gate = gate;
            let task = tasks.get(&self.intent.stage.task_id).ok_or("torrent task missing after publication")?;
            if !torrent_publish_proof::is_verified(&database, &self.intent, task)? {
                return Err("publication evidence missing or changed after rename".into());
            }
            if torrent_publish_inspection::inspect(&self.stage.area, &self.stage.root, &self.intent, task)? != Inspection::DestinationLayoutMatches {
                return Err("published destination is not proven to match".into());
            }
            torrent_publish_commit::commit(&database, &root, &self.intent, &completed, Some(&self.stage.root))?;
            #[cfg(feature = "torrent-test-hooks")]
            super::torrent_test_hooks::pause("after-completion")?;
            tasks.insert(completed.url.clone(), completed);
            // Once completion is durable, a cleanup failure is only a warning.
            // Keep the retained row on failure for later reconciliation.
            let cleanup_warning = torrent_stage_store::cleanup_recorded(&database, &self.stage.area,
                &self.intent.stage.task_id, &self.intent.stage.name, true).err().map(|error| error.to_string());
            if let Some(error) = &cleanup_warning { tracing::warn!(task_id = %self.intent.stage.task_id, %error, "completed torrent stage cleanup failed"); }
            Ok(CompletionReport { cleanup_warning })
        }).await?
    }
}

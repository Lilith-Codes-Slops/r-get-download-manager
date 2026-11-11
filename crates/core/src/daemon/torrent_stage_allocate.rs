//! Guarded stage allocation, not connected to download ingress.
use super::{torrent_manifest_store::ManifestReservation, torrent_stage_store};
use crate::protocols::torrent::{
    staging_cleanup,
    staging_tree::{StageArea, StageRecord},
};
use std::fs::File;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Stage identity and opened payload, protected until the caller is ready to
/// transfer responsibility to another operation using the same lifecycle lock.
/// The engine must not start before allocation and confirmation both succeed.
#[allow(dead_code)]
pub(super) struct AllocatedStage {
    pub area: StageArea,
    pub root: File,
    pub record: StageRecord,
    pub payload: File,
    pub reservation: ManifestReservation,
}

impl ManifestReservation {
    /// The root must be an already-open trusted download directory. Moves the
    /// guard into the blocking task so aborting the waiter cannot unlock an
    /// in-progress filesystem operation. An abandoned successful result leaves
    /// a recorded stage, not permission to delete unrecorded files.
    #[allow(dead_code)]
    pub(super) async fn allocate(self, root: File) -> Result<AllocatedStage> {
        tokio::task::spawn_blocking(move || allocate(self, root)).await?
    }
}

fn allocate(reservation: ManifestReservation, root: File) -> Result<AllocatedStage> {
    let db = &reservation.database;
    if !torrent_stage_store::has_reservation(db, &reservation.task_id, &reservation.name)? {
        return Err("staging reservation missing or changed".into());
    }
    let area = StageArea::open(&root)?;
    // Never adopt an existing name; on an error the reservation remains for
    // conservative reconciliation even if creation left no directory.
    let record = area.create_reserved(&reservation.name, &reservation.task_id)?;
    if let Err(error) = torrent_stage_store::confirm(db, &record) {
        // Only an unchanged unconfirmed reservation plus a verified marker and
        // inode permits removal. Keep the reservation even after successful
        // removal: forgetting a reserved row needs its own crash-safe protocol.
        let cleanup =
            match torrent_stage_store::has_reservation(db, &reservation.task_id, &reservation.name)
            {
                Ok(true) => staging_cleanup::cleanup(&area, &record).map_err(|e| e.to_string()),
                Ok(false) => Err("reservation changed".into()),
                Err(e) => Err(e.to_string()),
            };
        return Err(format!(
            "stage confirmation failed for {}: {error}; cleanup: {}",
            record.name,
            cleanup
                .err()
                .unwrap_or_else(|| "completed; reservation retained".into())
        )
        .into());
    }
    let payload = match area.create_payload(&record) {
        Ok(payload) => payload,
        Err(error) => {
            let cleanup = torrent_stage_store::cleanup_recorded(
                db,
                &area,
                &record.task_id,
                &record.name,
                false,
            );
            return Err(format!(
                "payload creation failed for {}: {error}; cleanup: {}",
                record.name,
                cleanup
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "completed".into())
            )
            .into());
        }
    };
    Ok(AllocatedStage {
        area,
        root,
        record,
        payload,
        reservation,
    })
}

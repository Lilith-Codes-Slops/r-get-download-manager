//! Conservative startup recovery for recorded, unfinished staging trees.
//! A future publication path MUST persist the retained/verification state
//! before verification starts; recovery only deletes rows still in staging.
use super::{persistence::Persistence, torrent_stage_store::{self, State}};
use crate::protocols::torrent::staging_tree::StageArea;
use nix::{fcntl::{openat, OFlag}, sys::stat::Mode};
use std::{fs::{File, OpenOptions}, io, os::unix::fs::OpenOptionsExt, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Report { pub cleaned: usize, pub retained: usize, pub unresolved: usize }

/// Run only after acquiring the daemon singleton lock and before starting any
/// new torrent workers. Unrecorded, retained and ambiguous trees are untouched.
pub(super) fn recover(database: &Path, root: &Path) -> Result<Report> {
    let entries = torrent_stage_store::load(database)?;
    let mut report = Report::default();
    if entries.is_empty() { return Ok(report); }
    let active = entries.iter().any(|entry| entry.state == State::Staging);
    let area = if active {
        match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)
            .and_then(|root| StageArea::open_existing(&root)) {
            Ok(area) => Some(area),
            Err(error) => { tracing::warn!(%error, root = %root.display(), "staging parent unavailable; recovery retains records"); None }
        }
    } else { None };
    for entry in entries {
        match entry.state {
            State::Retained => { report.retained += 1; },
            State::Reserved => {
                report.unresolved += 1;
                tracing::warn!(task_id = %entry.task_id, stage = %entry.name, "unconfirmed staging reservation retained for inspection");
            },
            State::Staging => {
                let Some(area) = &area else { report.unresolved += 1; continue; };
                let Some(identity) = entry.identity.as_ref() else { report.unresolved += 1; continue; };
                // Missing payload is ambiguous: it may never have been created,
                // or it may have been renamed before the DB was updated.
                let eligible = area.verify(identity).and_then(|stage| payload_present(&stage));
                match eligible {
                    Ok(true) => match torrent_stage_store::cleanup_recorded(database, area, &entry.task_id, &entry.name, false) {
                        Ok(()) => { report.cleaned += 1; tracing::info!(task_id = %entry.task_id, "cleaned interrupted torrent staging"); }
                        Err(error) => { report.unresolved += 1; tracing::warn!(task_id = %entry.task_id, %error, "staging cleanup failed; record retained"); }
                    },
                    Ok(false) => { report.unresolved += 1; tracing::warn!(task_id = %entry.task_id, stage = %entry.name, "missing staged payload may indicate publication; record retained"); }
                    Err(error) => { report.unresolved += 1; tracing::warn!(task_id = %entry.task_id, %error, "staging ownership or payload check failed; record retained"); }
                }
            },
        }
    }
    Ok(report)
}

fn payload_present(stage: &File) -> io::Result<bool> {
    match openat(stage, "payload", OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, Mode::empty()) {
        Ok(_) => Ok(true),
        Err(nix::errno::Errno::ENOENT) => Ok(false),
        Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

pub(super) async fn startup(store: &Persistence) {
    let Some(home) = std::env::var_os("HOME") else { tracing::warn!("HOME missing; torrent staging recovery skipped"); return; };
    let database = store.database_path(); let root = std::path::PathBuf::from(home).join("Downloads");
    match tokio::task::spawn_blocking(move || recover(&database, &root)).await {
        Ok(Ok(report)) => tracing::info!(cleaned = report.cleaned, retained = report.retained, unresolved = report.unresolved, "torrent staging recovery"),
        Ok(Err(error)) => tracing::warn!(%error, "torrent staging recovery failed; records retained"),
        Err(error) => tracing::warn!(%error, "torrent staging recovery task failed; records retained"),
    }
}

//! Single-file v1 torrent download; private staging and no-clobber publication.
use super::{
    bandwidth::BandwidthLimiter,
    control::{Controls, Tasks, TransferControl},
    persistence::Persistence,
};
use crate::protocols::torrent::{
    concurrency_queue,
    gated_storage::GatedFilesystemFactory,
    metadata,
    metrics::MetricsTracker,
    picker_capabilities,
    session::{download_only_options, with_discovery},
    source,
    stall_detector::StallEvent,
    storage_gate::StorageGate,
    task_identity::{self, Prepared},
};
use downloader_ipc::{DaemonEvent, DownloadInfo, TorrentIdentity};
use librqbit::{
    storage::StorageFactoryExt, AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent,
    Session,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn add(
    source: String,
    connections: u8,
    tasks: Tasks,
    controls: Controls,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
) -> DaemonEvent {
    add_inner(source, connections, None, tasks, controls, limiter, store).await
}

/// Internal confirmed admission is deliberately not exposed by the IPC server
/// until retry and collision state can be verified across all torrent paths.
#[allow(dead_code)]
pub(super) async fn add_confirmed(
    source: String,
    connections: u8,
    confirmed: super::confirmed_start::ConfirmedStart,
    tasks: Tasks,
    controls: Controls,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
) -> DaemonEvent {
    add_inner(
        source,
        connections,
        Some(confirmed),
        tasks,
        controls,
        limiter,
        store,
    )
    .await
}

async fn add_inner(
    source: String,
    connections: u8,
    confirmed: Option<super::confirmed_start::ConfirmedStart>,
    tasks: Tasks,
    controls: Controls,
    limiter: Arc<BandwidthLimiter>,
    store: Arc<Persistence>,
) -> DaemonEvent {
    let Some(classified) = source::classify(&source) else {
        return DaemonEvent::TaskFailed {
            task_id: source,
            error: "invalid torrent source".into(),
        };
    };
    let prepared =
        match tokio::time::timeout(Duration::from_secs(60), task_identity::prepare(&classified))
            .await
        {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                return DaemonEvent::TaskFailed {
                    task_id: source,
                    error: error.to_string(),
                }
            }
            Err(_) => {
                return DaemonEvent::TaskFailed {
                    task_id: source,
                    error: "metadata timeout".into(),
                }
            }
        };
    let multi_file = match &prepared.source {
        task_identity::PreparedSource::Metainfo(bytes) => {
            crate::protocols::torrent::webseed_support::log_metainfo(bytes);
            match crate::protocols::torrent::multi_file_metadata::preflight_local(bytes) {
                Ok(manifest) => manifest.is_some(),
                Err(error) => {
                    return DaemonEvent::TaskFailed {
                        task_id: source,
                        error: format!("invalid multi-file metadata: {error}"),
                    }
                }
            }
        }
        task_identity::PreparedSource::Magnet(_) => {
            tracing::info!("webseed declarations unavailable from magnet metadata; P2P only");
            false
        }
    };
    let admission = store.gate.lock().await;
    let mut guard = tasks.lock().await;
    let duplicate = guard
        .values()
        .find(|task| {
            task.torrent
                .as_ref()
                .and_then(|torrent| torrent.info_hash.as_deref())
                == Some(prepared.info_hash.as_str())
                || (task.torrent.is_none() && task.url == source)
        })
        .cloned();
    if let Some(task) = &duplicate {
        if confirmed.is_some() {
            return DaemonEvent::TaskFailed {
                task_id: task.url.clone(),
                error: "torrent already exists; remove the existing task first".into(),
            };
        }
        // A legacy add must not overwrite a confirmed name or stage on retry.
        if task.confirmed_filename.is_some() {
            return DaemonEvent::TaskFailed {
                task_id: task.url.clone(),
                error: "confirmed task already exists; use its retry control or remove it first"
                    .into(),
            };
        }
        let failed = task.state.starts_with("failed:") || task.state == "canceled";
        let same_source = task.torrent.as_ref().is_some_and(|t| t.source == source);
        if failed
            && same_source
            && task
                .torrent
                .as_ref()
                .and_then(|t| t.stage_name.as_ref())
                .is_some()
        {
            return DaemonEvent::TaskFailed { task_id: task.url.clone(), error: "remove the failed task with keep or delete before retrying its retained partial".into() };
        }
    }
    let retry = duplicate
        .as_ref()
        .is_some_and(|task| task.state.starts_with("failed:") || task.state == "canceled");
    let (id, created_at, seed) = match duplicate {
        Some(task)
            if (task.torrent.as_ref().is_some_and(|t| t.source == source)
                || (task.torrent.is_none() && task.url == source))
                && (task.state.starts_with("failed:") || task.state == "canceled") =>
        {
            let created_at = task
                .torrent
                .as_ref()
                .map(|t| t.created_at)
                .unwrap_or_else(task_identity::created_at);
            let seed = task.torrent.as_ref().is_some_and(|t| t.seed);
            (task.url, created_at, seed)
        }
        Some(task) => {
            return DaemonEvent::TaskStateChanged {
                task_id: task.url.clone(),
                state: format!("existing: {}", task.state),
                destination_path: super::destination::path(&task),
            }
        }
        None => {
            let id = loop {
                let candidate = task_identity::new_id();
                if !guard.contains_key(&candidate) {
                    break candidate;
                }
            };
            (id, task_identity::created_at(), false)
        }
    };
    let task = DownloadInfo {
        url: id.clone(),
        state: "queued".into(),
        filename: None,
        downloaded: 0,
        total: None,
        speed_bps: 0,
        final_path: None,
        destination_path: None,
        suggested_filename: None,
        confirmed_filename: confirmed.as_ref().map(|request| request.filename.clone()),
        torrent: Some(TorrentIdentity {
            source,
            info_hash: Some(prepared.info_hash.clone()),
            stage_name: None,
            seed,
            created_at,
            metrics: None,
            root_name: None,
            files: None,
            publication_warning: None,
        }),
        resume_supported: false,
        partial_name: None,
        requested_connections: Some(connections),
        active_connections: None,
        segment_count: None,
        connection_note: Some("Torrent: pause, resume and speed limits are not available".into()),
        live_connections_supported: false,
    };
    let binding = match confirmed {
        Some(request) => {
            store
                .bind_task_directory_locked_at(&task, false, request.folder)
                .await
        }
        None => store.bind_task_directory_locked(&task, retry).await,
    };
    if let Err(error) = binding {
        return DaemonEvent::TaskFailed {
            task_id: id,
            error: format!("could not bind task directory: {error}"),
        };
    }
    guard.insert(id.clone(), task);
    let ticket = concurrency_queue::global().register(id.clone());
    let (tx, rx) = watch::channel(TransferControl::Running);
    controls.lock().await.insert(id.clone(), tx);
    let task_id = id.clone();
    let worker_tasks = tasks.clone();
    let worker_store = store.clone();
    let worker_limiter = limiter.clone();
    // Do not let removal and re-admission overtake the worker's creation.
    tokio::spawn(async move {
        let tasks = worker_tasks;
        let store = worker_store;
        let limiter = worker_limiter;
        let mut receiver = rx.clone();
        let acquired =
            super::torrent_queue::wait_for_slot(&task_id, &tasks, &ticket, &mut receiver).await;
        drop(ticket);
        let (result, permit) = match acquired {
            Ok(permit) => {
                let result = if multi_file {
                    super::torrent_multi_transfer::run(
                        &task_id,
                        connections,
                        &tasks,
                        &store,
                        &limiter,
                        rx.clone(),
                        prepared,
                    )
                    .await
                } else {
                    run(&task_id, connections, &tasks, &store, rx.clone(), prepared).await
                };
                (result, Some(permit))
            }
            Err(error) => (Err(error), None),
        };
        {
            let mut guard = tasks.lock().await;
            if let Some(task) = guard.get_mut(&task_id) {
                task.speed_bps = 0;
                task.active_connections = None;
                if let Err(error) = result {
                    if error
                        .downcast_ref::<super::torrent_multi_transfer::StorageQuiescenceFailed>()
                        .is_some()
                    {
                        task.state = "quiescence-unknown".into();
                        task.connection_note = Some(error.to_string());
                    } else if *rx.borrow() == TransferControl::Canceled {
                        task.state = "canceled".into();
                    } else if error
                        .downcast_ref::<super::torrent_multi_transfer::PendingPublication>()
                        .is_some()
                    {
                        task.connection_note = Some(error.to_string());
                    } else {
                        task.state = format!("failed: {error}");
                    }
                } else if *rx.borrow() == TransferControl::Canceled {
                    task.state = "canceled".into();
                }
            }
        }
        drop(permit);
        super::progress_events::notify();
        if let Err(error) = store.save(&tasks, &limiter).await {
            eprintln!("torrent queue save failed: {error}");
        }
    });
    drop(guard);
    drop(admission);
    DaemonEvent::TaskStateChanged {
        task_id: id,
        state: "queued".into(),
        destination_path: None,
    }
}

async fn run(
    id: &str,
    connections: u8,
    tasks: &Tasks,
    store: &Persistence,
    mut control: watch::Receiver<TransferControl>,
    prepared: Prepared,
) -> Result<()> {
    let root = store.task_directory(id).await?;
    let confirmed = tasks
        .lock()
        .await
        .get(id)
        .map_or(true, |task| task.confirmed_filename.is_some());
    if confirmed {
        if !tokio::fs::metadata(&root).await?.is_dir() {
            return Err("bound download root is not a directory".into());
        }
    } else {
        tokio::fs::create_dir_all(&root).await?;
    }
    let stage = make_stage(&root).await?;
    super::torrent_cleanup::write_marker(&stage, id).await?;
    {
        let mut guard = tasks.lock().await;
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        if let Some(identity) = &mut task.torrent {
            identity.stage_name = stage
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
        }
    }
    picker_capabilities::log();
    let database = Persistence::default_path()?;
    let state = database.parent().ok_or("state database has no directory")?;
    let cache = state
        .join("torrent-dht")
        .join(&prepared.info_hash)
        .join("dht.json");
    let session = match Session::new_with_opts(stage.clone(), with_discovery(cache)).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!("torrent persistent discovery unavailable; falling back: {error:#}");
            Session::new_with_opts(stage.clone(), download_only_options())
                .await
                .map_err(|e| {
                    format!(
                        "torrent session: {e:#}; staging retained at {}",
                        stage.display()
                    )
                })?
        }
    };
    let gate = StorageGate::new();
    let result = download(
        &session,
        &gate,
        &root,
        &stage,
        id,
        connections,
        tasks,
        &mut control,
        prepared,
    )
    .await;
    session.stop().await;
    // stop() alone does not join all engine tasks; close the storage gate and
    // wait for admitted I/O before publication or cleanup.
    let closer = gate.clone();
    match tokio::task::spawn_blocking(move || closer.close_and_wait()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(
                super::torrent_multi_transfer::StorageQuiescenceFailed(format!(
                    "{error}; stage retained at {}",
                    stage.display()
                ))
                .into(),
            )
        }
        Err(error) => {
            return Err(
                super::torrent_multi_transfer::StorageQuiescenceFailed(format!(
                    "barrier task failed: {error}; stage retained at {}",
                    stage.display()
                ))
                .into(),
            )
        }
    }
    #[cfg(feature = "torrent-test-hooks")]
    {
        let rejected = crate::protocols::torrent::gated_storage::probe_late_write(gate.clone());
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            super::torrent_test_hooks::late_write(rejected)?;
            super::torrent_test_hooks::pause("after-barrier")
        })
        .await??;
    }
    let staged = match result {
        Ok(staged) => staged,
        Err(error) => {
            return Err(format!(
                "{error}; incomplete data, if any, is retained in {}",
                stage.display()
            )
            .into());
        }
    };
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == TransferControl::Canceled {
            return Err("torrent canceled before publication".into());
        }
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        task.state = "finalizing".into();
    }
    sync_staged(&staged).await?;
    let confirmed_name = tasks
        .lock()
        .await
        .get(id)
        .and_then(|task| task.confirmed_filename.clone());
    let name = staged.file_name().ok_or("torrent filename missing")?;
    let proposed = match &confirmed_name {
        Some(name) => {
            super::confirmed_start::validate_filename(name)?;
            root.join(name)
        }
        None => root.join(name),
    };
    let dest =
        super::collision::publish_for_task(&staged, &proposed, confirmed_name.is_some()).await?;
    {
        let mut guard = tasks.lock().await;
        let task = guard
            .get_mut(id)
            .ok_or("torrent task removed during publication")?;
        task.filename = dest.file_name().map(|s| s.to_string_lossy().into_owned());
        task.final_path = Some(dest.to_string_lossy().into_owned());
        // Remain finalizing until stage cleanup is finished; removal must not
        // observe completion while this worker is still changing the stage.
        if let Some(metrics) = task
            .torrent
            .as_ref()
            .and_then(|identity| identity.metrics.as_ref())
        {
            crate::protocols::torrent::diagnostics::log_completion(id, metrics);
        }
    }
    let cleaned = match tokio::fs::remove_file(&staged).await {
        Ok(()) => match tokio::fs::remove_file(stage.join(super::torrent_cleanup::MARKER)).await {
            Ok(()) => match tokio::fs::remove_dir(&stage).await {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("torrent staging directory cleanup: {error}");
                    false
                }
            },
            Err(error) => {
                eprintln!("torrent marker cleanup: {error}");
                false
            }
        },
        Err(error) => {
            eprintln!("torrent staging cleanup: {error}");
            false
        }
    };
    if let Some(task) = tasks.lock().await.get_mut(id) {
        if cleaned {
            if let Some(identity) = &mut task.torrent {
                identity.stage_name = None;
            }
        } else {
            task.connection_note = Some(format!(
                "published file; staging cleanup incomplete at {}",
                stage.display()
            ));
        }
        task.state = "completed".into();
    }
    Ok(())
}

async fn download(
    session: &Arc<Session>,
    gate: &StorageGate,
    root: &Path,
    stage: &Path,
    id: &str,
    connections: u8,
    tasks: &Tasks,
    control: &mut watch::Receiver<TransferControl>,
    prepared: Prepared,
) -> Result<PathBuf> {
    let info_hash = prepared.info_hash;
    let metadata_started = Instant::now();
    let meta = tokio::select! {
        result = metadata::resolve_prepared(session, prepared.source) => result?,
        _ = control.changed() => return Err("torrent canceled".into()),
    };
    let metadata_known = Instant::now();
    let mut metrics = MetricsTracker::new(
        metadata_known.saturating_duration_since(metadata_started),
        metadata_known,
        Duration::from_secs(30),
    );
    if meta.info_hash.as_string() != info_hash {
        return Err("torrent info-hash changed during metadata resolution".into());
    }
    if meta.info.info().private {
        return Err("private torrents are not supported in Phase 1".into());
    }
    let filename = meta
        .info
        .name()
        .ok_or("torrent filename missing")?
        .to_string();
    let confirmed_name = tasks
        .lock()
        .await
        .get(id)
        .and_then(|task| task.confirmed_filename.clone());
    let proposed = if let Some(name) = &confirmed_name {
        super::confirmed_start::validate_filename(name)?;
        root.join(name)
    } else {
        super::collision::available(&root.join(&filename)).await?
    };
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == TransferControl::Canceled {
            return Err("torrent canceled".into());
        }
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        task.suggested_filename = Some(filename.clone());
        task.filename = proposed
            .file_name()
            .map(|s| s.to_string_lossy().into_owned());
        task.destination_path = Some(proposed.to_string_lossy().into_owned());
        task.total = meta.info.info().length;
        if let Some(identity) = &mut task.torrent {
            identity.metrics = Some(metrics.snapshot.clone());
            identity.files = Some(crate::protocols::torrent::file_priorities::single_file(
                &filename,
                meta.info
                    .info()
                    .length
                    .ok_or("single file length missing")?,
            ));
        }
        task.state = "peers".into();
    }
    super::progress_events::notify();
    let opts = AddTorrentOptions {
        initial_peers: Some(meta.seen_peers),
        peer_limit: Some(connections as usize),
        storage_factory: Some(GatedFilesystemFactory { gate: gate.clone() }.boxed()),
        ..Default::default()
    };
    let added = tokio::select! {
        result = session.add_torrent(AddTorrent::from_bytes(meta.torrent_bytes), Some(opts)) => result.map_err(|e| format!("torrent start: {e:#}"))?,
        _ = control.changed() => return Err("torrent canceled".into()),
    };
    let handle = match added {
        AddTorrentResponse::Added(_, handle) => handle,
        _ => return Err("torrent already managed".into()),
    };
    monitor(&handle, id, tasks, control, &mut metrics).await?;
    Ok(stage.join(filename))
}

pub(super) async fn monitor(
    handle: &Arc<ManagedTorrent>,
    id: &str,
    tasks: &Tasks,
    control: &mut watch::Receiver<TransferControl>,
    metrics: &mut MetricsTracker,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    let mut no_peers_since = Instant::now();
    let mut last = Instant::now();
    let mut last_bytes = 0u64;
    loop {
        tokio::select! {
            _ = timer.tick() => {},
            _ = control.changed() => {
                metrics.observe(Instant::now(), false, None, None, false);
                update_metrics(tasks, id, metrics).await;
                return Err("torrent canceled".into());
            },
        }
        let stats = handle.stats();
        if let Some(error) = stats.error {
            metrics.observe(Instant::now(), false, None, None, false);
            update_metrics(tasks, id, metrics).await;
            return Err(format!("torrent engine: {error}").into());
        }
        let bytes = if matches!(stats.state, librqbit::TorrentStatsState::Live) {
            stats.progress_bytes
        } else {
            0
        };
        let live = stats.live.as_ref();
        let peers = live.map(|live| live.snapshot.peer_stats.live).unwrap_or(0);
        if peers > 0 {
            no_peers_since = Instant::now();
        }
        let now = Instant::now();
        let fetched = live.map(|live| live.snapshot.fetched_bytes);
        let pieces = live.map(|live| live.snapshot.downloaded_and_checked_pieces);
        let active = peers > 0
            && fetched.is_some_and(|n| n > 0)
            && (!stats.finished || metrics.needs_recovery());
        let transition = metrics.observe(now, active, fetched, pieces, stats.finished);
        let elapsed = now.duration_since(last).as_secs_f64();
        let speed = if elapsed > 0.0 {
            (bytes.saturating_sub(last_bytes) as f64 / elapsed) as u64
        } else {
            0
        };
        last = now;
        last_bytes = bytes;
        {
            let mut guard = tasks.lock().await;
            if *control.borrow() == TransferControl::Canceled {
                return Err("torrent canceled".into());
            }
            let task = guard.get_mut(id).ok_or("torrent task removed")?;
            task.downloaded = bytes;
            task.total = Some(stats.total_bytes);
            task.speed_bps = speed;
            task.connection_note = Some(format!(
                "Torrent peers: {peers}; pause/resume and speed limits unavailable"
            ));
            if let Some(identity) = &mut task.torrent {
                identity.metrics = Some(metrics.snapshot.clone());
            }
            task.state = if stats.finished {
                "finished"
            } else if peers > 0 && fetched.is_some_and(|n| n > 0) {
                "downloading"
            } else if peers > 0 {
                "connecting"
            } else {
                "peers"
            }
            .into();
        }
        if let Some((event, duration_ms)) = transition {
            let count = metrics.snapshot.stall_count;
            let event = match event {
                StallEvent::Stalled => {
                    tracing::warn!(
                        task_id = id,
                        count,
                        "torrent stalled; no automatic peer drop available"
                    );
                    DaemonEvent::TorrentStalled {
                        task_id: id.into(),
                        stall_count: count,
                    }
                }
                StallEvent::Recovered => {
                    tracing::info!(
                        task_id = id,
                        count,
                        duration_ms,
                        "torrent resumed receiving data"
                    );
                    DaemonEvent::TorrentRecovered {
                        task_id: id.into(),
                        stall_count: count,
                        duration_ms,
                    }
                }
            };
            super::torrent_events::send(event);
        }
        super::progress_events::notify();
        if stats.finished {
            return Ok(());
        }
        if no_peers_since.elapsed() >= Duration::from_secs(120) {
            return Err("no peers after 120 seconds".into());
        }
    }
}

async fn update_metrics(tasks: &Tasks, id: &str, metrics: &MetricsTracker) {
    if let Some(task) = tasks.lock().await.get_mut(id) {
        if let Some(identity) = &mut task.torrent {
            identity.metrics = Some(metrics.snapshot.clone());
        }
    }
}

async fn make_stage(root: &Path) -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".rget-torrent-{}-{n}", std::process::id()));
        let candidate = path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(candidate)
        })
        .await?;
        match result {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err("unable to create a unique torrent staging directory".into())
}

/// Sync the completed regular file before the durable publication link.
async fn sync_staged(path: &Path) -> Result<()> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "torrent staging file is not regular",
            ));
        }
        file.sync_all()
    })
    .await??;
    Ok(())
}

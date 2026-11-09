//! Single-file v1 torrent download; private staging and no-clobber publication.
use super::{bandwidth::BandwidthLimiter, control::{Controls, Tasks, TransferControl}, persistence::Persistence};
use crate::protocols::torrent::{metadata, session::{download_only_options, with_discovery}, source, task_identity::{self, Prepared}};
use downloader_ipc::{DaemonEvent, DownloadInfo, TorrentIdentity};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session};
use std::{path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn add(source: String, connections: u8, tasks: Tasks, controls: Controls,
    limiter: Arc<BandwidthLimiter>, store: Arc<Persistence>) -> DaemonEvent {
    let Some(classified) = source::classify(&source) else {
        return DaemonEvent::TaskFailed { task_id: source, error: "invalid torrent source".into() };
    };
    let prepared = match tokio::time::timeout(Duration::from_secs(60), task_identity::prepare(&classified)).await {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => return DaemonEvent::TaskFailed { task_id: source, error: error.to_string() },
        Err(_) => return DaemonEvent::TaskFailed { task_id: source, error: "metadata timeout".into() },
    };
    let mut guard = tasks.lock().await;
    let duplicate = guard.values().find(|task| {
        task.torrent.as_ref().and_then(|torrent| torrent.info_hash.as_deref()) == Some(prepared.info_hash.as_str())
            || (task.torrent.is_none() && task.url == source)
    }).cloned();
    if let Some(task) = &duplicate {
        let failed = task.state.starts_with("failed:") || task.state == "canceled";
        let same_source = task.torrent.as_ref().is_some_and(|t| t.source == source);
        if failed && same_source && task.torrent.as_ref().and_then(|t| t.stage_name.as_ref()).is_some() {
            return DaemonEvent::TaskFailed { task_id: task.url.clone(), error: "remove the failed task with keep or delete before retrying its retained partial".into() };
        }
    }
    let (id, created_at, seed) = match duplicate {
        Some(task) if (task.torrent.as_ref().is_some_and(|t| t.source == source)
            || (task.torrent.is_none() && task.url == source)) && (task.state.starts_with("failed:") || task.state == "canceled") => {
            let created_at = task.torrent.as_ref().map(|t| t.created_at).unwrap_or_else(task_identity::created_at);
            let seed = task.torrent.as_ref().is_some_and(|t| t.seed);
            (task.url, created_at, seed)
        }
        Some(task) => return DaemonEvent::TaskStateChanged {
            task_id: task.url.clone(), state: format!("existing: {}", task.state), destination_path: super::destination::path(&task)
        },
        None => {
            let id = loop { let candidate = task_identity::new_id(); if !guard.contains_key(&candidate) { break candidate; } };
            (id, task_identity::created_at(), false)
        },
    };
    let task = DownloadInfo { url: id.clone(), state: "metadata".into(), filename: None,
        downloaded: 0, total: None, speed_bps: 0, final_path: None, destination_path: None,
        suggested_filename: None, torrent: Some(TorrentIdentity { source, info_hash: Some(prepared.info_hash.clone()), stage_name: None, seed, created_at }),
        resume_supported: false, partial_name: None, requested_connections: Some(connections),
        active_connections: None, segment_count: None, connection_note: Some("Torrent: pause, resume and speed limits are not available".into()),
        live_connections_supported: false };
    guard.insert(id.clone(), task);
    let (tx, rx) = watch::channel(TransferControl::Running);
    controls.lock().await.insert(id.clone(), tx);
    drop(guard);
    let task_id = id.clone();
    tokio::spawn(async move {
        let result = run(&task_id, connections, &tasks, rx.clone(), prepared).await;
        {
            let mut guard = tasks.lock().await;
            if let Some(task) = guard.get_mut(&task_id) {
                task.speed_bps = 0; task.active_connections = None;
                if *rx.borrow() == TransferControl::Canceled { task.state = "canceled".into(); }
                else if let Err(error) = result { task.state = format!("failed: {error}"); }
            }
        }
        super::progress_events::notify();
        if let Err(error) = store.save(&tasks, &limiter).await { eprintln!("torrent queue save failed: {error}"); }
    });
    DaemonEvent::TaskStateChanged { task_id: id, state: "metadata".into(), destination_path: None }
}

async fn run(id: &str, connections: u8, tasks: &Tasks, mut control: watch::Receiver<TransferControl>, prepared: Prepared) -> Result<()> {
    let root = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
    tokio::fs::create_dir_all(&root).await?;
    let stage = make_stage(&root).await?;
    super::torrent_cleanup::write_marker(&stage, id).await?;
    {
        let mut guard = tasks.lock().await;
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        if let Some(identity) = &mut task.torrent {
            identity.stage_name = stage.file_name().map(|name| name.to_string_lossy().into_owned());
        }
    }
    let database = Persistence::default_path()?;
    let state = database.parent().ok_or("state database has no directory")?;
    // Only one active task owns a hash; separate folders also prevent different
    // sessions from racing over librqbit's process-wide DHT temp filename.
    let cache = state.join("torrent-dht").join(&prepared.info_hash).join("dht.json");
    let session = match Session::new_with_opts(stage.clone(), with_discovery(cache)).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!("torrent persistent discovery unavailable; falling back: {error:#}");
            Session::new_with_opts(stage.clone(), download_only_options()).await
                .map_err(|e| format!("torrent session: {e:#}; staging retained at {}", stage.display()))?
        }
    };
    let result = download(&session, &root, &stage, id, connections, tasks, &mut control, prepared).await;
    // Always stop the session, including on errors/cancel, before publication.
    session.stop().await;
    let staged = match result {
        Ok(staged) => staged,
        Err(error) => {
            // Leave the owned staging directory for explicit removal or inspection.
            return Err(format!("{error}; incomplete data, if any, is retained in {}", stage.display()).into());
        }
    };
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled before publication".into()); }
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        task.state = "finalizing".into();
    }
    sync_staged(&staged).await?;
    let name = staged.file_name().ok_or("torrent filename missing")?;
    let proposed = root.join(name);
    let dest = super::collision::publish(&staged, &proposed).await?;
    {
        let mut guard = tasks.lock().await;
        let task = guard.get_mut(id).ok_or("torrent task removed during publication")?;
        task.filename = dest.file_name().map(|s| s.to_string_lossy().into_owned());
        task.final_path = Some(dest.to_string_lossy().into_owned());
        task.state = "completed".into();
    }
    // Publication succeeded. Failure to remove the staging link must not undo it.
    if let Err(error) = tokio::fs::remove_file(&staged).await { eprintln!("torrent staging cleanup: {error}"); }
    if let Err(error) = tokio::fs::remove_file(stage.join(super::torrent_cleanup::MARKER)).await { eprintln!("torrent marker cleanup: {error}"); }
    if let Err(error) = tokio::fs::remove_dir(&stage).await { eprintln!("torrent staging directory cleanup: {error}"); }
    if let Some(task) = tasks.lock().await.get_mut(id) {
        if let Some(identity) = &mut task.torrent { identity.stage_name = None; }
    }
    Ok(())
}

async fn download(session: &Arc<Session>, root: &Path, stage: &Path, id: &str, connections: u8,
    tasks: &Tasks, control: &mut watch::Receiver<TransferControl>, prepared: Prepared) -> Result<PathBuf> {
    let info_hash = prepared.info_hash;
    let meta = tokio::select! {
        result = metadata::resolve_prepared(session, prepared.source) => result?,
        _ = control.changed() => return Err("torrent canceled".into()),
    };
    if meta.info_hash.as_string() != info_hash { return Err("torrent info-hash changed during metadata resolution".into()); }
    if meta.info.info().private { return Err("private torrents are not supported in Phase 1".into()); }
    let filename = meta.info.name().ok_or("torrent filename missing")?.to_string();
    let proposed = super::collision::available(&root.join(&filename)).await?;
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled".into()); }
        let task = guard.get_mut(id).ok_or("torrent task removed")?;
        task.suggested_filename = Some(filename.clone());
        task.filename = proposed.file_name().map(|s| s.to_string_lossy().into_owned());
        task.total = meta.info.info().length;
        task.state = "peers".into();
    }
    super::progress_events::notify();
    let opts = AddTorrentOptions { initial_peers: Some(meta.seen_peers), peer_limit: Some(connections as usize), ..Default::default() };
    let added = tokio::select! {
        result = session.add_torrent(AddTorrent::from_bytes(meta.torrent_bytes), Some(opts)) => result.map_err(|e| format!("torrent start: {e:#}"))?,
        _ = control.changed() => return Err("torrent canceled".into()),
    };
    let handle = match added { AddTorrentResponse::Added(_, handle) => handle, _ => return Err("torrent already managed".into()) };
    monitor(&handle, id, tasks, control).await?;
    Ok(stage.join(filename))
}

async fn monitor(handle: &Arc<ManagedTorrent>, id: &str, tasks: &Tasks,
    control: &mut watch::Receiver<TransferControl>) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    let mut no_peers_since = Instant::now();
    let mut last = Instant::now(); let mut last_bytes = 0u64;
    loop {
        tokio::select! {
            _ = timer.tick() => {},
            _ = control.changed() => return Err("torrent canceled".into()),
        }
        let stats = handle.stats();
        if let Some(error) = stats.error { return Err(format!("torrent engine: {error}").into()); }
        // Initialization reports checking progress, not downloaded data.
        let bytes = if matches!(stats.state, librqbit::TorrentStatsState::Live) { stats.progress_bytes } else { 0 };
        let live = stats.live.as_ref();
        let peers = live.map(|live| live.snapshot.peer_stats.live).unwrap_or(0);
        if peers > 0 { no_peers_since = Instant::now(); }
        let now = Instant::now();
        let elapsed = now.duration_since(last).as_secs_f64();
        let speed = if elapsed > 0.0 { (bytes.saturating_sub(last_bytes) as f64 / elapsed) as u64 } else { 0 };
        last = now; last_bytes = bytes;
        {
            let mut guard = tasks.lock().await;
            if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled".into()); }
            let task = guard.get_mut(id).ok_or("torrent task removed")?;
            task.downloaded = bytes; task.total = Some(stats.total_bytes); task.speed_bps = speed;
            task.connection_note = Some(format!("Torrent peers: {peers}; pause/resume and speed limits unavailable"));
            task.state = if stats.finished { "finished" } else if peers > 0 && bytes > 0 { "downloading" }
                else if peers > 0 { "connecting" } else { "peers" }.into();
        }
        super::progress_events::notify();
        if stats.finished { return Ok(()); }
        if no_peers_since.elapsed() >= Duration::from_secs(120) { return Err("no peers after 120 seconds".into()); }
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
            #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
            builder.create(candidate)
        }).await?;
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
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW); }
        let file = options.open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "torrent staging file is not regular"));
        }
        file.sync_all()
    }).await??;
    Ok(())
}

use downloader_ipc::DownloadInfo;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Minimal single-stream HTTP download into the task's root with live progress.
pub(super) async fn download_file(
    url: &str,
    connections: u8,
    root: &std::path::Path,
    tasks: Arc<Mutex<BTreeMap<String, DownloadInfo>>>,
    limiter: Arc<super::bandwidth::BandwidthLimiter>,
    store: Arc<super::persistence::Persistence>,
    force_rename: bool,
    mut control: tokio::sync::watch::Receiver<super::control::TransferControl>,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    if !super::control::wait_running(&tasks, url, &mut control).await {
        return Err("download canceled".into());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let resp = tokio::select! {
        result = client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity").send() => result?,
        _ = control.changed() => return Err("download canceled".into()),
    };
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("HTTP {}", resp.status()).into());
    }

    let total = resp.content_length();
    let mut resume = super::range::detect(&resp);
    let confirmed_name = tasks
        .lock()
        .await
        .get(url)
        .and_then(|task| task.confirmed_filename.clone());
    let metadata_name = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(|cd| {
            cd.split("filename=").nth(1).map(|s| {
                s.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string()
            })
        })
        .or_else(|| {
            resp.url()
                .path_segments()
                .and_then(|mut s| s.next_back())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty());

    // Metadata describes the source; it must not override the user's name.
    // Preserve the legacy fallback for unconfirmed tasks only.
    let metadata_name = if confirmed_name.is_some() {
        metadata_name.filter(|name| safe_filename(name).is_ok())
    } else {
        let name = metadata_name.unwrap_or_else(|| "download.bin".into());
        safe_filename(&name)?;
        Some(name)
    };
    // A split is attempted only when safe resume metadata and at least two
    // minimum-size ranges are available. The original response remains unused
    // so an unsupported probe can fall back to the normal single stream.
    let mut fallback_note: Option<&str> = None;
    #[cfg(target_os = "linux")]
    let split = if connections > 1 {
        let candidate = resume.as_ref().filter(|info| info.total >= 2 * 1024 * 1024);
        match candidate {
            Some(info) => {
                let supported = super::split::live::probe(&client, url, info, &mut control).await?;
                if !supported {
                    resume = None;
                    fallback_note = Some("Using one stream: the server ignored or rejected the byte-range probe; safe resume is unavailable.");
                }
                supported
            }
            None => {
                fallback_note = Some(if resume.is_some() {
                    "Using one stream: the file is smaller than the minimum size for split downloads."
                } else {
                    "Using one stream: verified byte-range support, a usable validator, or a known length is unavailable."
                });
                false
            }
        }
    } else {
        false
    };
    #[cfg(not(target_os = "linux"))]
    let split = {
        if connections > 1 {
            fallback_note =
                Some("Using one stream: split downloads are not supported on this platform.");
        }
        false
    };
    {
        let mut guard = tasks.lock().await;
        if *control.borrow() == super::control::TransferControl::Canceled
            || !guard.contains_key(url)
        {
            return Err("download canceled".into());
        }
        if let Some(task) = guard.get_mut(url) {
            task.suggested_filename = metadata_name.clone();
            task.total = total;
            task.resume_supported = resume.is_some();
            task.active_connections = Some(if split { 0 } else { 1 });
            if connections > 1 && !split {
                task.connection_note = fallback_note.map(str::to_owned);
            }
            task.live_connections_supported = split;
            if split {
                if let Some(info) = &resume {
                    task.segment_count =
                        Some((info.total / (1024 * 1024)).min(connections as u64) as u16);
                }
            }
        }
    }
    let selected_name = safe_filename(
        confirmed_name
            .as_deref()
            .or(metadata_name.as_deref())
            .ok_or("download filename unavailable")?,
    )?;
    let dest = root.join(selected_name);

    if let Some(parent) = dest.parent() {
        if confirmed_name.is_some() {
            if !tokio::fs::metadata(parent).await?.is_dir() {
                return Err("bound download root is not a directory".into());
            }
        } else {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // Confirmed starts must retain the exact approved name even when it is
    // already taken; publication checks it atomically after the data is ready.
    let dest = if confirmed_name.is_some() {
        dest
    } else {
        super::collision::available_from(&dest, force_rename).await?
    };
    let filename = dest
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid destination filename")?;
    if let Some(task) = tasks.lock().await.get_mut(url) {
        task.filename = Some(filename.to_owned());
        task.destination_path = Some(dest.to_string_lossy().into_owned());
    }

    // Confirmed files reserve their task association and inode durably before
    // any payload write. Legacy tasks retain their existing allocation path.
    #[cfg(target_os = "linux")]
    let (temporary, mut file) = if confirmed_name.is_some() {
        super::http_partial_owner::create(&dest, url, &tasks, &store, &control).await?
    } else {
        let (temporary, file) = create_temporary_file(&dest).await?;
        {
            let mut guard = tasks.lock().await;
            if *control.borrow() == super::control::TransferControl::Canceled
                || !guard.contains_key(url)
            {
                drop(guard);
                drop(file);
                tokio::fs::remove_file(&temporary).await?;
                return Err("download canceled".into());
            }
            if let Some(task) = guard.get_mut(url) {
                task.partial_name = temporary
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned);
            }
        }
        if let Err(error) = store.save(&tasks, &limiter).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        (temporary, file)
    };
    #[cfg(not(target_os = "linux"))]
    let (temporary, mut file) = {
        if confirmed_name.is_some() {
            return Err("confirmed HTTP partial ownership is unavailable on this platform".into());
        }
        let (temporary, file) = create_temporary_file(&dest).await?;
        {
            let mut guard = tasks.lock().await;
            if *control.borrow() == super::control::TransferControl::Canceled
                || !guard.contains_key(url)
            {
                drop(guard);
                drop(file);
                tokio::fs::remove_file(&temporary).await?;
                return Err("download canceled".into());
            }
            if let Some(task) = guard.get_mut(url) {
                task.partial_name = temporary
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned);
            }
        }
        if let Err(error) = store.save(&tasks, &limiter).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        (temporary, file)
    };
    #[cfg(target_os = "linux")]
    if split {
        let info = resume.as_ref().ok_or("split metadata unavailable")?;
        drop(resp);
        let prepared = super::split::live::create(
            &temporary,
            file.into_std().await,
            url,
            filename,
            info,
            connections,
        )?;
        return super::split::live::run(
            prepared, temporary, client, tasks, limiter, store, control,
        )
        .await;
    }
    let mut checkpoint = match resume.as_ref() {
        Some(info) => match super::checkpoint::Checkpoint::new(&temporary, url, filename, info) {
            Ok(checkpoint) => Some(checkpoint),
            Err(e) => {
                drop(file);
                if confirmed_name.is_none() {
                    let _ = tokio::fs::remove_file(&temporary).await;
                }
                return Err(e.into());
            }
        },
        None => None,
    };
    if let Some(checkpoint) = checkpoint.as_mut() {
        checkpoint.attach_persistence(store.clone(), tasks.clone());
        if let Err(e) = checkpoint.save(&file, true).await {
            drop(file);
            if confirmed_name.is_none() {
                let _ = tokio::fs::remove_file(&temporary).await;
                let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
            }
            return Err(e);
        }
    }
    let downloaded = match super::transfer_body::receive(
        &client,
        Some(resp),
        url,
        &mut file,
        &tasks,
        &limiter,
        &mut control,
        resume.as_ref(),
        total,
        checkpoint.as_mut(),
        0,
    )
    .await
    {
        Ok(n) => n,
        Err(e) => {
            let canceled = *control.borrow() == super::control::TransferControl::Canceled;
            if !canceled || confirmed_name.is_some() {
                if let Some(checkpoint) = checkpoint.as_mut() {
                    if let Err(error) = checkpoint.save(&file, true).await {
                        eprintln!("could not save failed transfer checkpoint: {error}");
                    }
                }
            }
            drop(file);
            if confirmed_name.is_none()
                && (*control.borrow() == super::control::TransferControl::Canceled
                    || checkpoint.is_none())
            {
                let _ = tokio::fs::remove_file(&temporary).await;
                if let Some(checkpoint) = &checkpoint {
                    let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
                }
            }
            return Err(e);
        }
    };
    if let Some(checkpoint) = checkpoint.as_mut() {
        if let Err(e) = checkpoint.save(&file, true).await {
            drop(file);
            if confirmed_name.is_none() {
                let _ = tokio::fs::remove_file(&temporary).await;
                let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
            }
            return Err(e);
        }
    }
    if let Err(e) = file.flush().await {
        drop(file);
        if confirmed_name.is_none() {
            let _ = tokio::fs::remove_file(&temporary).await;
            if let Some(checkpoint) = &checkpoint {
                let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
            }
        }
        return Err(e.into());
    }
    // Persist all data before linking even when no resume checkpoint exists.
    if let Err(error) = file.sync_data().await {
        return Err(error.into());
    }
    drop(file);
    // Serialize publication with task removal. A removed/canceled transfer must
    // not publish even if a new task with the same URL has since been queued.
    let mut guard = loop {
        if !super::control::wait_running(&tasks, url, &mut control).await {
            if confirmed_name.is_none() {
                let _ = tokio::fs::remove_file(&temporary).await;
                if let Some(checkpoint) = &checkpoint {
                    let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
                }
            }
            return Err("download canceled".into());
        }
        let mut guard = tasks.lock().await;
        let state = *control.borrow();
        if state == super::control::TransferControl::Canceled || !guard.contains_key(url) {
            drop(guard);
            if confirmed_name.is_none() {
                let _ = tokio::fs::remove_file(&temporary).await;
                if let Some(checkpoint) = &checkpoint {
                    let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
                }
            }
            return Err("download canceled".into());
        }
        if state == super::control::TransferControl::Paused {
            drop(guard);
            continue;
        }
        if let Some(task) = guard.get_mut(url) {
            task.state = "finalizing".into();
        }
        break guard;
    };
    // The link may exist even if directory syncing fails; preserve the partial
    // and checkpoint on every publication error instead of deleting evidence.
    #[cfg(target_os = "linux")]
    let dest = if confirmed_name.is_some() {
        super::task_directory::publish_http(&store.database_path(), url, &temporary, &dest).await?
    } else {
        super::collision::publish_for_task(&temporary, &dest, false).await?
    };
    #[cfg(not(target_os = "linux"))]
    let dest = super::collision::publish_for_task(&temporary, &dest, confirmed_name.is_some()).await?;
    if let Some(task) = guard.get_mut(url) {
        task.filename = dest.file_name().and_then(|n| n.to_str()).map(str::to_owned);
        task.destination_path = Some(dest.to_string_lossy().into_owned());
    }
    drop(guard);
    // A cleanup failure leaves only a redundant .part file, not a corrupted
    // final file. Never delete the successfully published destination.
    // Confirmed cleanup must not use a replaceable path. Retain both files
    // until descriptor-relative ownership-checked cleanup is available.
    if confirmed_name.is_none() {
        let _ = tokio::fs::remove_file(&temporary).await;
        if let Some(checkpoint) = &checkpoint {
            let _ = tokio::fs::remove_file(&checkpoint.sidecar).await;
        }
    }
    eprintln!("\nsaved {} bytes to {}", downloaded, dest.display());
    Ok(dest)
}

#[cfg(test)]
async fn check_destination(dest: &std::path::Path) -> std::io::Result<()> {
    match tokio::fs::symlink_metadata(dest).await {
        Ok(_) => Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}. Rename or move it before retrying; it was not overwritten", dest.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

async fn create_temporary_file(
    dest: &std::path::Path,
) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = dest.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination has no parent",
        )
    })?;
    for _ in 0..100 {
        let path = parent.join(format!(
            ".rget-{}-{}.part",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a temporary download file",
    ))
}

fn safe_filename(name: &str) -> Result<&str, std::io::Error> {
    use std::path::Component;
    let mut components = std::path::Path::new(name).components();
    if name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || name == "."
        || name == ".."
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsafe download filename",
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::{check_destination, safe_filename};
    use crate::daemon::publication::publish_no_clobber;

    #[test]
    fn rejects_unsafe_names() {
        for name in [
            "",
            ".",
            "..",
            "../outside",
            "/tmp/outside",
            "dir/file",
            "dir\\file",
        ] {
            assert!(safe_filename(name).is_err(), "accepted {name:?}");
        }
        assert_eq!(safe_filename("file.txt").unwrap(), "file.txt");
    }
    #[tokio::test]
    async fn collision_is_detected_and_late_collision_preserves_both_files() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rget-collision-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let dest = dir.join("file.bin");
        let partial = dir.join(".rget-test.part");
        assert!(check_destination(&dest).await.is_ok());
        tokio::fs::write(&partial, b"new data").await.unwrap();
        tokio::fs::write(&dest, b"existing data").await.unwrap();
        assert_eq!(
            check_destination(&dest).await.unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let error = publish_no_clobber(&partial, &dest).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("preserved at"));
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), b"existing data");
        assert_eq!(tokio::fs::read(&partial).await.unwrap(), b"new data");
        tokio::fs::remove_file(&dest).await.unwrap();
        publish_no_clobber(&partial, &dest).await.unwrap();
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), b"new data");
        assert_eq!(tokio::fs::read(&partial).await.unwrap(), b"new data");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

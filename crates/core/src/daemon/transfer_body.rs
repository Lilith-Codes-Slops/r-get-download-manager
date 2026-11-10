use super::bandwidth::BandwidthLimiter;
use super::control::{wait_running, Tasks, TransferControl};
use super::range::{validate, ResumeInfo};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

type TransferResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn receive(
    client: &reqwest::Client,
    initial: Option<reqwest::Response>,
    url: &str,
    file: &mut tokio::fs::File,
    tasks: &Tasks,
    limiter: &Arc<BandwidthLimiter>,
    control: &mut watch::Receiver<TransferControl>,
    resume: Option<&ResumeInfo>,
    total: Option<u64>,
    mut checkpoint: Option<&mut super::checkpoint::Checkpoint>,
    initial_downloaded: u64,
) -> TransferResult<u64> {
    let mut response = initial;
    let mut downloaded = initial_downloaded;
    let progress = super::progress::ProgressMeter::new(initial_downloaded);
    loop {
        let state = *control.borrow();
        if state == TransferControl::Paused {
            response.take();
            if let Some(task) = tasks.lock().await.get_mut(url) {
                task.active_connections = Some(0);
            }
            if let Some(checkpoint) = checkpoint.as_deref_mut() {
                checkpoint.save(file, true).await?;
            }
        }
        if !wait_running(tasks, url, control).await {
            return Err("download canceled".into());
        }
        if total == Some(downloaded) && downloaded != 0 {
            return Ok(downloaded);
        }
        let resp = if let Some(resp) = response.take() {
            resp
        } else {
            let info = resume.ok_or("safe resume metadata unavailable")?;
            let mut request = client
                .get(url)
                .header(reqwest::header::ACCEPT_ENCODING, "identity");
            if downloaded > 0 {
                request = request
                    .header(reqwest::header::RANGE, format!("bytes={downloaded}-"))
                    .header(reqwest::header::IF_RANGE, info.etag.as_str());
            }
            let result = tokio::select! {
                result = request.send() => Some(result?),
                changed = control.changed() => { if changed.is_err() { return Err("download canceled".into()); } None },
            };
            let Some(resp) = result else { continue };
            validate(&resp, info, downloaded)?;
            resp
        };
        if let Some(task) = tasks.lock().await.get_mut(url) {
            task.active_connections = Some(1);
        }
        let mut stream = resp.bytes_stream();
        loop {
            if *control.borrow() != TransferControl::Running {
                break;
            }
            let chunk = tokio::select! {
                item = stream.next() => item,
                changed = control.changed() => { if changed.is_err() { return Err("download canceled".into()); } break },
            };
            let Some(chunk) = chunk else {
                if total.is_some_and(|expected| expected != downloaded) {
                    return Err("download length does not match Content-Length".into());
                }
                return Ok(downloaded);
            };
            let chunk = chunk?;
            tokio::select! {
                _ = limiter.acquire(url, chunk.len()) => {},
                changed = control.changed() => { if changed.is_err() { return Err("download canceled".into()); } break },
            }
            if *control.borrow() != TransferControl::Running {
                break;
            }
            if total
                .is_some_and(|expected| chunk.len() as u64 > expected.saturating_sub(downloaded))
            {
                return Err("server sent more bytes than expected".into());
            }
            // Complete each write before acknowledging pause; partial canceled writes
            // are never counted as completed ranges.
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            if let Some(checkpoint) = checkpoint.as_deref_mut() {
                checkpoint.update(&chunk);
                checkpoint.save(file, false).await?;
            }
            let mut guard = tasks.lock().await;
            if *control.borrow() != TransferControl::Canceled {
                if let Some(task) = guard.get_mut(url) {
                    progress.update(task, downloaded);
                }
            }
        }
        // Dropping the response closes the request; resume opens a validated range.
    }
}

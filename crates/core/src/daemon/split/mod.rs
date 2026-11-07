//! Internal fixed-range split prototype. Not connected to the daemon yet:
//! it has no durable multi-range recovery, user controls or publication path.
mod segments;
mod worker;
#[cfg(test)]
mod tests;

use super::bandwidth::BandwidthLimiter;
use super::range::ResumeInfo;
use segments::SegmentMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

/// Download validated ranges into a fresh, exclusively owned partial file.
/// The file must be writable and opened without append mode. Returns only after
/// every worker has exited, including on an error. Until supervised cancellation
/// is implemented, callers must await this future rather than dropping it. The
/// caller must not publish the file unless this function returns success.
pub(super) async fn download_fixed(
    client: reqwest::Client,
    url: String,
    info: ResumeInfo,
    connections: u8,
    file: Arc<std::fs::File>,
    limiter: Arc<BandwidthLimiter>,
) -> Result<(), String> {
    let map = Arc::new(Mutex::new(SegmentMap::new(info.total, connections)?));
    if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
        return Err("fixed split requires a new empty partial file".into());
    }
    // Verify a bounded range before starting workers; unsupported servers can
    // fall back without having written anything to the partial.
    let probe = client.get(&url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, "bytes=0-0")
        .header(reqwest::header::IF_RANGE, info.etag.as_str())
        .send().await.map_err(|e| e.to_string())?;
    super::range::validate_bounded(&probe, &info, 0, 1)?;
    drop(probe);

    let count = map.lock().await.len();
    let (stop, _) = watch::channel(false);
    let mut workers = tokio::task::JoinSet::new();
    for owner in 0..count {
        let context = worker::Context {
            client: client.clone(), url: url.clone(), info: info.clone(),
            file: file.clone(), map: map.clone(), limiter: limiter.clone(),
            stop: stop.subscribe(), owner,
        };
        let stop = stop.clone();
        workers.spawn(async move {
            let result = worker::run(context).await;
            if result.is_err() { stop.send_replace(true); }
            result
        });
    }
    let mut first_error: Option<String> = None;
    while let Some(result) = workers.join_next().await {
        match result {
            Ok(Ok(())) => {},
            Ok(Err(error)) => {
                if first_error.is_none() || first_error.as_deref() == Some("split stopped") { first_error = Some(error); }
            },
            Err(error) => {
                stop.send_replace(true);
                if first_error.is_none() || first_error.as_deref() == Some("split stopped") {
                    first_error = Some(format!("split worker failed: {error}"));
                }
            },
        }
    }
    if let Some(error) = first_error { return Err(error); }
    if !map.lock().await.is_complete() { return Err("split ranges are incomplete".into()); }
    Ok(())
}

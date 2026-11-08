//! Split engine with work-stealing and adaptive concurrency.
pub(super) mod checkpoint;
pub(super) mod recovery;
pub(super) mod startup;
pub(super) mod live;
#[cfg(feature = "checkpoint-test-hooks")]
mod test_hooks;
mod adaptive;
mod adaptive_sizer;
mod steal_floor;
mod sizing_state;
#[cfg(test)]
mod steal_floor_tests;
mod runner;
pub(super) mod segments;
mod worker;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod range_validation_tests;
#[cfg(test)]
mod startup_tests;
#[cfg(test)]
mod stealing_tests;
#[cfg(test)]
mod adaptive_tests;
#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod rate_tests;

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
    let (stop, _) = watch::channel(false);
    run_map(client, url, info, file, map, limiter, stop, connections as usize, None).await
}

async fn run_map(
    client: reqwest::Client, url: String, info: ResumeInfo, file: Arc<std::fs::File>,
    map: Arc<Mutex<SegmentMap>>, limiter: Arc<BandwidthLimiter>, stop: watch::Sender<bool>, max_workers: usize,
    desired: Option<watch::Receiver<u8>>,
) -> Result<(), String> {
    if *stop.borrow() { return Err("split stopped".into()); }
    // Verify a bounded range before starting workers; unsupported servers can
    // fall back without having written anything to the partial.
    let request = client.get(&url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, "bytes=0-0")
        .header(reqwest::header::IF_RANGE, info.etag.as_str());
    let mut stop_rx = stop.subscribe();
    let probe = tokio::select! {
        result = request.send() => result.map_err(|e| e.to_string())?,
        _ = stop_rx.changed() => return Err("split stopped".into()),
    };
    super::range::validate_bounded(&probe, &info, 0, 1)?;
    drop(probe);

    let max = max_workers.min((info.total / (1024 * 1024)).min(32) as usize);
    adaptive::supervise(adaptive::Inputs { client, url, info, file, map, limiter, stop, max, desired,
        #[cfg(test)] peak_workers: None,
    }).await
}

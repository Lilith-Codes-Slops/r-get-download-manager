//! Completion summary, preserving unknown engine counters as unknown.
use downloader_ipc::TorrentMetrics;

pub fn log_completion(task_id: &str, metrics: &TorrentMetrics) {
    tracing::info!(task_id, time_to_first_piece_ms = ?metrics.time_to_first_piece_ms,
        stall_count = metrics.stall_count, duplicate_bytes = ?metrics.duplicate_bytes,
        hash_fails = ?Option::<u32>::None, peak_request_queue_depth = ?Option::<u32>::None,
        "torrent completed: sampled diagnostics");
}

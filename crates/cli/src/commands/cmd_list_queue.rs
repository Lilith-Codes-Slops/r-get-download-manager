use super::super::ipc_client;
use downloader_ipc::{ClientRequest, DaemonEvent};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    match ipc_client::roundtrip(&ClientRequest::GetDetailedQueue)? {
        DaemonEvent::DetailedQueue { tasks } => {
            if tasks.is_empty() { println!("Queue is empty."); }
            for task in tasks {
                let path = task.final_path.as_deref().or(task.destination_path.as_deref())
                    .or(task.filename.as_deref()).unwrap_or("destination pending");
                let label = if task.final_path.is_some() { "saved" } else { "destination (may change)" };
                let progress = task.total.map(|total| {
                    let pct = if total == 0 { 0.0 } else { task.downloaded as f64 * 100.0 / total as f64 };
                    format!("{} / {} bytes ({pct:.1}%)", task.downloaded, total)
                }).unwrap_or_else(|| format!("{} bytes", task.downloaded));
                println!("{}: {} — {} — {} bytes/s — {}: {}", task.url, task.state, progress, task.speed_bps, label, path);
                if let Some(note) = &task.connection_note { println!("  {note}"); }
                if let Some(m) = task.torrent.as_ref().and_then(|torrent| torrent.metrics.as_ref()) {
                    let first = m.time_to_first_piece_ms.map(|n| format!("{n} ms")).unwrap_or_else(|| "unknown".into());
                    println!("  stalls: {}{}; first verified piece: {first}", m.stall_count, if m.stalled { " (stalled)" } else { "" });
                }
            }
        }
        other => println!("{other:?}"),
    }
    Ok(())
}

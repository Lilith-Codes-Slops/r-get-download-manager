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
                println!("{}: {} — {}: {}", task.url, task.state, label, path);
            }
        }
        other => println!("{other:?}"),
    }
    Ok(())
}

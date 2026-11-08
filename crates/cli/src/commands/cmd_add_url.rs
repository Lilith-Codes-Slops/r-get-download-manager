use super::super::args_parser::AddArgs;
use super::super::ipc_client;
use downloader_ipc::{ClientRequest, DaemonEvent};
use std::time::{Duration, Instant};

pub async fn run(args: AddArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args.url.clone();
    let req = ClientRequest::AddTask {
        url: args.url, mirrors: args.mirrors, connections: args.connections,
        extract: args.extract, category: None, not_before: None,
    };
    let event = ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    // The server response determines the filename asynchronously. Do not
    // wait indefinitely or claim that a planned name is a published file.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(DaemonEvent::DetailedQueue { tasks }) = ipc_client::roundtrip(&ClientRequest::GetDetailedQueue) {
            if let Some(task) = tasks.iter().find(|task| task.url == url) {
                if let Some(path) = task.final_path.as_deref() { println!("Saved to: {path}"); return Ok(()); }
                if let Some(path) = task.destination_path.as_deref().or(task.filename.as_deref()) {
                    println!("Destination (may change): {path}"); return Ok(());
                }
                if task.state.starts_with("failed:") { println!("{}", task.state); return Ok(()); }
            }
        }
        if Instant::now() >= deadline { println!("Destination pending; use dl list to check later."); return Ok(()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

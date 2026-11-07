use super::super::ipc_client;
use downloader_ipc::ClientRequest;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let event = ipc_client::roundtrip(&ClientRequest::ListQueue)?;
    println!("{event:?}");
    Ok(())
}
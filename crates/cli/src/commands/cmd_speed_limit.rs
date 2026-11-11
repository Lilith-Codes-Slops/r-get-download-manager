use super::super::args_parser::LimitArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: LimitArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::SetLimit {
        task_id: args.task_id,
        kbps: args.kbps,
    };
    let event = super::super::ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}

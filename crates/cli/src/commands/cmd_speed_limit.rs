use super::super::args_parser::LimitArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: LimitArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::SetLimit {
        task_id: args.task_id,
        kbps: args.kbps,
    };
    println!("limit set");
    Ok(())
}

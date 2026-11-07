use super::super::args_parser::RefreshArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: RefreshArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::RefreshUrl {
        task_id: args.task_id,
        new_url: args.new_url,
    };
    let event = super::super::ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}

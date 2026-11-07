use super::super::args_parser::RmArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: RmArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::RemoveTask {
        task_id: args.task_id,
        delete_file: args.delete_file,
    };
    let event = super::super::ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}

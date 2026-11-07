use super::super::args_parser::RmArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: RmArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::RemoveTask {
        task_id: args.task_id,
        delete_file: args.delete_file,
    };
    println!("task removed");
    Ok(())
}

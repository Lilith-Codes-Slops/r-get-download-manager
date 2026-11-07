use super::super::args_parser::TaskArgs;
use downloader_ipc::ClientRequest;

pub async fn pause(args: TaskArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::Pause { task_id: args.task_id };
    println!("paused");
    Ok(())
}

pub async fn resume(args: TaskArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::Resume { task_id: args.task_id };
    println!("resumed");
    Ok(())
}

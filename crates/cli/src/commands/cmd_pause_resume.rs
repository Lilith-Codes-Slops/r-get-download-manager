use super::super::args_parser::TaskArgs;
use downloader_ipc::ClientRequest;

pub async fn pause(args: TaskArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::Pause {
        task_id: args.task_id,
    };
    let event = super::super::ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}

pub async fn resume(args: TaskArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::Resume {
        task_id: args.task_id,
    };
    let event = super::super::ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}

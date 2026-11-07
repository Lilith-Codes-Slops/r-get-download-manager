use super::super::args_parser::MirrorArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: MirrorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::AddMirror {
        task_id: args.task_id,
        mirror_url: args.mirror_url,
    };
    println!("mirror added");
    Ok(())
}

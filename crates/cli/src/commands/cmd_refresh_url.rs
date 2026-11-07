use super::super::args_parser::RefreshArgs;
use downloader_ipc::ClientRequest;

pub async fn run(args: RefreshArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _req = ClientRequest::RefreshUrl {
        task_id: args.task_id,
        new_url: args.new_url,
    };
    println!("url refreshed");
    Ok(())
}

use super::super::args_parser::AddArgs;
use super::super::ipc_client;
use downloader_ipc::ClientRequest;

pub async fn run(args: AddArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::AddTask {
        url: args.url,
        mirrors: args.mirrors,
        connections: args.connections,
        extract: args.extract,
        category: None,
        not_before: None,
    };
    let event = ipc_client::roundtrip(&req)?;
    println!("{event:?}");
    Ok(())
}
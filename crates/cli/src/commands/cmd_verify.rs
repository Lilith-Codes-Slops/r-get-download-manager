use super::super::{args_parser::VerifyArgs, ipc_client};
use downloader_ipc::ClientRequest;

pub async fn run(args: VerifyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let req = ClientRequest::VerifyChecksum {
        task_id: args.task_id,
        algorithm: args.algorithm,
        expected: args.expected,
    };
    println!("{:?}", ipc_client::roundtrip(&req)?);
    Ok(())
}

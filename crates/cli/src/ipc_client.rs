use downloader_ipc::{ClientRequest, DaemonEvent, EndpointPathResolver, FrameDecoder, FrameEncoder};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn roundtrip(req: &ClientRequest) -> Result<DaemonEvent, Box<dyn std::error::Error>> {
    let endpoint = EndpointPathResolver::resolve()?;
    let mut stream = UnixStream::connect(&endpoint)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    FrameEncoder::write_frame(&mut stream, req)?;
    stream.flush()?;

    let event: DaemonEvent = FrameDecoder::read_frame(&mut stream)?;
    Ok(event)
}
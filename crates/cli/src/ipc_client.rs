use downloader_ipc::{
    ClientRequest, DaemonEvent, EndpointPathResolver, FrameDecoder, FrameEncoder,
};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn roundtrip(req: &ClientRequest) -> Result<DaemonEvent, Box<dyn std::error::Error>> {
    let endpoint = EndpointPathResolver::resolve()?;
    let mut stream = UnixStream::connect(&endpoint)?;
    let read_timeout = if matches!(req, ClientRequest::VerifyChecksum { .. }) {
        1800
    } else {
        30
    };
    stream.set_read_timeout(Some(Duration::from_secs(read_timeout)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    FrameEncoder::write_frame(&mut stream, req)?;
    stream.flush()?;

    let event: DaemonEvent = FrameDecoder::read_frame(&mut stream)?;
    if let DaemonEvent::TaskFailed { error, .. } = &event {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, error.clone()).into());
    }
    Ok(event)
}

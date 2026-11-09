//! Opt-in live torrent transition stream; reconnect to resume after disconnect.
use downloader_ipc::{ClientRequest, DaemonEvent, EndpointPathResolver, FrameDecoder, FrameEncoder};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(EndpointPathResolver::resolve()?)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    FrameEncoder::write_frame(&mut stream, &ClientRequest::SubscribeTorrentEvents)?;
    let first: DaemonEvent = FrameDecoder::read_frame(&mut stream)?;
    if !matches!(first, DaemonEvent::TorrentEventsReady) {
        return Err(format!("torrent event stream unavailable: {first:?}").into());
    }
    println!("Watching torrent stall events. Press Ctrl+C to stop.");
    loop {
        let event: DaemonEvent = FrameDecoder::read_frame(&mut stream)?;
        match event {
            DaemonEvent::TorrentEventsHeartbeat => {},
            DaemonEvent::TorrentStalled { task_id, stall_count } => println!("{task_id}: stalled (count {stall_count})"),
            DaemonEvent::TorrentRecovered { task_id, stall_count, duration_ms } => println!("{task_id}: recovered after {duration_ms} ms (count {stall_count})"),
            other => return Err(format!("unexpected torrent event: {other:?}").into()),
        }
    }
}

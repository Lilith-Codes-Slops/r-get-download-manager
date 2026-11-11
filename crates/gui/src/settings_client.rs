//! Settings IPC. The GUI never opens the daemon's SQLite database.
use downloader_ipc::{ClientRequest, DaemonEvent};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;

pub type Overrides = BTreeMap<String, Value>;
pub type Changes = BTreeMap<String, Option<Value>>;

#[cfg(target_os = "linux")]
fn request(request: &ClientRequest) -> io::Result<DaemonEvent> {
    use downloader_ipc::{EndpointPathResolver, FrameDecoder, FrameEncoder};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    let endpoint = EndpointPathResolver::resolve().map_err(io::Error::other)?;
    let mut stream = UnixStream::connect(endpoint)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    FrameEncoder::write_frame(&mut stream, request).map_err(io::Error::other)?;
    FrameDecoder::read_frame(&mut stream).map_err(io::Error::other)
}

#[cfg(not(target_os = "linux"))]
fn request(_request: &ClientRequest) -> io::Result<DaemonEvent> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "The downloader daemon is supported on Linux only",
    ))
}

fn settings_response(response: DaemonEvent) -> io::Result<Overrides> {
    match response {
        DaemonEvent::GuiSettings { overrides } => Ok(overrides),
        DaemonEvent::TaskFailed { error, .. } => Err(io::Error::other(error)),
        other => Err(io::Error::other(format!(
            "unexpected daemon settings response: {other:?}"
        ))),
    }
}

pub fn load() -> io::Result<Overrides> {
    settings_response(request(&ClientRequest::GetGuiSettings)?)
}

pub fn update(changes: Changes) -> io::Result<Overrides> {
    settings_response(request(&ClientRequest::UpdateGuiSettings { changes })?)
}

fn directory_response(response: DaemonEvent) -> io::Result<std::path::PathBuf> {
    match response {
        DaemonEvent::DownloadDirectory { path } => Ok(path.into()),
        DaemonEvent::TaskFailed { error, .. } => Err(io::Error::other(error)),
        other => Err(io::Error::other(format!(
            "unexpected daemon directory response: {other:?}"
        ))),
    }
}

pub fn load_directory() -> io::Result<std::path::PathBuf> {
    directory_response(request(&ClientRequest::GetDownloadDirectory)?)
}

pub fn set_directory(path: &std::path::Path) -> io::Result<std::path::PathBuf> {
    let path = path.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "The path is not valid UTF-8")
    })?;
    directory_response(request(&ClientRequest::SetDownloadDirectory {
        path: path.to_owned(),
    })?)
}

pub fn reset() -> io::Result<()> {
    settings_response(request(&ClientRequest::ResetGuiSettings)?).map(|_| ())
}

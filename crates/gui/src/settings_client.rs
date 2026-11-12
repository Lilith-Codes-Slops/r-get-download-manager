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
    // Torrent admission may resolve metainfo before replying. Timing out at
    // the normal settings deadline can report failure after the task started.
    let timeout = if matches!(request, ClientRequest::AddConfirmedTask { .. }) {
        Duration::from_secs(75)
    } else {
        Duration::from_secs(3)
    };
    stream.set_read_timeout(Some(timeout))?;
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

/// Resolve a task-only folder through the daemon without changing settings.
#[allow(dead_code)]
pub fn validate_directory(path: &std::path::Path) -> io::Result<std::path::PathBuf> {
    let path = path.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "The path is not valid UTF-8")
    })?;
    directory_response(request(&ClientRequest::ValidateDownloadDirectory {
        path: path.to_owned(),
    })?)
}

fn confirmed_request(
    url: String,
    filename: String,
    folder: &std::path::Path,
) -> io::Result<ClientRequest> {
    let folder = folder.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "The path is not valid UTF-8")
    })?;
    Ok(ClientRequest::AddConfirmedTask {
        url,
        mirrors: Vec::new(),
        connections: 8,
        extract: false,
        category: None,
        not_before: None,
        filename,
        folder: folder.to_owned(),
    })
}

/// Send a confirmed task start without changing the daemon-wide directory.
/// Call off the UI thread because the local IPC request may block.
pub fn start_confirmed(url: String, filename: String, folder: &std::path::Path) -> io::Result<()> {
    let response = request(&confirmed_request(url, filename, folder)?)
    .map_err(|error| {
        io::Error::other(format!(
            "Could not confirm the daemon response: {error}. Check the daemon queue before retrying."
        ))
    })?;
    start_response(response)
}

fn start_response(response: DaemonEvent) -> io::Result<()> {
    match response {
        // Torrents have daemon-generated stable task IDs rather than the
        // submitted source. This connection has exactly one request, so the
        // reply belongs to that request regardless of which ID was assigned.
        DaemonEvent::TaskStateChanged { state, .. } if state.starts_with("existing:") => {
            Err(io::Error::other(
                "This download already exists; the confirmed name and folder were not applied.",
            ))
        }
        DaemonEvent::TaskStateChanged { task_id, state, .. }
            if !task_id.is_empty() && matches!(state.as_str(), "queued" | "downloading") =>
        {
            Ok(())
        }
        DaemonEvent::TaskFailed { error, .. } => Err(io::Error::other(error)),
        other => Err(io::Error::other(format!(
            "Unexpected daemon start response: {other:?}. Check the daemon queue before retrying."
        ))),
    }
}

pub fn reset() -> io::Result<()> {
    settings_response(request(&ClientRequest::ResetGuiSettings)?).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_start_accepts_daemon_assigned_torrent_id_and_reports_failure() {
        assert!(start_response(DaemonEvent::TaskStateChanged {
            task_id: "torrent-task-id".into(),
            state: "queued".into(),
            destination_path: None,
        })
        .is_ok());
        let error = start_response(DaemonEvent::TaskFailed {
            task_id: "torrent-task-id".into(),
            error: "admission failed".into(),
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "admission failed");
        let error = start_response(DaemonEvent::TaskStateChanged {
            task_id: "torrent-task-id".into(),
            state: "existing: downloading".into(),
            destination_path: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert!(start_response(DaemonEvent::TaskStateChanged {
            task_id: String::new(),
            state: "queued".into(),
            destination_path: None,
        })
        .is_err());
    }

    #[test]
    fn confirmed_start_preserves_source_name_and_one_time_folder() {
        let request = confirmed_request(
            "https://example.test/file".into(),
            "my file.bin".into(),
            std::path::Path::new("/tmp/one-time folder"),
        )
        .unwrap();
        assert!(matches!(request, ClientRequest::AddConfirmedTask {
            url, filename, folder, connections: 8, ..
        } if url == "https://example.test/file" && filename == "my file.bin" && folder == "/tmp/one-time folder"));
    }
}

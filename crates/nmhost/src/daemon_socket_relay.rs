use super::{stdio_frame_reader::StdioFrameReader, stdio_frame_writer::StdioFrameWriter};
use downloader_ipc::{ClientRequest, DaemonEvent, EndpointPathResolver, FrameEncoder, IpcError, IpcResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct DaemonSocketRelay;

impl DaemonSocketRelay {
    pub async fn run(mut reader: StdioFrameReader, mut writer: StdioFrameWriter) -> IpcResult<()> {
        loop {
            let msg: serde_json::Value = match reader.read_frame() {
                Ok(msg) => msg,
                Err(IpcError::ConnectionClosed) => return Ok(()),
                Err(e) => return Err(e),
            };
            let reply = match Self::relay(&msg).await {
                Ok(reply) => reply,
                Err(e) => DaemonEvent::TaskFailed { task_id: String::new(), error: e.to_string() },
            };
            writer.write_frame(&reply)?;
        }
    }

    async fn relay(msg: &serde_json::Value) -> IpcResult<DaemonEvent> {
        let url = msg.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| IpcError::Framing("missing URL".into()))?;
        if let Some(op) = msg.get("op").and_then(|v| v.as_str()) {
            if op != "add_task" { return Err(IpcError::Framing("unsupported operation".into())); }
        }
        let req = ClientRequest::AddTask {
            url: url.to_string(), mirrors: vec![], connections: 1, extract: false,
            category: None, not_before: None,
        };
        let endpoint = EndpointPathResolver::resolve()?;
        #[cfg(target_os = "linux")]
        let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
        #[cfg(target_os = "windows")]
        let mut stream = tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)?;
        stream.write_all(&FrameEncoder::encode(&req)?).await?;
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).await?;
        let len = u32::from_le_bytes(prefix) as usize;
        if len == 0 || len > downloader_ipc::MAX_FRAME_PAYLOAD {
            return Err(IpcError::InvalidLengthPrefix("invalid daemon response length".into()));
        }
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

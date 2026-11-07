use downloader_ipc::{FrameEncoder, IpcResult};
use std::io;

pub struct StdioFrameWriter {
    stdout: io::Stdout,
}

impl StdioFrameWriter {
    pub fn new() -> Self {
        Self { stdout: io::stdout() }
    }

    pub fn write_frame<T: serde::Serialize>(&mut self, value: &T) -> IpcResult<()> {
        FrameEncoder::write_frame(&mut self.stdout.lock(), value)
    }
}

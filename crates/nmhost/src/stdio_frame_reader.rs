use downloader_ipc::{FrameDecoder, IpcResult};
use std::io;

pub struct StdioFrameReader {
    stdin: io::Stdin,
}

impl StdioFrameReader {
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        {
            // Binary mode would be set via winapi here.
        }
        Self { stdin: io::stdin() }
    }

    pub fn read_frame<T: serde::de::DeserializeOwned>(&mut self) -> IpcResult<T> {
        FrameDecoder::read_frame(&mut self.stdin.lock())
    }
}

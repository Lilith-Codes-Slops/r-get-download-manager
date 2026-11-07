use super::{stdio_frame_reader::StdioFrameReader, stdio_frame_writer::StdioFrameWriter};
use downloader_ipc::IpcResult;

pub struct DaemonSocketRelay;

impl DaemonSocketRelay {
    pub async fn run(
        mut reader: StdioFrameReader,
        mut writer: StdioFrameWriter,
    ) -> IpcResult<()> {
        loop {
            let msg: serde_json::Value = reader.read_frame()?;
            writer.write_frame(&msg)?;
        }
    }
}

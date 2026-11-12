//! Display-only destination. A planned name is not an owned or published file.
use downloader_ipc::DownloadInfo;

pub(super) fn path(task: &DownloadInfo) -> Option<String> {
    task.final_path
        .clone()
        .or_else(|| task.destination_path.clone())
}

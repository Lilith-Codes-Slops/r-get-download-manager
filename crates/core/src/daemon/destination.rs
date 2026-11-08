//! Display-only destination. A planned name is not an owned or published file.
use downloader_ipc::DownloadInfo;
use std::path::{Component, Path};

pub(super) fn path(task: &DownloadInfo) -> Option<String> {
    if let Some(path) = &task.final_path { return Some(path.clone()); }
    let name = task.filename.as_deref()?;
    let mut parts = Path::new(name).components();
    if name.contains('\\') || !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() { return None; }
    let root = std::env::var_os("HOME")?;
    Some(Path::new(&root).join("Downloads").join(name).to_string_lossy().into_owned())
}

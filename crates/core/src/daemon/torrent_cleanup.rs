//! Conservative cleanup for an explicitly recorded, task-owned staging folder.
use downloader_ipc::DownloadInfo;
use std::path::{Component, Path};
use tokio::io::AsyncWriteExt;

pub(super) const MARKER: &str = ".rget-owner";

pub(super) async fn write_marker(stage: &Path, id: &str) -> std::io::Result<()> {
    let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(stage.join(MARKER)).await?;
    file.write_all(id.as_bytes()).await?;
    file.sync_all().await
}

fn single_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    !name.chars().any(|c| matches!(c, '/' | '\\' | '\0')) && matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub(super) async fn remove_stage(task: &DownloadInfo, root: &Path) -> Result<(), String> {
    let Some(name) = task.torrent.as_ref().and_then(|identity| identity.stage_name.as_deref()) else { return Ok(()) };
    if !single_name(name) || !name.starts_with(".rget-torrent-") {
        return Err("refusing to delete unsafe torrent staging path".into());
    }
    let stage = root.join(name);
    let metadata = match tokio::fs::symlink_metadata(&stage).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("could not inspect torrent staging directory: {e}")),
    };
    if !metadata.is_dir() { return Err("refusing non-directory torrent staging path".into()); }
    let marker = stage.join(MARKER);
    let meta = tokio::fs::symlink_metadata(&marker).await.map_err(|e| format!("missing torrent ownership marker: {e}"))?;
    if !meta.is_file() || meta.len() > 512 { return Err("invalid torrent ownership marker".into()); }
    let owner = tokio::fs::read(&marker).await.map_err(|e| format!("could not read torrent ownership marker: {e}"))?;
    if owner != task.url.as_bytes() { return Err("torrent ownership marker does not match task".into()); }
    let payload = task.suggested_filename.as_deref();
    if payload.is_some_and(|name| !single_name(name) || name == MARKER) { return Err("unsafe torrent staging filename".into()); }
    let mut entries = tokio::fs::read_dir(&stage).await.map_err(|e| format!("could not inspect torrent staging: {e}"))?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| format!("could not read torrent staging: {e}"))? {
        let name = entry.file_name();
        if name.to_str() != Some(MARKER) && payload != name.to_str() { return Err("unrecognized file in torrent staging directory; nothing removed".into()); }
        let meta = tokio::fs::symlink_metadata(entry.path()).await.map_err(|e| format!("could not inspect torrent staging entry: {e}"))?;
        if !meta.is_file() { return Err("refusing non-regular torrent staging entry".into()); }
    }
    if let Some(payload) = payload {
        match tokio::fs::remove_file(stage.join(payload)).await {
            Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("could not remove torrent partial: {e}")),
        }
    }
    tokio::fs::remove_file(&marker).await.map_err(|e| format!("could not remove torrent marker: {e}"))?;
    tokio::fs::remove_dir(&stage).await.map_err(|e| format!("could not remove torrent staging directory: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_partial_removal_refuses_unknown_entries() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rget-torrent-clean-{}-{nonce}", std::process::id()));
        let stage = root.join(".rget-torrent-test"); tokio::fs::create_dir_all(&stage).await.unwrap();
        let task: DownloadInfo = serde_json::from_str(r#"{"url":"task-1","state":"failed","filename":null,"downloaded":0,"total":null,"speed_bps":0,"final_path":null,"suggested_filename":"file.bin","torrent":{"source":"magnet:?xt=test","info_hash":null,"stage_name":".rget-torrent-test","seed":false,"created_at":0}}"#).unwrap();
        write_marker(&stage, &task.url).await.unwrap();
        tokio::fs::write(stage.join("file.bin"), b"partial").await.unwrap();
        tokio::fs::write(stage.join("unknown"), b"keep").await.unwrap();
        assert!(remove_stage(&task, &root).await.is_err());
        assert!(stage.join("file.bin").exists()); assert!(stage.join(MARKER).exists());
        tokio::fs::remove_file(stage.join("unknown")).await.unwrap();
        remove_stage(&task, &root).await.unwrap();
        assert!(!stage.exists());
        tokio::fs::remove_dir(root).await.unwrap();
    }
}

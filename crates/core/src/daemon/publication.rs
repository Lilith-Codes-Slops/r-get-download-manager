//! No-clobber publication shared by fresh, recovered and split transfers.
use std::path::Path;

pub(super) async fn publish_no_clobber(partial: &Path, dest: &Path) -> std::io::Result<()> {
    match tokio::fs::hard_link(partial, dest).await {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(std::io::Error::new(
            e.kind(), format!("destination appeared during download: {}. Existing file was not overwritten; downloaded data is preserved at {}", dest.display(), partial.display()))),
        Err(e) => return Err(e),
        Ok(()) => {},
    }
    // If syncing fails, the link may already exist. The caller must preserve
    // the partial and checkpoint, so recovery or manual inspection is possible.
    #[cfg(unix)]
    {
        let parent = dest.parent().ok_or_else(|| std::io::Error::other("destination has no directory"))?.to_owned();
        tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
            .await.map_err(std::io::Error::other)??;
    }
    Ok(())
}

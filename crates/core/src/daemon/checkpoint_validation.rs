//! Validate checkpoint contents before any future crash recovery can use them.
use super::checkpoint::RecoveryRecord;
use std::io::{Error, ErrorKind, Read};
use std::path::Path;

pub(super) fn validate(
    sidecar: &Path,
    partial: &Path,
) -> Result<RecoveryRecord, Box<dyn std::error::Error + Send + Sync>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(sidecar)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err("invalid sidecar file".into());
    }
    let mut data = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut data)?;
    if data.len() > 64 * 1024 {
        return Err("oversized sidecar".into());
    }
    let record: RecoveryRecord = serde_json::from_slice(&data)?;
    if record.version != 1
        || record.sequence == 0
        || record.downloaded > record.total
        || !super::range::strong_etag(&record.etag)
    {
        return Err("invalid recovery metadata".into());
    }
    for url in [&record.task_url, &record.resolved_url] {
        if !matches!(reqwest::Url::parse(url), Ok(ref url) if matches!(url.scheme(), "http" | "https"))
        {
            return Err("invalid recovery URL".into());
        }
    }
    let component = Path::new(&record.filename);
    if record.filename.contains('\\')
        || component.components().count() != 1
        || !matches!(
            component.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err("unsafe final filename".into());
    }
    let name = partial
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("invalid partial filename")?;
    if name != record.partial_name
        || !name.starts_with(".rget-")
        || !name.ends_with(".part")
        || sidecar.parent() != partial.parent()
    {
        return Err("sidecar does not match partial file".into());
    }
    let mut expected_sidecar = partial.as_os_str().to_os_string();
    expected_sidecar.push(".meta");
    if sidecar != Path::new(&expected_sidecar) {
        return Err("unexpected sidecar path".into());
    }
    verify_prefix(&record, partial)?;
    Ok(record)
}

/// Open without following symlinks and hash the recorded prefix on the same
/// handle that the caller may later truncate and use for resumed writes.
pub(super) fn verify_prefix(
    record: &RecoveryRecord,
    partial: &Path,
) -> Result<(std::fs::File, blake3::Hasher), Box<dyn std::error::Error + Send + Sync>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(partial)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() < record.downloaded {
        return Err("partial file is missing checkpointed bytes".into());
    }
    let mut hasher = blake3::Hasher::new();
    let mut remaining = record.downloaded;
    let mut buf = [0u8; 64 * 1024];
    while remaining > 0 {
        let count = file.read(&mut buf[..remaining.min(64 * 1024) as usize])?;
        if count == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "partial file truncated").into());
        }
        hasher.update(&buf[..count]);
        remaining -= count as u64;
    }
    if hasher.finalize().to_hex().as_str() != record.prefix_blake3 {
        return Err("partial file checksum mismatch".into());
    }
    Ok((file, hasher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::checkpoint::Checkpoint;
    use crate::daemon::range::ResumeInfo;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn verifies_checkpoint_and_rejects_tampering() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rget-checkpoint-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let partial = dir.join(".rget-123-0.part");
        let mut file = tokio::fs::File::create(&partial).await.unwrap();
        let resume = ResumeInfo {
            etag: "\"v1\"".into(),
            total: 12,
            url: reqwest::Url::parse("https://example.test/file").unwrap(),
        };
        let mut checkpoint =
            Checkpoint::new(&partial, "https://example.test/file", "file.bin", &resume).unwrap();
        file.write_all(b"abcd").await.unwrap();
        checkpoint.update(b"abcd");
        checkpoint.save(&file, true).await.unwrap();
        let record = validate(&checkpoint.sidecar, &partial).unwrap();
        assert_eq!(record.downloaded, 4);
        assert_eq!(record.sequence, 1);
        // Uncheckpointed tail is permitted but must never be trusted for recovery.
        file.write_all(b"tail").await.unwrap();
        file.sync_all().await.unwrap();
        assert!(validate(&checkpoint.sidecar, &partial).is_ok());
        drop(file);
        std::fs::write(&partial, b"evil").unwrap();
        assert!(validate(&checkpoint.sidecar, &partial).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn rejects_sidecar_with_unsafe_filename() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rget-checkpoint-bad-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let partial = dir.join(".rget-123-0.part");
        let file = tokio::fs::File::create(&partial).await.unwrap();
        let resume = ResumeInfo {
            etag: "\"v1\"".into(),
            total: 12,
            url: reqwest::Url::parse("https://example.test/file").unwrap(),
        };
        let mut checkpoint =
            Checkpoint::new(&partial, "https://example.test/file", "../outside", &resume).unwrap();
        checkpoint.save(&file, true).await.unwrap();
        assert!(validate(&checkpoint.sidecar, &partial).is_err());
        drop(file);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! No-clobber destination naming. A free-name check is advisory; publication
//! must still use an atomic operation because another task may win the race.
use std::{io, path::{Path, PathBuf}};

const LIMIT: usize = 9999;

fn candidate(path: &Path, index: usize) -> io::Result<PathBuf> {
    if index == 0 { return Ok(path.to_owned()); }
    let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid destination name"))?;
    let extension = path.extension().and_then(|s| s.to_str());
    let name = match extension { Some(ext) => format!("{stem} ({index}).{ext}"), None => format!("{stem} ({index})") };
    Ok(path.with_file_name(name))
}

pub(super) async fn available(path: &Path) -> io::Result<PathBuf> { available_from(path, false).await }

pub(super) async fn available_from(path: &Path, renamed: bool) -> io::Result<PathBuf> {
    for index in usize::from(renamed)..=LIMIT {
        let name = candidate(path, index)?;
        match tokio::fs::symlink_metadata(&name).await {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(name),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination name limit exhausted"))
}

/// Hard-link the partial under the first name won atomically. Once linked, a
/// directory-sync failure is returned without trying another name.
pub(super) async fn publish(partial: &Path, proposed: &Path) -> io::Result<PathBuf> {
    for index in 0..=LIMIT {
        let dest = candidate(proposed, index)?;
        match tokio::fs::hard_link(partial, &dest).await {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
            Ok(()) => {
                #[cfg(unix)]
                {
                    let parent = dest.parent().ok_or_else(|| io::Error::other("destination has no directory"))?.to_owned();
                    tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all()).await.map_err(io::Error::other)??;
                }
                return Ok(dest);
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination name limit exhausted"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn multiple_collisions_and_late_race_never_overwrite() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("rget-names-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let original = dir.join("file.tar.gz"); let one = dir.join("file.tar (1).gz");
        let two = dir.join("file.tar (2).gz"); let partial = dir.join("partial");
        tokio::fs::write(&original, b"old").await.unwrap();
        tokio::fs::write(&one, b"other").await.unwrap();
        tokio::fs::write(&partial, b"new").await.unwrap();
        assert_eq!(available(&original).await.unwrap(), two);
        tokio::fs::write(&two, b"race").await.unwrap();
        let published = publish(&partial, &original).await.unwrap();
        assert_eq!(published, dir.join("file.tar (3).gz"));
        assert_eq!(tokio::fs::read(&original).await.unwrap(), b"old");
        assert_eq!(tokio::fs::read(&two).await.unwrap(), b"race");
        assert_eq!(tokio::fs::read(&published).await.unwrap(), b"new");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
    #[test]
    fn names_without_extensions_and_dotfiles() {
        assert_eq!(candidate(Path::new("README"), 1).unwrap(), Path::new("README (1)"));
        assert_eq!(candidate(Path::new(".hidden"), 2).unwrap(), Path::new(".hidden (2)"));
    }
}

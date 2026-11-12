//! No-clobber destination naming. A free-name check is advisory; publication
//! must still use an atomic operation because another task may win the race.
use std::{
    io,
    path::{Path, PathBuf},
};

const LIMIT: usize = 9999;

fn candidate(path: &Path, index: usize) -> io::Result<PathBuf> {
    if index == 0 {
        return Ok(path.to_owned());
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid destination name"))?;
    let extension = path.extension().and_then(|s| s.to_str());
    // A prior advisory selection may already have a numeric suffix. Continue
    // that sequence rather than nesting another suffix on a late collision.
    let suffix = stem
        .strip_suffix(')')
        .and_then(|s| s.rsplit_once(" ("))
        .filter(|(_, digits)| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
    let (base, number) = if let Some((base, digits)) = suffix {
        (base, add_decimal(digits, index))
    } else {
        (stem, index.to_string())
    };
    let name = match extension {
        Some(ext) => format!("{base} ({number}).{ext}"),
        None => format!("{base} ({number})"),
    };
    Ok(path.with_file_name(name))
}

// The suffix can be longer than an integer type even when the filename itself
// is valid. Add the bounded attempt count without parsing or overflowing.
fn add_decimal(digits: &str, mut add: usize) -> String {
    let mut out = Vec::with_capacity(digits.len() + 1);
    for digit in digits.bytes().rev() {
        let value = (digit - b'0') as usize + add;
        out.push(b'0' + (value % 10) as u8);
        add = value / 10;
    }
    while add > 0 {
        out.push(b'0' + (add % 10) as u8);
        add /= 10;
    }
    while out.len() > 1 && out.last() == Some(&b'0') {
        out.pop();
    }
    out.reverse();
    String::from_utf8(out).expect("decimal digits are UTF-8")
}

pub(super) async fn available(path: &Path) -> io::Result<PathBuf> {
    available_from(path, false).await
}

pub(super) async fn available_from(path: &Path, renamed: bool) -> io::Result<PathBuf> {
    for index in usize::from(renamed)..=LIMIT {
        let name = candidate(path, index)?;
        match tokio::fs::symlink_metadata(&name).await {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(name),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "destination name limit exhausted",
    ))
}

/// Result of attempting publication under a name explicitly approved by the user.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ConfirmedPublication {
    Published(PathBuf),
    NeedsConfirmation(PathBuf),
}

/// Publish only under the approved name. A collision proposes a suffix but does
/// not write to it: callers must obtain a new confirmation and retry. The
/// no-clobber link also covers a collision occurring after confirmation.
/// Callers must validate that both paths belong to the same bound task root and
/// must persist pending confirmation before exposing this flow to clients.
#[allow(dead_code)]
pub(super) async fn publish_confirmed(
    partial: &Path,
    approved: &Path,
) -> io::Result<ConfirmedPublication> {
    match tokio::fs::hard_link(partial, approved).await {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Ok(ConfirmedPublication::NeedsConfirmation(
                available_from(approved, true).await?,
            ));
        }
        Err(error) => return Err(error),
        Ok(()) => {}
    }
    // A sync error occurs after publication, so it must never be interpreted as
    // a collision or trigger another link under a different name.
    #[cfg(unix)]
    {
        let parent = approved
            .parent()
            .ok_or_else(|| io::Error::other("destination has no directory"))?
            .to_owned();
        tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
            .await
            .map_err(io::Error::other)??;
    }
    Ok(ConfirmedPublication::Published(approved.to_owned()))
}

/// Interim fail-closed publication for confirmed starts. Until the durable
/// second-confirmation flow is integrated, a collision is an error and the
/// caller must retain the partial; a suggested suffix is never published.
pub(super) async fn publish_for_task(
    partial: &Path,
    proposed: &Path,
    confirmed: bool,
) -> io::Result<PathBuf> {
    if !confirmed {
        return publish(partial, proposed).await;
    }
    match publish_confirmed(partial, proposed).await? {
        ConfirmedPublication::Published(path) => Ok(path),
        ConfirmedPublication::NeedsConfirmation(candidate) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "destination already exists; confirmation required for {}",
                candidate.display()
            ),
        )),
    }
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
                    let parent = dest
                        .parent()
                        .ok_or_else(|| io::Error::other("destination has no directory"))?
                        .to_owned();
                    tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
                        .await
                        .map_err(io::Error::other)??;
                }
                return Ok(dest);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "destination name limit exhausted",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn confirmed_publication_never_uses_a_suffix_without_new_approval() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rget-confirmed-publish-{}-{nonce}",
            std::process::id()
        ));
        tokio::fs::create_dir(&dir).await.unwrap();
        let original = dir.join("file.txt");
        let first = dir.join("file (1).txt");
        let second = dir.join("file (2).txt");
        let partial = dir.join("partial");
        tokio::fs::write(&original, b"existing").await.unwrap();
        tokio::fs::write(&partial, b"download").await.unwrap();

        assert_eq!(
            publish_confirmed(&partial, &original).await.unwrap(),
            ConfirmedPublication::NeedsConfirmation(first.clone())
        );
        assert!(!first.exists());
        assert_eq!(tokio::fs::read(&original).await.unwrap(), b"existing");
        // Someone claims the suggested name before the user approves it.
        tokio::fs::write(&first, b"race").await.unwrap();
        assert_eq!(
            publish_confirmed(&partial, &first).await.unwrap(),
            ConfirmedPublication::NeedsConfirmation(second.clone())
        );
        assert!(!second.exists());
        assert_eq!(tokio::fs::read(&first).await.unwrap(), b"race");
        assert_eq!(
            publish_confirmed(&partial, &second).await.unwrap(),
            ConfirmedPublication::Published(second.clone())
        );
        assert_eq!(tokio::fs::read(&second).await.unwrap(), b"download");
        assert_eq!(tokio::fs::read(&partial).await.unwrap(), b"download");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn two_confirmed_publications_cannot_claim_the_same_name() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rget-confirmed-race-{}-{nonce}",
            std::process::id()
        ));
        tokio::fs::create_dir(&dir).await.unwrap();
        let destination = dir.join("file.txt");
        let suffix = dir.join("file (1).txt");
        let first = dir.join("partial-a");
        let second = dir.join("partial-b");
        tokio::fs::write(&first, b"first").await.unwrap();
        tokio::fs::write(&second, b"second").await.unwrap();
        let (a, b) = tokio::join!(
            publish_confirmed(&first, &destination),
            publish_confirmed(&second, &destination)
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(
            (a == ConfirmedPublication::Published(destination.clone())
                && b == ConfirmedPublication::NeedsConfirmation(suffix.clone()))
                || (b == ConfirmedPublication::Published(destination.clone())
                    && a == ConfirmedPublication::NeedsConfirmation(suffix.clone()))
        );
        assert!(!suffix.exists());
        assert_eq!(tokio::fs::read(&first).await.unwrap(), b"first");
        assert_eq!(tokio::fs::read(&second).await.unwrap(), b"second");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn multiple_collisions_and_late_race_never_overwrite() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rget-names-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let original = dir.join("file.tar.gz");
        let one = dir.join("file.tar (1).gz");
        let two = dir.join("file.tar (2).gz");
        let partial = dir.join("partial");
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
    #[tokio::test]
    async fn selected_suffix_continues_after_a_late_collision() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rget-late-name-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let original = dir.join("file.ext");
        let partial = dir.join("partial");
        tokio::fs::write(&original, b"original").await.unwrap();
        tokio::fs::write(&partial, b"download").await.unwrap();
        let selected = available(&original).await.unwrap();
        assert_eq!(selected, dir.join("file (1).ext"));
        tokio::fs::write(&selected, b"other").await.unwrap();
        let final_path = publish(&partial, &selected).await.unwrap();
        assert_eq!(final_path, dir.join("file (2).ext"));
        assert_eq!(tokio::fs::read(&selected).await.unwrap(), b"other");
        assert_eq!(tokio::fs::read(&final_path).await.unwrap(), b"download");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn simultaneous_publications_atomically_claim_distinct_names() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rget-parallel-name-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let first = dir.join("partial-a");
        let second = dir.join("partial-b");
        let dest = dir.join("file.ext");
        tokio::fs::write(&first, b"first").await.unwrap();
        tokio::fs::write(&second, b"second").await.unwrap();
        let (a, b) = tokio::join!(publish(&first, &dest), publish(&second, &dest));
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(a, b);
        assert!([&a, &b].contains(&&dest));
        assert!([&a, &b].contains(&&dir.join("file (1).ext")));
        assert_eq!(tokio::fs::read(&a).await.unwrap(), b"first");
        assert_eq!(tokio::fs::read(&b).await.unwrap(), b"second");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn names_without_extensions_and_dotfiles() {
        assert_eq!(
            candidate(Path::new("README"), 1).unwrap(),
            Path::new("README (1)")
        );
        assert_eq!(
            candidate(Path::new(".hidden"), 2).unwrap(),
            Path::new(".hidden (2)")
        );
        assert_eq!(
            candidate(Path::new("report (009).txt"), 1).unwrap(),
            Path::new("report (10).txt")
        );
        assert_eq!(
            candidate(Path::new("report (999999999999999999999).txt"), 1).unwrap(),
            Path::new("report (1000000000000000000000).txt")
        );
    }
}

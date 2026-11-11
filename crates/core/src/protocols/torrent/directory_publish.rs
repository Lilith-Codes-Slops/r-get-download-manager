//! Linux GNU no-replace rename primitive; not yet wired into torrent downloads.
//!
//! Callers must prove ownership, validate and sync the whole staging tree,
//! prevent concurrent modifications, and reconcile after crashes. This module
//! alone is not a complete safe multi-file publication or cleanup protocol.
use super::multi_file_layout::{candidate, validate_component};
use std::{fs::File, io};

/// A successful rename is never represented as an ordinary failure: even when
/// syncing either parent fails, the destination already exists and must be
/// reconciled, not retried or deleted as an uncommitted partial.
#[derive(Debug)]
pub struct PublishedDirectory {
    pub name: String,
    pub sync_error: Option<io::Error>,
}

/// Rename to one exact journaled destination; never silently try a suffix.
/// The caller must durably record this exact name before invoking the rename.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn publish_exact(
    source_parent: &File,
    source_name: &str,
    destination_parent: &File,
    destination_name: &str,
) -> io::Result<PublishedDirectory> {
    use nix::fcntl::{renameat2, RenameFlags};
    validate_component(source_name)?;
    validate_component(destination_name)?;
    renameat2(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    let source_sync = source_parent.sync_all();
    let destination_sync = destination_parent.sync_all();
    Ok(PublishedDirectory {
        name: destination_name.to_owned(),
        sync_error: source_sync.and(destination_sync).err(),
    })
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn publish_exact(
    _source_parent: &File,
    _source_name: &str,
    _destination_parent: &File,
    _destination_name: &str,
) -> io::Result<PublishedDirectory> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace directory rename unavailable",
    ))
}

/// Atomically move a caller-owned directory relative to anchored parent
/// descriptors, never replacing an existing file, directory, or symlink.
/// Source validation and ownership are the caller's responsibility.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn publish(
    source_parent: &File,
    source_name: &str,
    destination_parent: &File,
    root_name: &str,
) -> io::Result<PublishedDirectory> {
    use nix::{
        errno::Errno,
        fcntl::{renameat2, RenameFlags},
    };
    validate_component(source_name)?;
    validate_component(root_name)?;
    for index in 0..=9999 {
        let name = candidate(std::path::Path::new(""), root_name, index)?;
        match renameat2(
            source_parent,
            source_name,
            destination_parent,
            name.as_path(),
            RenameFlags::RENAME_NOREPLACE,
        ) {
            Ok(()) => {
                let source_sync = source_parent.sync_all();
                let destination_sync = destination_parent.sync_all();
                let sync_error = source_sync.and(destination_sync).err();
                return Ok(PublishedDirectory {
                    name: name.to_string_lossy().into_owned(),
                    sync_error,
                });
            }
            Err(Errno::EEXIST) => continue,
            Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "directory destination name limit exhausted",
    ))
}

/// Fail closed on platforms where this wrapper cannot provide no-replace.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn publish(
    _source_parent: &File,
    _source_name: &str,
    _destination_parent: &File,
    _root_name: &str,
) -> io::Result<PublishedDirectory> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace directory rename unavailable",
    ))
}

#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn lab() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rget-directory-publish-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn existing_files_and_empty_directories_are_never_replaced() {
        let root = lab();
        let parent = File::open(&root).unwrap();
        std::fs::write(root.join("name"), b"existing").unwrap();
        std::fs::create_dir(root.join("name (1)")).unwrap();
        std::os::unix::fs::symlink("missing-target", root.join("name (2)")).unwrap();
        std::fs::create_dir(root.join("stage")).unwrap();
        std::fs::write(root.join("stage/payload"), b"new").unwrap();
        let result = publish(&parent, "stage", &parent, "name").unwrap();
        assert!(result.sync_error.is_none());
        assert_eq!(result.name, "name (3)");
        assert_eq!(std::fs::read(root.join("name")).unwrap(), b"existing");
        assert!(root.join("name (1)").is_dir());
        assert!(std::fs::symlink_metadata(root.join("name (2)"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read(root.join("name (3)/payload")).unwrap(),
            b"new"
        );
        assert!(!root.join("stage").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_publications_claim_distinct_names() {
        let root = lab();
        for name in ["first", "second"] {
            std::fs::create_dir(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("payload"), name).unwrap();
        }
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|name| {
                let root = root.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let parent = File::open(root).unwrap();
                    barrier.wait();
                    publish(&parent, name, &parent, "download").unwrap()
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_ne!(results[0].name, results[1].name);
        assert!(results.iter().all(|result| result.sync_error.is_none()));
        assert_eq!(
            std::fs::read(root.join(&results[0].name).join("payload")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(root.join(&results[1].name).join("payload")).unwrap(),
            b"second"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_path_traversal_and_leaves_source_intact() {
        let root = lab();
        let parent = File::open(&root).unwrap();
        std::fs::create_dir(root.join("stage")).unwrap();
        assert!(publish(&parent, "stage", &parent, "../escape").is_err());
        assert!(publish(&parent, "../stage", &parent, "safe").is_err());
        assert!(root.join("stage").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }
}

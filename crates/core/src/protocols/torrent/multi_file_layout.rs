//! Validation helpers only. Multi-file downloads remain disabled until the
//! daemon has an atomic no-replace directory publication and recovery path.
use std::{
    io,
    path::{Component, Path, PathBuf},
};

/// Metainfo components must each be exactly one path component (not complete cross-platform validation).
pub fn validate_component(name: &str) -> io::Result<()> {
    let mut components = Path::new(name).components();
    if name.is_empty()
        || name.chars().any(|c| matches!(c, '/' | '\\' | '\0'))
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe torrent path component",
        ));
    }
    Ok(())
}

/// Directory suffixes apply to the complete name, including dots.
pub fn candidate(root: &Path, name: &str, index: u32) -> io::Result<PathBuf> {
    validate_component(name)?;
    if index == 0 {
        Ok(root.join(name))
    } else {
        Ok(root.join(format!("{name} ({index})")))
    }
}

/// Validate a sequence before constructing paths; this does not check for
/// duplicate files, prefix conflicts, symlinks, or filesystem races.
pub fn validate_relative<'a>(parts: impl IntoIterator<Item = &'a str>) -> io::Result<PathBuf> {
    let mut path = PathBuf::new();
    for part in parts {
        validate_component(part)?;
        path.push(part);
    }
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty torrent path",
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_components_and_builds_suffixes() {
        for bad in ["", ".", "..", "../a", "/abs", "a/b", "a\\b", "bad\0name"] {
            assert!(validate_component(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(
            candidate(Path::new("/tmp"), "archive.tar", 1).unwrap(),
            Path::new("/tmp/archive.tar (1)")
        );
        assert_eq!(
            validate_relative(["dir", "file.txt"]).unwrap(),
            Path::new("dir/file.txt")
        );
        assert!(validate_relative(std::iter::empty()).is_err());
    }
}

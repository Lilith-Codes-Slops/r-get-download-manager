//! Strict, bounded metadata validation before creating any payload files.
use super::multi_file_layout::{validate_component, validate_relative};
use std::{collections::BTreeSet, io, path::PathBuf};

const MAX_FILES: usize = 100_000;
const MAX_COMPONENTS: usize = 128;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestFile {
    pub components: Vec<String>,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedManifest {
    root_name: String,
    files: Vec<ManifestFile>,
    total_bytes: u64,
}

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, message) }

impl ValidatedManifest {
    pub fn root_name(&self) -> &str { &self.root_name }
    pub fn files(&self) -> &[ManifestFile] { &self.files }
    pub fn total_bytes(&self) -> u64 { self.total_bytes }

    /// Call only with file lengths and components extracted from validated v1
    /// metainfo. Expected total must independently match the metadata total.
    pub fn new(root_name: String, files: Vec<ManifestFile>, expected_total: u64) -> io::Result<Self> {
        validate_component(&root_name)?;
        if files.is_empty() || files.len() > MAX_FILES { return Err(invalid("invalid torrent file count")); }
        let mut paths = BTreeSet::<PathBuf>::new();
        let mut total = 0u64;
        for file in &files {
            if file.components.is_empty() || file.components.len() > MAX_COMPONENTS { return Err(invalid("invalid component count")); }
            let bytes = file.components.iter().try_fold(0usize, |n, part| n.checked_add(part.len() + 1)).ok_or_else(|| invalid("path length overflow"))?;
            if bytes > MAX_PATH_BYTES { return Err(invalid("torrent path too long")); }
            let path = validate_relative(file.components.iter().map(String::as_str))?;
            if !paths.insert(path) { return Err(invalid("duplicate torrent path")); }
            total = total.checked_add(file.length).ok_or_else(|| invalid("torrent length overflow"))?;
        }
        if total != expected_total { return Err(invalid("torrent length mismatch")); }
        for path in &paths {
            for ancestor in path.ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()) {
                if paths.contains(ancestor) { return Err(invalid("torrent file conflicts with directory")); }
            }
        }
        Ok(Self { root_name, files, total_bytes: total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(parts: &[&str], length: u64) -> ManifestFile {
        ManifestFile { components: parts.iter().map(|s| (*s).into()).collect(), length }
    }
    fn validate(files: Vec<ManifestFile>, total: u64) -> io::Result<ValidatedManifest> {
        ValidatedManifest::new("root".into(), files, total)
    }
    #[test]
    fn rejects_unsafe_paths_duplicate_paths_and_prefix_conflicts() {
        for parts in [vec!["..", "escape"], vec!["/absolute"], vec!["a/b"], vec![""], vec!["."]] {
            assert!(validate(vec![file(&parts, 1)], 1).is_err(), "{parts:?}");
        }
        assert!(validate(vec![file(&["a"], 1), file(&["a"], 2)], 3).is_err());
        assert!(validate(vec![file(&["a", "b"], 1), file(&["a"], 2)], 3).is_err());
        assert!(validate(vec![file(&["a"], 1), file(&["a", "b"], 2)], 3).is_err());
    }
    #[test]
    fn accepts_legitimate_empty_files_but_rejects_mismatched_or_overflowing_totals() {
        let files = vec![file(&["empty"], 0), file(&["nested", "a"], 3)];
        assert_eq!(validate(files.clone(), 3).unwrap().total_bytes, 3);
        assert!(validate(files, 4).is_err());
        assert!(validate(vec![file(&["a"], u64::MAX), file(&["b"], 1)], 0).is_err());
        assert!(validate(vec![], 0).is_err());
        assert!(ValidatedManifest::new("../root".into(), vec![file(&["a"], 1)], 1).is_err());
    }
}

//! Read-only scan of the chosen directory for the prototype Downloads view.
use crate::fake_backend::LibraryItem;
use std::io;
use std::path::Path;

pub const SCAN_SOURCE: &str = "rget:directory-scan";

pub fn scan(directory: &Path) -> io::Result<Vec<LibraryItem>> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".rget-") || !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let size = entry.metadata()?.len();
        result.push(LibraryItem {
            id: format!("directory:{}", path.to_string_lossy()),
            name,
            source: SCAN_SOURCE.into(),
            path: Some(path),
            size_mb: size as f64 / (1024.0 * 1024.0),
            finished: "On disk".into(),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

pub fn merge(state: &mut crate::fake_backend::State, entries: Vec<LibraryItem>) {
    state.library.retain(|item| item.source != SCAN_SOURCE);
    for item in entries {
        if !state
            .library
            .iter()
            .any(|existing| existing.path == item.path)
        {
            state.library.push(item);
        }
    }
    state.selections.retain(|selected| {
        selected.scope != crate::fake_backend::Scope::Library
            || state.library.iter().any(|item| item.id == selected.id)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_is_read_only_and_excludes_internal_files() {
        let root = std::env::temp_dir().join(format!(
            "rget-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file.txt"), b"content").unwrap();
        std::fs::write(root.join(".rget-a.part"), b"partial").unwrap();
        std::fs::create_dir(root.join("folder")).unwrap();
        let items = scan(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "file.txt");
        assert_eq!(std::fs::read(root.join("file.txt")).unwrap(), b"content");
        assert_eq!(
            std::fs::read(root.join(".rget-a.part")).unwrap(),
            b"partial"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

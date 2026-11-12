//! Syntactic validation for the download confirmation form.
use std::path::{Component, Path, PathBuf};

pub fn validate(filename: &str, folder: &str) -> Result<(String, PathBuf), &'static str> {
    let name = filename;
    let mut components = Path::new(name).components();
    if name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("Enter a valid file name without directory separators.");
    }
    let path = PathBuf::from(folder);
    if folder.contains('\0')
        || !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("Enter an absolute folder path without parent traversal.");
    }
    Ok((name.to_owned(), path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_confirmed_name_and_preserves_the_chosen_folder() {
        assert_eq!(
            validate("file.txt", "/downloads /"),
            Ok(("file.txt".into(), PathBuf::from("/downloads /")))
        );
        assert_eq!(
            validate(" file.txt ", "/downloads"),
            Ok((" file.txt ".into(), PathBuf::from("/downloads")))
        );
        for name in ["", " ", ".", "..", "a/b", "a/", "a/.", "a\\b", "a\0b"] {
            assert!(validate(name, "/downloads").is_err(), "{name:?}");
        }
        for folder in ["relative", "/downloads/../other", "/bad\0path"] {
            assert!(validate("file.txt", folder).is_err(), "{folder:?}");
        }
    }
}

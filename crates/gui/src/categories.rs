//! Category assignment and library filtering for the fake state.
use crate::fake_backend::{LibraryItem, State};

/// Classify by the final extension only, ignoring ASCII case.
pub fn category_of<'a>(state: &'a State, name: &str) -> &'a str {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if extension.is_empty() {
        return "other";
    }
    state
        .categories
        .iter()
        .find(|category| {
            category
                .extensions
                .iter()
                .any(|ext| ext.eq_ignore_ascii_case(extension))
        })
        .map(|category| category.id.as_str())
        .unwrap_or("other")
}

pub fn category_name<'a>(state: &'a State, id: &'a str) -> &'a str {
    match id {
        "other" => "Uncategorized",
        "all" => "All",
        _ => state
            .categories
            .iter()
            .find(|category| category.id == id)
            .map(|category| category.name.as_str())
            .unwrap_or(id),
    }
}

pub fn filtered_library(state: &State) -> Vec<&LibraryItem> {
    let query = state.library_filter.trim().to_lowercase();
    state
        .library
        .iter()
        .filter(|item| {
            (state.selected_category == "all"
                || category_of(state, &item.name) == state.selected_category)
                && crate::search_model::matches_query(&item.name, &query)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::{Command, FakeBackend};
    use std::path::PathBuf;

    #[test]
    fn extension_and_search_filters_work_together() {
        let dir = std::env::temp_dir().join(format!("rget-category-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut backend = FakeBackend::open(dir.join("state.json")).unwrap();
        let mut state = backend.snapshot().clone();
        state.library[0].name = "My.FILE.PDF".into();
        let mut other = state.library[0].clone();
        other.id = "other".into();
        other.name = "file.pdf.backup".into();
        state.library.push(other);
        for (name, expected) in [
            ("photo.JPEG", "images"),
            ("movie.MKV", "videos"),
            ("doc.PDF", "pdfs"),
            ("archive.TAR", "compressed"),
            ("paper.DOCX", "documents"),
            ("track.FLAC", "audio"),
            ("system.ISO", "software"),
        ] {
            assert_eq!(category_of(&state, name), expected);
        }
        assert_eq!(category_of(&state, "file.PDF"), "pdfs");
        assert_eq!(category_of(&state, "file.pdf.backup"), "other");
        assert_eq!(category_of(&state, "file"), "other");
        assert_eq!(category_of(&state, "file."), "other");
        state.selected_category = "pdfs".into();
        state.library_filter = " mY.fILE ".into();
        assert_eq!(filtered_library(&state).len(), 1);
        state.library_filter = "backup".into();
        assert!(filtered_library(&state).is_empty());
        state.selected_category = "other".into();
        assert_eq!(filtered_library(&state).len(), 1);
        backend
            .dispatch(Command::SetLibraryFilter("hello".into()))
            .unwrap();
        assert_eq!(
            FakeBackend::open(PathBuf::from(&dir).join("state.json"))
                .unwrap()
                .snapshot()
                .library_filter,
            "hello"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! Suggestions for the shared toolbar search, respecting existing view filters.
use std::collections::HashSet;

use crate::categories::category_of;
use crate::fake_backend::{State, View};

pub fn suggestions<'a>(state: &'a State, query: &str) -> Vec<&'a str> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |name: &'a str| {
        let normalized = name.to_lowercase();
        if normalized.contains(&query) && seen.insert(normalized) {
            names.push(name);
        }
    };
    match state.view {
        View::Tasks => {
            let active_filter = state.active_filter.trim().to_lowercase();
            let seed_filter = state.seed_filter.trim().to_lowercase();
            for task in &state.tasks {
                if task.name.to_lowercase().contains(&active_filter) {
                    push(&task.name);
                }
            }
            for seed in &state.seeds {
                if seed.name.to_lowercase().contains(&seed_filter) {
                    push(&seed.name);
                }
            }
        }
        View::Downloads => {
            for item in &state.library {
                if state.selected_category == "all"
                    || category_of(state, &item.name) == state.selected_category
                {
                    push(&item.name);
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::FakeBackend;

    #[test]
    fn suggestions_follow_view_filters_and_are_live_and_deduplicated() {
        let path = std::env::temp_dir()
            .join(format!("rget-search-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        state.tasks[0].name = "Report.pdf".into();
        let mut duplicate = state.tasks[0].clone();
        duplicate.id = "duplicate".into();
        duplicate.name = "REPORT.PDF".into();
        state.tasks.push(duplicate);
        state.seeds[0].name = "Report.iso".into();
        state.library[0].name = "Report.pdf".into();
        state.view = View::Tasks;
        assert!(suggestions(&state, "  ").is_empty());
        assert_eq!(suggestions(&state, "reP"), vec!["Report.pdf", "Report.iso"]);
        state.active_filter = "missing".into();
        assert_eq!(suggestions(&state, "report"), vec!["Report.iso"]);
        state.view = View::Downloads;
        state.selected_category = "pdfs".into();
        assert_eq!(suggestions(&state, "report"), vec!["Report.pdf"]);
        state.selected_category = "images".into();
        assert!(suggestions(&state, "report").is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

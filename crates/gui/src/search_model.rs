//! Global search over completed downloads, independently of the current view.
use std::io;

use crate::fake_backend::{Scope, Selection, State, View};

#[derive(Debug, PartialEq, Eq)]
pub struct SearchHit<'a> {
    pub name: &'a str,
    pub selection: Selection,
}

impl SearchHit<'_> {
    pub fn scope_label(&self) -> &'static str {
        match self.selection.scope {
            Scope::Task => "Task",
            Scope::Seed => "Transfer",
            Scope::Library => "Download",
        }
    }
}

/// Query must already be trimmed and lowercased. A leading dot searches the
/// final extension, with prefix matching so suggestions update as you type.
pub fn matches_query(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    if let Some(extension) = query.strip_prefix('.') {
        return name == query
            || name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.starts_with(extension));
    }
    name.contains(query)
}

pub fn suggestions<'a>(state: &'a State, query: &str) -> Vec<SearchHit<'a>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for item in &state.library {
        if matches_query(&item.name, &query) {
            matches.push(SearchHit {
                name: &item.name,
                selection: Selection {
                    scope: Scope::Library,
                    id: item.id.clone(),
                },
            });
        }
    }
    // Seeds represent completed downloads, including stopped seeding entries.
    for seed in &state.seeds {
        if matches_query(&seed.name, &query) {
            matches.push(SearchHit {
                name: &seed.name,
                selection: Selection {
                    scope: Scope::Seed,
                    id: seed.id.clone(),
                },
            });
        }
    }
    // Usually the simulation moves 100% tasks to the library on the same tick,
    // but completed tasks may exist in loaded state between transitions.
    for task in &state.tasks {
        if task.progress.is_finite() && task.progress >= 100.0 && matches_query(&task.name, &query)
        {
            matches.push(SearchHit {
                name: &task.name,
                selection: Selection {
                    scope: Scope::Task,
                    id: task.id.clone(),
                },
            });
        }
    }
    matches
}

/// Navigate to a result and make it visible despite a previous view filter.
/// The backend calls this on a cloned state before committing it to disk.
pub fn select_result(state: &mut State, item: Selection) -> io::Result<()> {
    let name = match item.scope {
        Scope::Library => state
            .library
            .iter()
            .find(|entry| entry.id == item.id)
            .map(|entry| entry.name.clone()),
        Scope::Seed => state
            .seeds
            .iter()
            .find(|entry| entry.id == item.id)
            .map(|entry| entry.name.clone()),
        Scope::Task => state
            .tasks
            .iter()
            .find(|entry| {
                entry.id == item.id && entry.progress.is_finite() && entry.progress >= 100.0
            })
            .map(|entry| entry.name.clone()),
    }
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Search result is no longer available",
        )
    })?;
    match item.scope {
        Scope::Library => {
            state.view = View::Downloads;
            state.selected_category = "all".into();
        }
        Scope::Seed => {
            state.view = View::Tasks;
            state.seed_filter.clear();
        }
        Scope::Task => {
            state.view = View::Tasks;
            state.active_filter.clear();
        }
    }
    state.library_filter = name;
    state.selections = vec![item.clone()];
    state.selection_anchor = Some(item);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::{FakeBackend, View};

    #[test]
    fn searches_both_views_and_excludes_incomplete_tasks() {
        let path = std::env::temp_dir()
            .join(format!("rget-global-search-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        state.library[0].name = "Completed.TXT".into();
        state.seeds[0].name = "Seed.txt".into();
        state.seeds[0].running = false;
        state.tasks[0].name = "Incomplete.txt".into();
        state.tasks[0].progress = 99.0;
        let mut completed = state.tasks[0].clone();
        completed.id = "completed-task".into();
        completed.name = "Task.txt".into();
        completed.progress = 100.0;
        state.tasks.push(completed);
        state.active_filter = "unrelated".into();
        state.seed_filter = "unrelated".into();
        state.selected_category = "images".into();
        assert!(suggestions(&state, "  ").is_empty());
        for view in [View::Tasks, View::Downloads] {
            state.view = view;
            let names: Vec<_> = suggestions(&state, ".tXt")
                .into_iter()
                .map(|hit| hit.name)
                .collect();
            assert_eq!(names, ["Completed.TXT", "Seed.txt", "Task.txt"]);
            assert_eq!(suggestions(&state, "complete").len(), 1);
        }
        assert!(matches_query("notes.TxT", ".t"));
        assert!(matches_query(".config.txt", ".config.txt"));
        assert!(!matches_query("notes.txt.zip", ".txt"));
        assert!(!matches_query("extensionless", ".txt"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

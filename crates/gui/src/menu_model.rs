//! Pure selection, action availability, and clipboard content rules.
use crate::fake_backend::{Scope, Selection, State, TaskStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowCommand {
    Pause,
    Resume,
    Retry,
    ToggleSeed,
    OpenFolder,
    CopyLink,
    Copy,
    Cut,
    Properties,
    Remove,
    Delete,
}

pub fn allowed(state: &State, item: &Selection, command: RowCommand) -> bool {
    let task = state
        .tasks
        .iter()
        .find(|task| item.scope == Scope::Task && item.id == task.id);
    let seed = state
        .seeds
        .iter()
        .find(|seed| item.scope == Scope::Seed && item.id == seed.id);
    let exists = task.is_some()
        || seed.is_some()
        || (item.scope == Scope::Library && state.library.iter().any(|entry| entry.id == item.id));
    match command {
        RowCommand::Pause => task.is_some_and(|t| matches!(t.status, TaskStatus::Active)),
        RowCommand::Resume => task.is_some_and(|t| matches!(t.status, TaskStatus::Paused)),
        RowCommand::Retry => task.is_some_and(|t| matches!(t.status, TaskStatus::Failed)),
        RowCommand::ToggleSeed => seed.is_some(),
        RowCommand::CopyLink => {
            item_source(state, item).is_some_and(|(_, source)| !source.is_empty())
        }
        RowCommand::Copy | RowCommand::Cut => {
            clipboard_text(state, std::slice::from_ref(item), false).is_some()
        }
        _ => exists,
    }
}

pub fn item_source<'a>(
    state: &'a State,
    item: &Selection,
) -> Option<(Option<&'a std::path::Path>, &'a str)> {
    match item.scope {
        Scope::Task => state
            .tasks
            .iter()
            .find(|t| t.id == item.id)
            .map(|t| (t.path.as_deref(), t.source.as_str())),
        Scope::Seed => state
            .seeds
            .iter()
            .find(|s| s.id == item.id)
            .map(|s| (s.path.as_deref(), s.source.as_str())),
        Scope::Library => state
            .library
            .iter()
            .find(|l| l.id == item.id)
            .map(|l| (l.path.as_deref(), l.source.as_str())),
    }
}

/// Use real paths if known, otherwise the actual stored source. Never invent a path.
pub fn clipboard_text(state: &State, items: &[Selection], link_only: bool) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(items.len());
    for item in items {
        let (path, source) = item_source(state, item)?;
        let value = if link_only {
            source
        } else if let Some(path) = path {
            path.to_str()?
        } else {
            source
        };
        if value.is_empty() || value.contains(['\n', '\r']) {
            return None;
        }
        lines.push(value);
    }
    Some(lines.join("\n"))
}

/// A context click on an already selected row retains the multi-selection;
/// otherwise the context action applies only to the clicked row.
pub fn targets(state: &State, item: &Selection) -> Vec<Selection> {
    if state.selections.contains(item) {
        state.selections.clone()
    } else {
        vec![item.clone()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::FakeBackend;
    #[test]
    fn status_actions_and_real_clipboard_values() {
        let path = std::env::temp_dir()
            .join(format!("rget-menus-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let task = Selection {
            scope: Scope::Task,
            id: state.tasks[0].id.clone(),
        };
        let file = Selection {
            scope: Scope::Library,
            id: state.library[0].id.clone(),
        };
        assert!(allowed(&state, &task, RowCommand::Pause));
        assert!(!allowed(&state, &task, RowCommand::Retry));
        state.tasks[0].status = TaskStatus::Failed;
        assert!(allowed(&state, &task, RowCommand::Retry));
        assert!(!allowed(&state, &file, RowCommand::ToggleSeed));
        state.tasks[0].path = Some("/tmp/real path.iso".into());
        state.selections = vec![task.clone(), file.clone()];
        assert_eq!(targets(&state, &task), state.selections);
        assert_eq!(
            clipboard_text(&state, &state.selections, false).unwrap(),
            format!("/tmp/real path.iso\n{}", state.library[0].source)
        );
        assert_eq!(
            clipboard_text(&state, std::slice::from_ref(&task), true).unwrap(),
            state.tasks[0].source
        );
        state.tasks[0].path = None;
        state.tasks[0].source.clear();
        assert!(clipboard_text(&state, &[task], false).is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

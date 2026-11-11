//! Toast messages for the requested actions, independent of presentation.
use crate::fake_backend::{Command, State};

pub fn action_message(command: &Command, state: &State) -> Option<String> {
    match command {
        Command::PauseAll => Some("Paused all downloads".into()),
        Command::ResumeAll => Some("Resumed all downloads".into()),
        Command::CreateCategory { name, .. } => Some(format!("Category created: {}", name.trim())),
        Command::UpdateCategory { name, .. } => Some(format!("Category updated: {}", name.trim())),
        Command::DeleteCategory(id) => state
            .categories
            .iter()
            .find(|category| &category.id == id)
            .map(|category| format!("Category deleted: {}", category.name)),
        _ => None,
    }
}

pub fn visible_with_alerts_off(text: &str) -> bool {
    text.starts_with("Added ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::FakeBackend;

    #[test]
    fn action_messages_use_normalized_names_and_exact_wording() {
        let path = std::env::temp_dir()
            .join(format!("rget-toast-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        for (command, expected) in [
            (Command::PauseAll, "Paused all downloads"),
            (Command::ResumeAll, "Resumed all downloads"),
            (
                Command::CreateCategory {
                    name: "  Books  ".into(),
                    extensions: vec!["epub".into()],
                },
                "Category created: Books",
            ),
        ] {
            let message = action_message(&command, backend.snapshot());
            backend.dispatch(command).unwrap();
            assert_eq!(message.as_deref(), Some(expected));
        }
        let id = backend.snapshot().selected_category.clone();
        let update = Command::UpdateCategory {
            id: id.clone(),
            name: "  Reading  ".into(),
            extensions: vec!["epub".into()],
        };
        let message = action_message(&update, backend.snapshot());
        backend.dispatch(update).unwrap();
        assert_eq!(message.as_deref(), Some("Category updated: Reading"));
        let delete = Command::DeleteCategory(id);
        let message = action_message(&delete, backend.snapshot());
        backend.dispatch(delete).unwrap();
        assert_eq!(message.as_deref(), Some("Category deleted: Reading"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn only_added_toasts_bypass_disabled_alerts() {
        assert!(visible_with_alerts_off("Added file.iso"));
        assert!(!visible_with_alerts_off("Paused all downloads"));
        assert!(!visible_with_alerts_off("Category created: Books"));
    }
}

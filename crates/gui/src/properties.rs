//! Build the reference Properties fields from a fake-backend snapshot.
use crate::categories::{category_name, category_of};
use crate::fake_backend::{Scope, Selection, State, TaskStatus};

pub fn rows(state: &State, selected: &Selection) -> Option<Vec<(&'static str, String)>> {
    match selected.scope {
        Scope::Task => {
            let item = state.tasks.iter().find(|item| item.id == selected.id)?;
            let status = match item.status {
                TaskStatus::Failed => "Failed",
                TaskStatus::Paused => "Paused",
                TaskStatus::Active if !state.daemon_connected => "Waiting for daemon",
                TaskStatus::Active if item.queued => "Queued",
                TaskStatus::Active => "Downloading",
            };
            let progress = if item.progress.is_finite() {
                item.progress.floor()
            } else {
                0.0
            };
            Some(vec![
                ("Name", item.name.clone()),
                ("Status", status.into()),
                ("Progress", format!("{progress:.0}%")),
                ("Size", size(item.size_mb)),
                ("Connections", item.connections.to_string()),
                ("Source", source(&item.name, &item.source)),
            ])
        }
        Scope::Seed => {
            let item = state.seeds.iter().find(|item| item.id == selected.id)?;
            Some(vec![
                ("Name", item.name.clone()),
                ("Ratio", format!("{:.2}", item.ratio)),
                ("Size", size(item.size_mb)),
                (
                    "Status",
                    if item.running { "Seeding" } else { "Stopped" }.into(),
                ),
            ])
        }
        Scope::Library => {
            let item = state.library.iter().find(|item| item.id == selected.id)?;
            Some(vec![
                ("Name", item.name.clone()),
                ("Size", size(item.size_mb)),
                (
                    "Category",
                    category_name(state, category_of(state, &item.name)).to_owned(),
                ),
                ("Finished", item.finished.clone()),
            ])
        }
    }
}

fn source(name: &str, source: &str) -> String {
    if !source.is_empty() {
        return source.to_owned();
    }
    let encoded: String = name
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(byte) {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("https://downloads.example.com/{encoded}")
}

fn size(mb: f64) -> String {
    if mb >= 10_000.0 {
        format!("{:.0} GB", mb / 1000.0)
    } else if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", mb * 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::FakeBackend;
    #[test]
    fn properties_include_each_reference_field_for_each_scope() {
        let path = std::env::temp_dir()
            .join(format!("rget-props-{}", std::process::id()))
            .join("state.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let backend = FakeBackend::open(path.clone()).unwrap();
        let mut state = backend.snapshot().clone();
        let task = Selection {
            scope: Scope::Task,
            id: state.tasks[0].id.clone(),
        };
        let seed = Selection {
            scope: Scope::Seed,
            id: state.seeds[0].id.clone(),
        };
        let file = Selection {
            scope: Scope::Library,
            id: state.library[0].id.clone(),
        };
        let keys = |rows: Vec<(&'static str, String)>| {
            rows.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };
        assert_eq!(
            keys(rows(&state, &task).unwrap()),
            [
                "Name",
                "Status",
                "Progress",
                "Size",
                "Connections",
                "Source"
            ]
        );
        assert_eq!(
            keys(rows(&state, &seed).unwrap()),
            ["Name", "Ratio", "Size", "Status"]
        );
        assert_eq!(
            keys(rows(&state, &file).unwrap()),
            ["Name", "Size", "Category", "Finished"]
        );
        assert_eq!(rows(&state, &file).unwrap()[2].1, "PDFs");
        state.tasks[0].source.clear();
        state.tasks[0].name = "sp ace.zip".into();
        assert_eq!(
            rows(&state, &task).unwrap()[5].1,
            "https://downloads.example.com/sp%20ace.zip"
        );
        state.daemon_connected = false;
        assert_eq!(rows(&state, &task).unwrap()[1].1, "Waiting for daemon");
        state.tasks[0].status = TaskStatus::Paused;
        assert_eq!(rows(&state, &task).unwrap()[1].1, "Paused");
        state.tasks[0].status = TaskStatus::Failed;
        assert_eq!(rows(&state, &task).unwrap()[1].1, "Failed");
        assert!(rows(
            &state,
            &Selection {
                scope: Scope::Library,
                id: "missing".into()
            }
        )
        .is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

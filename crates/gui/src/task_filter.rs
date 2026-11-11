//! Filtering rules shared by the native Tasks panes.
use crate::fake_backend::{Seed, State, Task};

pub fn active_items(state: &State) -> Vec<&Task> {
    let query = state.active_filter.trim().to_lowercase();
    state
        .tasks
        .iter()
        .filter(|task| matches_name(&task.name, &query))
        .collect()
}

pub fn transfer_items(state: &State) -> Vec<&Seed> {
    let query = state.seed_filter.trim().to_lowercase();
    state
        .seeds
        .iter()
        .filter(|seed| matches_name(&seed.name, &query))
        .collect()
}

fn matches_name(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::{Command, FakeBackend, TaskStatus};

    #[test]
    fn filters_are_independent_and_stopped_seeds_remain_visible() {
        let dir = std::env::temp_dir().join(format!("rget-task-filter-{}", std::process::id()));
        let path = dir.join("state.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut backend = FakeBackend::open(path).unwrap();
        let mut state = backend.snapshot().clone();
        let mut paused = state.tasks[0].clone();
        paused.name = "Paused.ZIP".into();
        paused.status = TaskStatus::Paused;
        let mut failed = paused.clone();
        failed.name = "Failed.zip".into();
        failed.status = TaskStatus::Failed;
        state.tasks.extend([paused, failed]);
        let mut stopped = state.seeds[0].clone();
        stopped.name = "Stopped.mp4".into();
        stopped.running = false;
        state.seeds.push(stopped);
        state.active_filter = " zIp ".into();
        state.seed_filter = "VIDEO".into();
        assert_eq!(active_items(&state).len(), 2);
        assert_eq!(transfer_items(&state).len(), 1);
        state.seed_filter = "stopped".into();
        assert_eq!(transfer_items(&state).len(), 1);
        assert!(!transfer_items(&state)[0].running);
        assert_eq!(active_items(&state).len(), 2);
        state.seed_filter.clear();
        state.daemon_connected = false;
        assert_eq!(transfer_items(&state).len(), 2);
        assert_eq!(active_items(&state).len(), 2);
        backend
            .dispatch(Command::SetActiveFilter("paused".into()))
            .unwrap();
        backend
            .dispatch(Command::SetSeedFilter("video".into()))
            .unwrap();
        let restored = FakeBackend::open(dir.join("state.json")).unwrap();
        assert_eq!(restored.snapshot().active_filter, "paused");
        assert_eq!(restored.snapshot().seed_filter, "video");
        let _ = std::fs::remove_dir_all(dir);
    }
}

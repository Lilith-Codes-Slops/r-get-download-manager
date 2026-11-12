//! The reference's 500 ms fake-transfer tick. Never touches downloaded files.
use std::io;
use std::time::Duration;

use crate::fake_backend::{LibraryItem, Scope, Selection, State, TaskStatus};

pub const TICK_INTERVAL: Duration = Duration::from_millis(500);
const SIM: f64 = 4.0;

/// Advance one reference tick. Random samples must be in [0, 1), just like
/// Math.random(). The caller supplies them so the math is deterministic in tests.
pub fn tick(state: &mut State, mut random: impl FnMut() -> f64) -> io::Result<()> {
    let max = state.settings.max_concurrent.clamp(1, 10) as usize;
    let mut active_index = 0usize;
    for task in &mut state.tasks {
        if matches!(task.status, TaskStatus::Active) {
            task.queued = active_index >= max;
            active_index += 1;
        }
    }
    let running = if state.daemon_connected {
        active_index.min(max)
    } else {
        0
    };
    let cap = state
        .settings
        .speed_limit_kbps
        .map_or(f64::INFINITY, |limit| limit / running.max(1) as f64);
    let mut finished = Vec::new();
    let mut failed = Vec::new();
    let seconds = TICK_INTERVAL.as_secs_f64() * SIM;
    for (index, task) in state.tasks.iter_mut().enumerate() {
        if !state.daemon_connected || !matches!(task.status, TaskStatus::Active) || task.queued {
            task.speed_kbps = 0.0;
            continue;
        }
        validate_values(task.size_mb, task.base_kbps, task.progress)?;
        let speed = (task.base_kbps * (0.85 + sample(&mut random)? * 0.3)).min(cap);
        let progress =
            (task.progress + speed * seconds / (task.size_mb * 1000.0) * 100.0).min(100.0);
        if !speed.is_finite() || !progress.is_finite() {
            return Err(invalid_simulation());
        }
        task.speed_kbps = speed;
        task.progress = progress;
        if progress >= 100.0 {
            finished.push(index);
        } else if task.fail_at.is_some_and(|threshold| progress >= threshold) {
            failed.push(index);
        }
    }
    for seed in &mut state.seeds {
        if seed.running && state.daemon_connected {
            validate_values(seed.size_mb, seed.base_upload_kbps, seed.ratio)?;
            let speed = seed.base_upload_kbps * (0.9 + sample(&mut random)? * 0.2);
            let ratio = seed.ratio + speed * seconds / (seed.size_mb * 1000.0);
            if !speed.is_finite() || !ratio.is_finite() {
                return Err(invalid_simulation());
            }
            seed.speed_kbps = speed;
            seed.ratio = ratio;
            if ratio >= state.settings.seed_ratio {
                seed.running = false;
                seed.speed_kbps = 0.0;
            }
        } else {
            seed.speed_kbps = 0.0;
        }
    }
    // HTML finishes before failing; retain its ordering for library insertions.
    for index in failed {
        let task = &mut state.tasks[index];
        task.status = TaskStatus::Failed;
        task.speed_kbps = 0.0;
        task.queued = false;
    }
    let completed: Vec<_> = finished
        .iter()
        .map(|&index| state.tasks[index].clone())
        .collect();
    for task in completed {
        let id = next_library_id(state)?;
        let selection = Selection {
            scope: Scope::Task,
            id: task.id.clone(),
        };
        state.selections.retain(|item| item != &selection);
        state.favorites.retain(|item| item != &selection);
        if state.selection_anchor.as_ref() == Some(&selection) {
            state.selection_anchor = None;
        }
        state.library.insert(
            0,
            LibraryItem {
                id,
                name: task.name,
                source: task.source,
                path: task.path,
                size_mb: task.size_mb,
                finished: "Just now".into(),
            },
        );
    }
    for index in finished.into_iter().rev() {
        state.tasks.remove(index);
    }
    Ok(())
}

fn next_library_id(state: &mut State) -> io::Result<String> {
    loop {
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Library IDs exhausted"))?;
        let id = format!("l{}", state.next_id);
        if !state.library.iter().any(|item| item.id == id) {
            return Ok(id);
        }
    }
}
fn invalid_simulation() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid simulated transfer values",
    )
}
fn validate_values(size: f64, speed: f64, progress: f64) -> io::Result<()> {
    if size.is_finite() && size > 0.0 && speed.is_finite() && speed >= 0.0 && progress.is_finite() {
        Ok(())
    } else {
        Err(invalid_simulation())
    }
}
fn sample(random: &mut impl FnMut() -> f64) -> io::Result<f64> {
    let value = random();
    if (0.0..1.0).contains(&value) {
        Ok(value)
    } else {
        Err(invalid_simulation())
    }
}

/// Generate inexpensive independent-looking samples; this is not cryptography.
pub struct Random(u64);
impl Random {
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e3779b97f4a7c15 } else { seed })
    }
    pub fn sample(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545f4914f6cdd1d) >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

/// Reconstruct notifications only after the backend's atomic save has succeeded.
/// Ordering follows tick(): ratio notices, then completions, then failures.
pub fn notifications(before: &State, after: &State) -> Vec<String> {
    let mut messages = Vec::new();
    for seed in &before.seeds {
        if seed.running && after.seeds.iter().any(|s| s.id == seed.id && !s.running) {
            messages.push(format!(
                "{} reached ratio {:.1}; seeding stopped",
                seed.name, after.settings.seed_ratio
            ));
        }
    }
    for task in &before.tasks {
        if !after.tasks.iter().any(|t| t.id == task.id) {
            messages.push(format!("Completed: {}", task.name));
        }
    }
    for task in &before.tasks {
        if !matches!(task.status, TaskStatus::Failed)
            && after
                .tasks
                .iter()
                .any(|t| t.id == task.id && matches!(t.status, TaskStatus::Failed))
        {
            messages.push(format!("Failed: {}", task.name));
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::{Command, FakeBackend};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn backend() -> (FakeBackend, std::path::PathBuf) {
        let path = std::env::temp_dir()
            .join(format!(
                "rget-sim-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("state.json");
        (FakeBackend::open(path.clone()).unwrap(), path)
    }
    fn clean(path: &std::path::Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
    fn task(state: &State, id: &str, status: TaskStatus) -> crate::fake_backend::Task {
        let mut task = state.tasks[0].clone();
        task.id = id.into();
        task.name = format!("{id}.iso");
        task.status = status;
        task.progress = 0.0;
        task.size_mb = 1000.0;
        task.base_kbps = 1000.0;
        task.speed_kbps = 3000.0;
        task.fail_at = None;
        task.queued = false;
        task
    }
    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn limit_split_queue_and_progress_follow_reference_formula() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        state.settings.max_concurrent = 2;
        state.settings.speed_limit_kbps = Some(1000.0);
        state.tasks = vec![
            task(&state, "a", TaskStatus::Active),
            task(&state, "b", TaskStatus::Active),
            task(&state, "c", TaskStatus::Active),
            task(&state, "p", TaskStatus::Paused),
            task(&state, "f", TaskStatus::Failed),
        ];
        state.seeds.clear();
        tick(&mut state, || 0.5).unwrap();
        for item in &state.tasks[..2] {
            near(item.speed_kbps, 500.0);
            near(item.progress, 0.1);
            assert!(!item.queued);
        }
        assert!(state.tasks[2].queued);
        for item in &state.tasks[2..] {
            near(item.speed_kbps, 0.0);
            near(item.progress, 0.0);
        }
        // A paused task frees a slot on the following tick; the cap is still split evenly.
        state.tasks[0].status = TaskStatus::Paused;
        tick(&mut state, || 0.5).unwrap();
        assert!(!state.tasks[2].queued);
        near(state.tasks[1].progress, 0.2);
        near(state.tasks[2].progress, 0.1);
        near(state.tasks[0].speed_kbps, 0.0);
        clean(&path);
    }

    #[test]
    fn unlimited_random_factors_and_bounds_are_exact() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        state.settings.max_concurrent = 10;
        state.settings.speed_limit_kbps = None;
        state.tasks = vec![
            task(&state, "a", TaskStatus::Active),
            task(&state, "b", TaskStatus::Active),
        ];
        state.seeds.clear();
        let mut values = [0.0, 0.5].into_iter();
        tick(&mut state, || values.next().unwrap()).unwrap();
        near(state.tasks[0].speed_kbps, 850.0);
        near(state.tasks[0].progress, 0.17);
        near(state.tasks[1].speed_kbps, 1000.0);
        near(state.tasks[1].progress, 0.2);
        let mut rng = Random::new(0);
        for _ in 0..10_000 {
            assert!((0.0..1.0).contains(&rng.sample()));
        }
        clean(&path);
    }

    #[test]
    fn daemon_disconnected_prevents_progress_and_seeding_but_still_queues() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        state.daemon_connected = false;
        state.settings.max_concurrent = 1;
        state.tasks = vec![
            task(&state, "a", TaskStatus::Active),
            task(&state, "b", TaskStatus::Active),
        ];
        let original_ratio = state.seeds[0].ratio;
        state.seeds[0].speed_kbps = 500.0;
        tick(&mut state, || {
            panic!("disconnected transfers must not consume randomness")
        })
        .unwrap();
        assert!(state.tasks[1].queued);
        assert!(state
            .tasks
            .iter()
            .all(|t| t.progress == 0.0 && t.speed_kbps == 0.0));
        assert_eq!(state.seeds[0].ratio, original_ratio);
        assert_eq!(state.seeds[0].speed_kbps, 0.0);
        assert!(state.seeds[0].running);
        state.daemon_connected = true;
        tick(&mut state, || 0.5).unwrap();
        assert!(state.tasks[0].progress > 0.0);
        assert_eq!(state.tasks[1].progress, 0.0);
        assert!(state.seeds[0].ratio > original_ratio);
        clean(&path);
    }

    #[test]
    fn completion_precedes_failure_and_seed_ratio_stop_preserves_real_source() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        let mut finished = task(&state, "finish", TaskStatus::Active);
        finished.progress = 99.95;
        finished.fail_at = Some(1.0);
        finished.path = Some("/actual/source.iso".into());
        finished.source = "https://host/source.iso".into();
        let mut failed = task(&state, "fail", TaskStatus::Active);
        failed.progress = 9.9;
        failed.fail_at = Some(10.0);
        state.tasks = vec![finished, failed];
        let selected = Selection {
            scope: Scope::Task,
            id: "finish".into(),
        };
        state.selections = vec![selected.clone()];
        state.selection_anchor = Some(selected.clone());
        state.favorites = vec![selected];
        state.settings.seed_ratio = 2.0;
        state.seeds[0].ratio = 1.999;
        state.seeds[0].size_mb = 1000.0;
        state.seeds[0].base_upload_kbps = 1000.0;
        let before = state.clone();
        tick(&mut state, || 0.5).unwrap();
        assert_eq!(state.tasks.len(), 1);
        assert!(matches!(state.tasks[0].status, TaskStatus::Failed));
        near(state.tasks[0].speed_kbps, 0.0);
        assert!(!state.seeds[0].running);
        near(state.seeds[0].ratio, 2.001);
        near(state.seeds[0].speed_kbps, 0.0);
        assert!(
            state.selections.is_empty()
                && state.favorites.is_empty()
                && state.selection_anchor.is_none()
        );
        let entry = &state.library[0];
        assert_eq!(entry.name, "finish.iso");
        assert_eq!(entry.finished, "Just now");
        assert_eq!(
            entry.path.as_deref(),
            Some(std::path::Path::new("/actual/source.iso"))
        );
        assert_eq!(entry.source, "https://host/source.iso");
        assert_eq!(
            notifications(&before, &state),
            vec![
                format!(
                    "{} reached ratio 2.0; seeding stopped",
                    before.seeds[0].name
                ),
                "Completed: finish.iso".into(),
                "Failed: fail.iso".into()
            ]
        );
        let old = state.clone();
        tick(&mut state, || panic!("no transfers running")).unwrap();
        assert!(notifications(&old, &state).is_empty());
        clean(&path);
    }

    #[test]
    fn multiple_completions_are_prepended_in_reference_order_and_ids_are_unique() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        let mut a = task(&state, "a", TaskStatus::Active);
        let mut b = task(&state, "b", TaskStatus::Active);
        a.progress = 99.9;
        b.progress = 99.9;
        state.tasks = vec![a, b];
        state.seeds.clear();
        state.next_id = 20;
        state.library[0].id = "l21".into();
        tick(&mut state, || 0.5).unwrap();
        assert!(state.tasks.is_empty());
        assert_eq!(
            (&state.library[0].name, &state.library[1].name),
            (&"b.iso".to_string(), &"a.iso".to_string())
        );
        assert_eq!(state.library[0].id, "l23");
        assert_eq!(state.library[1].id, "l22");
        clean(&path);
    }

    #[test]
    fn concurrent_limit_is_bounded_from_one_to_ten() {
        let (backend, path) = backend();
        let mut state = backend.snapshot().clone();
        state.tasks = (0..12)
            .map(|i| task(&state, &format!("t{i}"), TaskStatus::Active))
            .collect();
        state.seeds.clear();
        state.settings.max_concurrent = 100;
        tick(&mut state, || 0.5).unwrap();
        assert_eq!(state.tasks.iter().filter(|item| !item.queued).count(), 10);
        state.settings.max_concurrent = 0;
        tick(&mut state, || 0.5).unwrap();
        assert_eq!(state.tasks.iter().filter(|item| !item.queued).count(), 1);
        clean(&path);
    }

    #[test]
    fn completion_and_failure_are_persisted_by_the_tick_command() {
        let (mut backend, path) = backend();
        let mut state = backend.snapshot().clone();
        let mut a = task(&state, "done", TaskStatus::Active);
        let mut b = task(&state, "bad", TaskStatus::Active);
        a.progress = 99.99;
        a.source = "https://example.org/actual.iso".into();
        b.progress = 10.0;
        b.fail_at = Some(10.0);
        state.tasks = vec![a, b];
        state.seeds.clear();
        backend.dispatch(Command::ReplaceState(state)).unwrap();
        let before = backend.snapshot().clone();
        backend.dispatch(Command::Tick { seed: 101 }).unwrap();
        let after = backend.snapshot();
        assert_eq!(
            notifications(&before, after),
            vec!["Completed: done.iso", "Failed: bad.iso"]
        );
        let restored = FakeBackend::open(path.clone()).unwrap();
        assert_eq!(restored.snapshot().tasks.len(), 1);
        assert!(matches!(
            restored.snapshot().tasks[0].status,
            TaskStatus::Failed
        ));
        assert_eq!(
            restored.snapshot().library[0].source,
            "https://example.org/actual.iso"
        );
        clean(&path);
    }

    #[test]
    fn backend_tick_is_atomic_persistent_and_rejects_invalid_data() {
        let (mut backend, path) = backend();
        let start = backend.snapshot().tasks[0].progress;
        backend.dispatch(Command::Tick { seed: 1 }).unwrap();
        let progress = backend.snapshot().tasks[0].progress;
        assert!(progress > start);
        let restored = FakeBackend::open(path.clone()).unwrap();
        near(restored.snapshot().tasks[0].progress, progress);
        let mut state = backend.snapshot().clone();
        state.tasks[0].size_mb = 0.0;
        backend
            .dispatch(Command::ReplaceState(state.clone()))
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(backend.dispatch(Command::Tick { seed: 2 }).is_err());
        assert_eq!(backend.snapshot(), &state);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // A failed disk save must also leave the published snapshot unchanged.
        let mut fixed = state;
        fixed.tasks[0].size_mb = 1000.0;
        backend
            .dispatch(Command::ReplaceState(fixed.clone()))
            .unwrap();
        let dir = path.parent().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::write(dir, "not a directory").unwrap();
        assert!(backend.dispatch(Command::Tick { seed: 3 }).is_err());
        assert_eq!(backend.snapshot(), &fixed);
        std::fs::remove_file(dir).unwrap();
    }
}

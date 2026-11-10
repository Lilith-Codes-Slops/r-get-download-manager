//! Apply one ordered progress-stream event to the GUI's task cache.
use downloader_ipc::{DaemonEvent, DownloadInfo};

pub(super) fn apply(tasks: &mut Vec<DownloadInfo>, event: DaemonEvent) {
    match event {
        DaemonEvent::ProgressReset => tasks.clear(),
        DaemonEvent::ProgressUpdate { task_id, task } if task_id == task.url => {
            if let Some(existing) = tasks.iter_mut().find(|existing| existing.url == task.url) {
                *existing = *task;
            } else {
                tasks.push(*task);
            }
        }
        DaemonEvent::ProgressRemoved { task_id } => tasks.retain(|task| task.url != task_id),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(downloaded: u64, generation: &str) -> DownloadInfo {
        serde_json::from_str(&format!(r#"{{"url":"task","state":"downloading","filename":"file","downloaded":{downloaded},"total":12,"speed_bps":1,"final_path":null,"partial_name":"{generation}"}}"#)).unwrap()
    }
    #[test]
    fn reset_update_retry_and_remove_replace_the_task_view() {
        let mut tasks = vec![task(9, "old")];
        apply(&mut tasks, DaemonEvent::ProgressReset);
        assert!(tasks.is_empty());
        apply(
            &mut tasks,
            DaemonEvent::ProgressUpdate {
                task_id: "task".into(),
                task: Box::new(task(3, "new")),
            },
        );
        apply(
            &mut tasks,
            DaemonEvent::ProgressUpdate {
                task_id: "task".into(),
                task: Box::new(task(7, "new")),
            },
        );
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].downloaded, 7);
        assert_eq!(tasks[0].partial_name.as_deref(), Some("new"));
        apply(
            &mut tasks,
            DaemonEvent::ProgressRemoved {
                task_id: "task".into(),
            },
        );
        assert!(tasks.is_empty());
    }
}

//! Wait for fair download capacity without holding queue or task locks.
use super::control::{Tasks, TransferControl};
use crate::protocols::torrent::concurrency_queue::Ticket;
use std::time::Duration;
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) async fn wait_for_slot(
    id: &str,
    tasks: &Tasks,
    ticket: &Ticket,
    control: &mut watch::Receiver<TransferControl>,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    let acquire = ticket.acquire();
    tokio::pin!(acquire);
    let mut refresh = tokio::time::interval(Duration::from_millis(250));
    loop {
        if *control.borrow() == TransferControl::Canceled {
            return Err("torrent canceled".into());
        }
        tokio::select! {
            biased;
            _ = control.changed() => return Err("torrent canceled".into()),
            permit = &mut acquire => {
                let permit = permit?;
                let mut guard = tasks.lock().await;
                if *control.borrow() == TransferControl::Canceled { return Err("torrent canceled".into()); }
                let task = guard.get_mut(id).ok_or("torrent task removed")?;
                task.state = "metadata".into();
                task.connection_note = Some("Torrent: pause, resume and speed limits are not available".into());
                drop(guard); super::progress_events::notify();
                return Ok(permit);
            },
            _ = refresh.tick() => {
                if let Some(position) = ticket.position() {
                    let mut guard = tasks.lock().await;
                    let mut changed = false;
                    if let Some(task) = guard.get_mut(id) {
                        let note = format!("Queued for download: position {position}");
                        if task.state == "queued" && task.connection_note.as_deref() != Some(note.as_str()) {
                            task.connection_note = Some(note); changed = true;
                        }
                    }
                    drop(guard); if changed { super::progress_events::notify(); }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::torrent::concurrency_queue::ConcurrencyQueue;
    use std::{collections::BTreeMap, sync::Arc};
    use tokio::sync::Mutex;

    fn task(id: &str) -> downloader_ipc::DownloadInfo {
        serde_json::from_str(&format!(r#"{{"url":"{id}","state":"queued","filename":null,"downloaded":0,"total":null,"speed_bps":0,"final_path":null}}"#)).unwrap()
    }

    #[tokio::test]
    async fn waiting_task_can_be_canceled_and_next_task_is_promoted() {
        let queue = ConcurrencyQueue::new(1);
        let first = queue.register("first".into());
        let active = first.acquire().await.unwrap();
        let waiting = queue.register("second".into());
        let tasks: Tasks = Arc::new(Mutex::new(BTreeMap::from([(
            "second".into(),
            task("second"),
        )])));
        let (sender, mut receiver) = watch::channel(TransferControl::Running);
        {
            let future = wait_for_slot("second", &tasks, &waiting, &mut receiver);
            tokio::pin!(future);
            assert!(tokio::time::timeout(Duration::from_millis(30), &mut future)
                .await
                .is_err());
            assert_eq!(tasks.lock().await["second"].state, "queued");
            sender.send_replace(TransferControl::Canceled);
            assert!(future.await.is_err());
        }
        drop(waiting);
        let next = queue.register("third".into());
        tasks.lock().await.insert("third".into(), task("third"));
        let (_sender, mut receiver) = watch::channel(TransferControl::Running);
        let next_future = wait_for_slot("third", &tasks, &next, &mut receiver);
        tokio::pin!(next_future);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut next_future)
                .await
                .is_err()
        );
        drop(active);
        let _permit = next_future.await.unwrap();
        assert_eq!(tasks.lock().await["third"].state, "metadata");
    }
}

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};

pub struct ScheduledTimer;

impl ScheduledTimer {
    pub async fn wait_until(not_before: i64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if not_before > now {
            sleep(Duration::from_secs((not_before - now) as u64)).await;
        }
    }
}

//! Optional bounded host policy memory. It never changes checkpoint identity.
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STABLE: Duration = Duration::from_secs(12);
const FRESH: Duration = Duration::from_secs(5);
const MAX_HOSTS: i64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bucket {
    Low,
    Medium,
    High,
}
impl Bucket {
    fn from_rtt(value: f64) -> Option<Self> {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        Some(if value < 50.0 {
            Self::Low
        } else if value < 200.0 {
            Self::Medium
        } else {
            Self::High
        })
    }
    pub fn representative_ms(self) -> f64 {
        match self {
            Self::Low => 25.0,
            Self::Medium => 100.0,
            Self::High => 250.0,
        }
    }
    fn code(self) -> i64 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }
    fn parse(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Low),
            1 => Some(Self::Medium),
            2 => Some(Self::High),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Remembered {
    pub target: u64,
    pub bucket: Bucket,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub memory: Remembered,
    pub observed_ms: i64,
}

#[derive(Default)]
pub(super) struct Stability {
    since: Option<Instant>,
    last: Option<Instant>,
    last_ms: Option<i64>,
    measured_rtt: bool,
    goodput: bool,
}
impl Stability {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn sample(&mut self, goodput: Option<f64>, rtt: Option<f64>, slowdown: bool) {
        if slowdown {
            self.reset();
            return;
        }
        if let Some(rate) = goodput {
            if !rate.is_finite() || rate <= 0.0 {
                self.reset();
                return;
            }
            self.goodput = true;
        }
        if rtt.is_some_and(|value| Bucket::from_rtt(value).is_some()) {
            self.measured_rtt = true;
        }
    }
    pub fn success(&mut self, now: Instant) {
        if !self.measured_rtt {
            return;
        }
        self.since.get_or_insert(now);
        self.last = Some(now);
        self.last_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok());
    }
    pub fn observation(&self, now: Instant, target: u64, rtt: Option<f64>) -> Option<Observation> {
        if !self.goodput
            || !self.measured_rtt
            || now.checked_duration_since(self.since?)? < STABLE
            || now.checked_duration_since(self.last?)? > FRESH
        {
            return None;
        }
        Some(Observation {
            memory: Remembered {
                target,
                bucket: Bucket::from_rtt(rtt?)?,
            },
            observed_ms: self.last_ms?,
        })
    }
}

/// Normalize the final redirect origin, including scheme and effective port.
pub(super) fn origin(url: &reqwest::Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let key = format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str()?.to_ascii_lowercase(),
        url.port_or_known_default()?
    );
    (key.len() <= 1024).then_some(key)
}
fn open(path: PathBuf) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(250))?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS rget_host_sizing_v1 (origin TEXT PRIMARY KEY NOT NULL, target INTEGER NOT NULL, bucket INTEGER NOT NULL, observed_ms INTEGER NOT NULL)")?;
    Ok(conn)
}
fn wall_ms() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}
fn valid(target: i64, bucket: i64, observed_ms: i64) -> Option<Remembered> {
    let target = u64::try_from(target).ok()?;
    if !(super::adaptive_sizer::MIN_TARGET..=super::adaptive_sizer::MAX_TARGET).contains(&target)
        || observed_ms <= 0
        || observed_ms > wall_ms()?.saturating_add(300_000)
    {
        return None;
    }
    Some(Remembered {
        target,
        bucket: Bucket::parse(bucket)?,
    })
}
pub(super) async fn load(path: PathBuf, key: String) -> Option<Remembered> {
    let work = tokio::task::spawn_blocking(move || -> rusqlite::Result<Option<Remembered>> {
        let conn = open(path)?;
        let row = conn
            .query_row(
                "SELECT target, bucket, observed_ms FROM rget_host_sizing_v1 WHERE origin = ?1",
                [&key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(target, bucket, time)| valid(target, bucket, time)))
    });
    match tokio::time::timeout(Duration::from_millis(500), work).await {
        Ok(Ok(Ok(value))) => value,
        _ => {
            tracing::warn!("could not load optional host sizing history");
            None
        }
    }
}
pub(super) async fn save(path: PathBuf, key: String, value: Observation) {
    let work = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
        let mut conn = open(path)?;
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO rget_host_sizing_v1(origin,target,bucket,observed_ms) VALUES (?1,?2,?3,?4) ON CONFLICT(origin) DO UPDATE SET target=excluded.target, bucket=excluded.bucket, observed_ms=excluded.observed_ms WHERE excluded.observed_ms > rget_host_sizing_v1.observed_ms OR rget_host_sizing_v1.observed_ms <= 0 OR rget_host_sizing_v1.observed_ms > excluded.observed_ms + 300000 OR rget_host_sizing_v1.target < 1048576 OR rget_host_sizing_v1.target > 134217728 OR rget_host_sizing_v1.bucket NOT IN (0,1,2)",
            params![key, value.memory.target as i64, value.memory.bucket.code(), value.observed_ms])?;
        tx.execute("DELETE FROM rget_host_sizing_v1 WHERE origin IN (SELECT origin FROM rget_host_sizing_v1 ORDER BY observed_ms DESC, origin ASC LIMIT -1 OFFSET ?1)", [MAX_HOSTS])?;
        tx.commit()
    });
    if !matches!(
        tokio::time::timeout(Duration::from_millis(750), work).await,
        Ok(Ok(Ok(())))
    ) {
        tracing::warn!("could not save optional host sizing history");
    }
}

/// At most one write per task at a time, and at most once every 15 seconds.
/// Database I/O never holds a segment-map lock or fails the transfer.
pub(super) struct Writer {
    path: PathBuf,
    key: Option<String>,
    pending: Option<tokio::task::JoinHandle<()>>,
    last_write: Option<Instant>,
    last_observed: i64,
}
impl Writer {
    pub fn new(path: PathBuf, key: Option<String>) -> Self {
        Self {
            path,
            key,
            pending: None,
            last_write: None,
            last_observed: 0,
        }
    }
    pub fn consider(&mut self, observation: Option<Observation>, now: Instant) {
        let (Some(key), Some(value)) = (self.key.as_ref(), observation) else {
            return;
        };
        if value.observed_ms <= self.last_observed
            || self.pending.as_ref().is_some_and(|job| !job.is_finished())
            || self
                .last_write
                .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(15))
        {
            return;
        }
        self.last_write = Some(now);
        self.last_observed = value.observed_ms;
        let path = self.path.clone();
        let key = key.clone();
        self.pending = Some(tokio::spawn(async move {
            save(path, key, value).await;
        }));
    }
    pub async fn finish(&mut self) {
        if let Some(job) = self.pending.take() {
            let _ = tokio::time::timeout(Duration::from_millis(800), job).await;
        }
    }
}

#[cfg(test)]
#[path = "host_memory_tests.rs"]
mod tests;

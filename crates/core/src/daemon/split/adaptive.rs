//! Conservative AIMD supervision for bounded, ownership-safe split workers.
use super::{segments::SegmentMap, worker};
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use std::{collections::BTreeMap, sync::Arc, time::{Duration, Instant}};
use tokio::{sync::{watch, Mutex}, task::JoinSet};

const INTERVAL: Duration = Duration::from_secs(2);
const COOLDOWN: Duration = Duration::from_secs(8);
const MIN_SAMPLE: u64 = 64 * 1024;
const MAX_SINGLE_REJECTIONS: u8 = 6;

struct Aimd { target: usize, max: usize, previous: u64, cooldown_until: Instant }
impl Aimd {
    fn new(max: usize) -> Self {
        Self { target: max.min(2), max, previous: 0, cooldown_until: Instant::now() }
    }
    fn reject(&mut self, now: Instant) {
        self.target = (self.target / 2).max(1);
        self.previous = 0;
        self.cooldown_until = now + COOLDOWN;
    }
    fn sample(&mut self, bytes: u64, now: Instant) -> bool {
        if now < self.cooldown_until { self.previous = 0; return false; }
        if self.previous >= MIN_SAMPLE && bytes < self.previous / 2 {
            self.reject(now);
        } else if bytes >= MIN_SAMPLE && (self.previous == 0 || bytes >= self.previous * 9 / 10) && self.target < self.max {
            self.target += 1;
            self.previous = bytes;
            return true;
        }
        self.previous = bytes;
        false
    }
}

pub(super) struct Inputs {
    pub client: reqwest::Client, pub url: String, pub info: ResumeInfo,
    pub file: Arc<std::fs::File>, pub map: Arc<Mutex<SegmentMap>>,
    pub limiter: Arc<BandwidthLimiter>, pub stop: watch::Sender<bool>, pub max: usize,
    pub desired: Option<watch::Receiver<u8>>,
    #[cfg(test)]
    pub peak_workers: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

struct WorkerSignals { retire: watch::Sender<bool>, drain: watch::Sender<bool> }
fn spawn_worker(inputs: &Inputs, workers: &mut JoinSet<(usize, Result<worker::Exit, String>)>,
    running: &mut BTreeMap<usize, WorkerSignals>, owner: usize) {
    let (retire, receiver) = watch::channel(false);
    let (drain, drain_rx) = watch::channel(false);
    let context = worker::Context { client: inputs.client.clone(), url: inputs.url.clone(), info: inputs.info.clone(),
        file: inputs.file.clone(), map: inputs.map.clone(), limiter: inputs.limiter.clone(),
        stop: inputs.stop.subscribe(), retire: receiver, drain: drain_rx, desired: inputs.desired.clone(), owner };
    running.insert(owner, WorkerSignals { retire, drain });
    #[cfg(test)]
    if let Some(peak) = &inputs.peak_workers {
        peak.fetch_max(running.len(), std::sync::atomic::Ordering::SeqCst);
    }
    workers.spawn(async move { (owner, worker::run(context).await) });
}

pub(super) async fn supervise(mut inputs: Inputs) -> Result<(), String> {
    let mut aimd = Aimd::new(inputs.max);
    let mut workers = JoinSet::new();
    let mut running: BTreeMap<usize, WorkerSignals> = BTreeMap::new();
    let mut owner = 0usize;
    let mut first_error: Option<String> = None;
    let mut rejections = 0u8;
    let mut retry_at = Instant::now();
    let mut timer = tokio::time::interval(INTERVAL);
    timer.tick().await;
    let mut previous_bytes: u64 = inputs.map.lock().await.records().iter().map(|s| s.downloaded).sum();
    let mut try_steal = false;
    loop {
        if *inputs.stop.borrow() && first_error.is_none() { first_error = Some("split stopped".into()); }
        if first_error.is_none() {
            let map = inputs.map.lock().await;
            let pending = map.pending_count();
            let stealable = map.stealable_slots();
            let complete = map.is_complete();
            drop(map);
            if complete && running.is_empty() { return Ok(()); }
            // Pending work must always be retried, even when the target fell to
            // one. An increase may also start workers solely to steal.
            if Instant::now() >= retry_at {
                let slots = aimd.target.saturating_sub(running.len());
                let count = slots.min(pending + if try_steal { stealable } else { 0 });
                for _ in 0..count {
                    spawn_worker(&inputs, &mut workers, &mut running, owner);
                    owner += 1;
                }
            }
            try_steal = false;
            let active = running.values().filter(|signal| !*signal.retire.borrow() && !*signal.drain.borrow()).count();
            for signal in running.values().rev().filter(|signal| !*signal.retire.borrow() && !*signal.drain.borrow()).take(active.saturating_sub(aimd.target)) {
                signal.retire.send_replace(true);
            }
        } else {
            inputs.stop.send_replace(true);
            if workers.is_empty() { return Err(first_error.unwrap()); }
        }
        tokio::select! {
            result = workers.join_next(), if !workers.is_empty() => {
                match result {
                    Some(Ok((id, result))) => {
                        running.remove(&id);
                        if running.len() < aimd.target { try_steal = true; }
                        match result {
                            Ok(worker::Exit::Rejected) => {
                                aimd.reject(Instant::now());
                                if running.is_empty() {
                                    rejections += 1;
                                    if rejections >= MAX_SINGLE_REJECTIONS { first_error = Some("server repeatedly rejected split requests (429/503)".into()); }
                                    retry_at = Instant::now() + INTERVAL;
                                }
                            },
                            Ok(_) => {},
                            Err(error) => {
                                if first_error.is_none() || first_error.as_deref() == Some("split stopped") { first_error = Some(error); }
                            }
                        }
                    },
                    Some(Err(error)) => { first_error = Some(format!("split worker failed: {error}")); inputs.stop.send_replace(true); },
                    None => { first_error = Some("split supervisor lost workers".into()); }
                }
            },
            changed = async {
                match &mut inputs.desired { Some(rx) => rx.changed().await.is_ok(), None => std::future::pending::<bool>().await }
            } => {
                if changed {
                    let count = inputs.desired.as_mut().map(|rx| *rx.borrow_and_update()).unwrap_or(1) as usize;
                    let new_max = count.clamp(1, 32).min((inputs.info.total / (1024 * 1024)).max(1).min(32) as usize);
                    let old_max = aimd.max;
                    aimd.max = new_max;
                    if new_max > old_max {
                        aimd.target = new_max;
                        let active = running.values().filter(|signal| !*signal.retire.borrow() && !*signal.drain.borrow()).count();
                        for signal in running.values().filter(|signal| *signal.drain.borrow() && !*signal.retire.borrow()).take(new_max.saturating_sub(active)) {
                            signal.drain.send_replace(false);
                        }
                        try_steal = true;
                    } else if new_max < old_max {
                        aimd.target = aimd.target.min(new_max);
                        let active = running.values().filter(|signal| !*signal.retire.borrow() && !*signal.drain.borrow()).count();
                        for signal in running.values().rev().filter(|signal| !*signal.retire.borrow() && !*signal.drain.borrow()).take(active.saturating_sub(aimd.target)) {
                            signal.drain.send_replace(true);
                        }
                    }
                } else { inputs.desired = None; }
            },
            _ = timer.tick() => {
                let bytes: u64 = inputs.map.lock().await.records().iter().map(|s| s.downloaded).sum();
                let delta = bytes.saturating_sub(previous_bytes);
                previous_bytes = bytes;
                if delta > 0 { rejections = 0; }
                if aimd.sample(delta, Instant::now()) { try_steal = true; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn increases_with_goodput_and_halves_on_slowdown_or_rejection() {
        let mut aimd = Aimd::new(4);
        let now = Instant::now();
        assert_eq!(aimd.target, 2);
        assert!(aimd.sample(MIN_SAMPLE * 4, now));
        assert_eq!(aimd.target, 3);
        assert!(!aimd.sample(MIN_SAMPLE, now));
        assert_eq!(aimd.target, 1);
        assert!(!aimd.sample(MIN_SAMPLE * 4, now + Duration::from_secs(1)));
        assert!(aimd.sample(MIN_SAMPLE * 4, now + COOLDOWN));
        aimd.reject(now + COOLDOWN);
        assert_eq!(aimd.target, 1);
        for n in 1..10 { aimd.sample(MIN_SAMPLE * 4, now + COOLDOWN * (n + 1)); }
        assert_eq!(aimd.target, 4);
    }
}

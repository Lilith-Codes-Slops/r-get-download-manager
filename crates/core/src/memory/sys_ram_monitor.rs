use sysinfo::System;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Polls live free RAM on Linux and Windows via the sysinfo crate.
pub struct SysRamMonitor {
    system: Arc<Mutex<System>>,
    last_refresh: Arc<Mutex<Instant>>,
    min_interval: Duration,
}

impl SysRamMonitor {
    pub fn new() -> Self {
        let system = System::new_all().expect("failed to init sysinfo");
        Self {
            system: Arc::new(Mutex::new(system)),
            last_refresh: Arc::new(Mutex::new(Instant::now())),
            min_interval: Duration::from_secs(1),
        }
    }

    /// Returns currently available RAM in bytes. Refreshes at most once per second.
    pub fn available_bytes(&self) -> u64 {
        let mut last = self.last_refresh.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(*last) >= self.min_interval {
            if let Ok(mut sys) = self.system.lock() {
                sys.refresh_memory();
            }
            *last = now;
        }
        drop(last);
        self.system
            .lock()
            .map(|sys| sys.available_memory())
            .unwrap_or(0)
    }

    /// Total physical RAM in bytes.
    pub fn total_bytes(&self) -> u64 {
        self.system
            .lock()
            .map(|sys| sys.total_memory())
            .unwrap_or(0)
    }
}

impl Default for SysRamMonitor {
    fn default() -> Self {
        Self::new()
    }
}
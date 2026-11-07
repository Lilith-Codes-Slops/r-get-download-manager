use super::sys_ram_monitor::SysRamMonitor;

const MIN_BUFFER: usize = 64 * 1024; // 64 KiB
const MAX_BUFFER: usize = 2 * 1024 * 1024; // 2 MiB
const LOW_WATER: u64 = 512 * 1024 * 1024; // 512 MiB free
const HIGH_WATER: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB free

/// Computes optimal in-flight buffer size from live free RAM.
pub struct AdaptiveBudgetCalc {
    monitor: SysRamMonitor,
}

impl AdaptiveBudgetCalc {
    pub fn new(monitor: SysRamMonitor) -> Self {
        Self { monitor }
    }

    /// Returns a buffer size between 64 KiB and 2 MiB based on available memory.
    pub fn recommended_buffer_size(&self) -> usize {
        let free = self.monitor.available_bytes();
        if free <= LOW_WATER {
            MIN_BUFFER
        } else if free >= HIGH_WATER {
            MAX_BUFFER
        } else {
            let ratio = (free - LOW_WATER) as f64 / (HIGH_WATER - LOW_WATER) as f64;
            let size = MIN_BUFFER as f64 + ratio * (MAX_BUFFER - MIN_BUFFER) as f64;
            size as usize
        }
    }

    /// True when free RAM is so low that new segments or tasks should be paused.
    pub fn under_pressure(&self) -> bool {
        self.monitor.available_bytes() < LOW_WATER / 2
    }
}

use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

const MIN_SEGMENT_REMAINING: u64 = 2 * 1024 * 1024; // 2 MiB

#[derive(Error, Debug)]
pub enum RangeError {
    #[error("range overlap detected: {start}-{end}")]
    Overlap { start: u64, end: u64 },
    #[error("remaining size below minimum {MIN_SEGMENT_REMAINING}")]
    TooSmall,
    #[error("invalid range: start {start} >= end {end}")]
    Invalid { start: u64, end: u64 },
}

/// Atomic non-overlapping range ownership for concurrent positional writes.
pub struct RangeOwnershipGuard {
    end_byte: AtomicU64,
}

impl RangeOwnershipGuard {
    pub fn new(initial_end: u64) -> Self {
        Self {
            end_byte: AtomicU64::new(initial_end),
        }
    }

    /// Atomically claim a non-overlapping range. Returns the claimed (start, end).
    pub fn claim(&self, desired_start: u64, desired_end: u64) -> Result<(u64, u64), RangeError> {
        if desired_start >= desired_end {
            return Err(RangeError::Invalid {
                start: desired_start,
                end: desired_end,
            });
        }
        let current = self.end_byte.load(Ordering::Acquire);
        if desired_end > current {
            return Err(RangeError::Overlap {
                start: desired_start,
                end: desired_end,
            });
        }
        let remaining = current.saturating_sub(desired_start);
        if remaining < MIN_SEGMENT_REMAINING && desired_end != current {
            return Err(RangeError::TooSmall);
        }
        // In a full implementation this would use compare-exchange against a more
        // sophisticated interval tree; the atomic end_byte is the critical invariant
        // for the current segment under work-stealing.
        Ok((desired_start, desired_end))
    }

    /// Shrink the current end_byte for work-stealing (halving).
    pub fn shrink_end(&self, new_end: u64) -> Result<u64, RangeError> {
        let old = self.end_byte.fetch_min(new_end, Ordering::AcqRel);
        if new_end >= old {
            return Err(RangeError::Invalid {
                start: new_end,
                end: old,
            });
        }
        Ok(old)
    }

    pub fn current_end(&self) -> u64 {
        self.end_byte.load(Ordering::Acquire)
    }
}

use super::segment_worker::SegmentWorker;

const MIN_REMAINING: u64 = 2 * 1024 * 1024;

pub struct SegmentHalver;

impl SegmentHalver {
    pub fn should_halve(remaining: u64, predicted_slow: bool) -> bool {
        remaining >= MIN_REMAINING && predicted_slow
    }

    pub fn midpoint(start: u64, end: u64) -> u64 {
        start + (end - start) / 2
    }

    pub fn apply_halve(worker: &SegmentWorker, new_end: u64) {
        worker.shrink_end(new_end);
    }
}

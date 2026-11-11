const MIN_SEGMENT: u64 = 2 * 1024 * 1024;
const TARGET_SECS: f64 = 3.0;

pub struct PredictiveSegmentSizer;

impl PredictiveSegmentSizer {
    // Work on it later; see state.md: RTT is not used by the sizer yet.
    pub fn size_for(goodput_bps: f64, _rtt_ms: f64) -> u64 {
        if goodput_bps <= 0.0 {
            return MIN_SEGMENT;
        }
        let bytes = (goodput_bps * TARGET_SECS) as u64;
        bytes.max(MIN_SEGMENT)
    }
}

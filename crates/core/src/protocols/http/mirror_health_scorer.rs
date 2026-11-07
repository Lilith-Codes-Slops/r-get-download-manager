pub struct MirrorHealthScorer;

impl MirrorHealthScorer {
    pub fn penalize(status: u16) -> bool {
        matches!(status, 429 | 500..=599)
    }

    pub fn score(error_rate: f32, goodput: f64) -> f64 {
        if error_rate > 0.5 {
            0.0
        } else {
            goodput * (1.0 - error_rate as f64)
        }
    }
}

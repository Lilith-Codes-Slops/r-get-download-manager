use super::transfer_performance_model::TransferPerformanceModel;

pub struct MirrorTtfbRacer;

impl MirrorTtfbRacer {
    pub fn rank(model: &TransferPerformanceModel, mirrors: &[&str], size: u64) -> Vec<String> {
        let mut scored: Vec<_> = mirrors
            .iter()
            .filter_map(|id| {
                model
                    .predicted_completion(id, size)
                    .map(|t| (id.to_string(), t))
            })
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(id, _)| id).collect()
    }
}

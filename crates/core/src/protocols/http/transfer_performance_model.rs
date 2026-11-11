use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct MirrorStats {
    pub rtt_ms: f64,
    pub goodput_ema: f64,
    pub error_rate: f32,
    pub active: u32,
}

pub struct TransferPerformanceModel {
    mirrors: RwLock<HashMap<String, MirrorStats>>,
}

impl TransferPerformanceModel {
    pub fn new() -> Self {
        Self {
            mirrors: RwLock::new(HashMap::new()),
        }
    }

    pub fn update(&self, mirror_id: &str, rtt: f64, goodput: f64, err: f32) {
        let mut map = self.mirrors.write().unwrap();
        let entry = map.entry(mirror_id.to_string()).or_default();
        entry.rtt_ms = rtt;
        entry.goodput_ema = entry.goodput_ema * 0.7 + goodput * 0.3;
        entry.error_rate = err;
    }

    pub fn predicted_completion(&self, mirror_id: &str, size: u64) -> Option<f64> {
        let map = self.mirrors.read().unwrap();
        map.get(mirror_id).map(|s| {
            if s.goodput_ema > 0.0 {
                s.rtt_ms / 1000.0 + size as f64 / s.goodput_ema
            } else {
                s.rtt_ms / 1000.0
            }
        })
    }
}

impl Default for TransferPerformanceModel {
    fn default() -> Self {
        Self::new()
    }
}

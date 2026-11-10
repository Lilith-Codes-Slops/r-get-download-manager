pub struct CongestionController {
    base_connections: usize,
}

impl CongestionController {
    pub fn new(base: usize) -> Self {
        Self {
            base_connections: base,
        }
    }

    pub fn adjust(&self, all_slow: bool, high_error: bool) -> usize {
        if all_slow || high_error {
            (self.base_connections / 2).max(1)
        } else {
            self.base_connections
        }
    }
}

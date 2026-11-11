use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct SegmentWorker {
    // Work on it later; see state.md: the client is reserved for future requests.
    _client: Client,
    end_byte: AtomicU64,
}

impl SegmentWorker {
    pub fn new(client: Client, initial_end: u64) -> Self {
        Self {
            _client: client,
            end_byte: AtomicU64::new(initial_end),
        }
    }

    pub fn current_end(&self) -> u64 {
        self.end_byte.load(Ordering::Acquire)
    }

    pub fn shrink_end(&self, new_end: u64) {
        self.end_byte.fetch_min(new_end, Ordering::AcqRel);
    }
}

pub struct TailDuplicator;

impl TailDuplicator {
    pub fn should_duplicate(remaining: u64) -> bool {
        remaining > 0 && remaining <= 4 * 1024 * 1024
    }
}

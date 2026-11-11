pub struct TrayService;

impl TrayService {
    pub fn new() -> Self {
        Self
    }

    pub async fn run(&self) {
        // Platform tray integration would live here.
    }
}

impl Default for TrayService {
    fn default() -> Self {
        Self::new()
    }
}

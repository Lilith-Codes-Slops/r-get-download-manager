pub struct LiveProgressBars;

impl LiveProgressBars {
    pub fn render(task_id: &str, progress: f64, speed: u64) {
        println!("{task_id}: {:.1}% @ {} B/s", progress * 100.0, speed);
    }
}

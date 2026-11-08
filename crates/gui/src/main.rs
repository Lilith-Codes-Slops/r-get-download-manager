#![forbid(unsafe_code)]

mod app_state;
mod ipc_bridge_worker;
mod progress_stream;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([800.0, 580.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rGet",
        options,
        Box::new(|cc| Ok(Box::new(app_state::AppState::new(cc)))),
    )
}

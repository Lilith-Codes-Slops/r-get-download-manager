#![forbid(unsafe_code)]

use eframe::egui;

mod app_state;
mod dialogs;
mod ipc_bridge_worker;
mod theme;
mod views;
mod widgets;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "rget",
        options,
        Box::new(|_cc| Ok(Box::new(app_state::AppState::default()))),
    )
}
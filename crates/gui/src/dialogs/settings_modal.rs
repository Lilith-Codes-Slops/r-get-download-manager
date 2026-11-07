use eframe::egui;

pub struct SettingsModal;

impl SettingsModal {
    pub fn show(ctx: &egui::Context, open: &mut bool) {
        egui::Window::new("Settings").open(open).show(ctx, |ui| {
            ui.label("Settings");
        });
    }
}

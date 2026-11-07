use eframe::egui;

pub struct RefreshUrlModal;

impl RefreshUrlModal {
    pub fn show(ctx: &egui::Context, open: &mut bool) {
        egui::Window::new("Refresh URL").open(open).show(ctx, |ui| {
            ui.label("New URL");
        });
    }
}

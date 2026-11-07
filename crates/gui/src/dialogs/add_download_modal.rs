use eframe::egui;

pub struct AddDownloadModal;

impl AddDownloadModal {
    pub fn show(ctx: &egui::Context, open: &mut bool) {
        egui::Window::new("Add Download").open(open).show(ctx, |ui| {
            ui.label("URL");
        });
    }
}

use eframe::egui;

pub struct StreamQualityModal;

impl StreamQualityModal {
    pub fn show(ctx: &egui::Context, open: &mut bool) {
        egui::Window::new("Select Quality").open(open).show(ctx, |ui| {
            ui.label("Quality");
        });
    }
}

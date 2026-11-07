use eframe::egui;

pub struct LightTheme;

impl LightTheme {
    pub fn apply(ctx: &egui::Context) {
        ctx.set_visuals(egui::Visuals::light());
    }
}

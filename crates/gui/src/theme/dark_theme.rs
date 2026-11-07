use eframe::egui;

pub struct DarkTheme;

impl DarkTheme {
    pub fn apply(ctx: &egui::Context) {
        ctx.set_visuals(egui::Visuals::dark());
    }
}

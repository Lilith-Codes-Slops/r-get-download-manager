use eframe::egui;

pub struct MirrorHealthTable;

impl MirrorHealthTable {
    pub fn show(ui: &mut egui::Ui) {
        ui.label("Mirrors");
    }
}

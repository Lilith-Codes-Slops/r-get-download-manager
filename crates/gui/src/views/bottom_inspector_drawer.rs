use eframe::egui;

pub struct BottomInspectorDrawer;

impl BottomInspectorDrawer {
    pub fn show(ui: &mut egui::Ui) {
        ui.label("Inspector");
    }
}

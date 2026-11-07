use eframe::egui;

pub struct CentralTaskTable;

impl CentralTaskTable {
    pub fn show(ui: &mut egui::Ui) {
        ui.label("No tasks");
    }
}

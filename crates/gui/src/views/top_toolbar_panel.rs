use eframe::egui;

pub struct TopToolbarPanel;

impl TopToolbarPanel {
    pub fn show(ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Add URL").clicked() {}
            if ui.button("Pause All").clicked() {}
        });
    }
}

use eframe::egui;

pub struct SidebarCategoryPanel;

impl SidebarCategoryPanel {
    pub fn show(ui: &mut egui::Ui) {
        ui.selectable_label(true, "All");
        ui.selectable_label(false, "Active");
        ui.selectable_label(false, "Completed");
    }
}

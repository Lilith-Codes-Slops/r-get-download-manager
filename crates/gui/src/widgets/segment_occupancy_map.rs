use eframe::egui;

pub struct SegmentOccupancyMap;

impl SegmentOccupancyMap {
    pub fn show(ui: &mut egui::Ui, segments: usize) {
        ui.label(format!("{segments} segments"));
    }
}

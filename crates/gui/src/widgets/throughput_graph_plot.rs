use eframe::egui;

pub struct ThroughputGraphPlot;

impl ThroughputGraphPlot {
    pub fn show(ui: &mut egui::Ui, points: &[f32]) {
        let _ = points;
        ui.label("Throughput");
    }
}

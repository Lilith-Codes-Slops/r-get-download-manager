use eframe::egui;

pub struct AppState {
    pub selected_task: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self { selected_task: None }
    }
}

impl eframe::App for AppState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("rget");
            ui.label("Downloads");
        });
    }
}
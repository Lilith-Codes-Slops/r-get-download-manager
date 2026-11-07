use super::*;

impl AppState {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(RichText::new("rGet").strong().color(Color32::from_rgb(99, 165, 255)));
            ui.add_space(16.0);
            if ui.button("+ Add URL").clicked() { self.add_open = true; }
            unavailable(ui, "Add file", "Torrent and Metalink are not implemented.");
            unavailable(ui, "Pause all", "Not yet supported.");
            unavailable(ui, "Resume all", "Not yet supported.");
            unavailable(ui, "Cancel all", "Not yet supported.");
            if ui.button("Settings").clicked() { self.settings_open = true; }
        });
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new("LIBRARY").small().weak());
        if ui.selectable_label(self.view == View::Tasks, format!("Tasks   {}", self.tasks.len())).clicked() { self.view = View::Tasks; }
        if ui.selectable_label(self.view == View::Downloads, "Downloads").clicked() { self.view = View::Downloads; self.refresh_files(); }
        ui.separator();
        ui.label(format!("Active     {}", self.tasks.iter().filter(|t| t.state == "downloading").count()));
        ui.label(format!("Completed  {}", self.tasks.iter().filter(|t| t.state == "completed").count()));
        let color = if self.connected { Color32::from_rgb(91, 193, 143) } else { Color32::from_rgb(232, 171, 92) };
        ui.label(RichText::new(if self.connected { "● Daemon connected" } else { "● Connecting / offline" }).color(color).small());
    }

    pub(super) fn tasks_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tasks");
        ui.label(RichText::new("Your download queue and live transfer details").weak());
        let speed: u64 = self.tasks.iter().map(|t| t.speed_bps).sum();
        ui.label(format!("{} downloads  ·  Download {}/s  ·  Upload not supported", self.tasks.len(), format_bytes(speed)));
        ui.separator();
        let height = (ui.available_height() * 0.46).max(110.0);
        egui::ScrollArea::vertical().id_salt("task_list").max_height(height).show(ui, |ui| {
            if self.tasks.is_empty() { ui.add_space(24.0); ui.label("No downloads yet. Add a URL to get started."); }
            for task in &self.tasks {
                let selected = self.selected_task.as_deref() == Some(task.url.as_str());
                let name = task.filename.as_deref().unwrap_or_else(|| task.url.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&task.url));
                ui.horizontal(|ui| {
                    if ui.selectable_label(selected, RichText::new(name).strong()).clicked() { self.selected_task = Some(task.url.clone()); }
                    ui.label(RichText::new(&task.state).weak());
                    ui.label(format!("{}/s", format_bytes(task.speed_bps)));
                });
                if let Some(total) = task.total.filter(|n| *n > 0) {
                    ui.add(egui::ProgressBar::new((task.downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32).show_percentage().desired_width(ui.available_width()));
                } else { ui.label(format!("{} downloaded · total unknown", format_bytes(task.downloaded))); }
                ui.separator();
            }
        });
        let selected = self.selected_task.as_ref().and_then(|id| self.tasks.iter().find(|t| &t.url == id)).cloned();
        if let Some(task) = selected { self.task_details(ui, &task); }
        else { ui.label(RichText::new("Select a task to view details").weak()); }
    }

    fn task_details(&mut self, ui: &mut egui::Ui, task: &DownloadInfo) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.detail, Detail::Status, "Download status");
            ui.selectable_value(&mut self.detail, Detail::Network, "Network & splits");
            ui.selectable_value(&mut self.detail, Detail::Actions, "Post-download actions");
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("details").show(ui, |ui| match self.detail {
            Detail::Status => {
                ui.label(RichText::new(&task.url).small());
                egui::Grid::new("details_grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    metric(ui, "Status", &task.state);
                    metric(ui, "Progress", &format!("{} / {}", format_bytes(task.downloaded), task.total.map(format_bytes).unwrap_or_else(|| "Unknown".into())));
                    metric(ui, "Download rate", &format!("{}/s (average)", format_bytes(task.speed_bps)));
                    let eta = task.total.and_then(|total| if task.speed_bps > 0 { Some(total.saturating_sub(task.downloaded) / task.speed_bps) } else { None });
                    metric(ui, "Time remaining", &eta.map(format_duration).unwrap_or_else(|| "Unknown".into()));
                    metric(ui, "Resume support", "Not available");
                    if let Some(path) = &task.final_path { metric(ui, "Saved to", path); }
                });
                ui.horizontal(|ui| {
                    unavailable(ui, "Pause", "Resume is not implemented by the daemon.");
                    unavailable(ui, "Cancel", "Cancellation is not implemented by the daemon.");
                    if task.state.starts_with("failed:") && ui.button("Retry").clicked() { self.send(add_request(task.url.clone())); }
                });
            }
            Detail::Network => {
                ui.heading("Network & splits");
                ui.label("The engine currently uses one connection per download.");
                unavailable(ui, "Configure speed limit", "Not yet supported in the GUI.");
                unavailable(ui, "Configure connections", "Segmented downloads are not implemented.");
            }
            Detail::Actions => {
                ui.heading("Post-download actions");
                ui.label("Automation is not available yet. No actions will run automatically.");
                for label in ["Open file location", "Close rGet", "Verify checksum", "Extract archive", "Run antivirus scan", "System power action"] {
                    ui.add_enabled(false, egui::Checkbox::new(&mut false, label));
                }
            }
        });
    }

    pub(super) fn downloads_view(&mut self, ui: &mut egui::Ui) {
        if self.last_files_refresh.elapsed() >= Duration::from_secs(3) { self.refresh_files(); }
        ui.heading("Downloads");
        ui.label(RichText::new(download_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "Download directory unavailable".into())).weak());
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh").clicked() { self.refresh_files(); }
            if ui.add_enabled(self.selected_file.is_some(), egui::Button::new("Open file")).clicked() {
                if let Some(path) = &self.selected_file { if let Err(e) = open_path(path) { self.notice = Some(e); } }
            }
            if ui.button("Open folder").clicked() {
                if let Some(path) = download_dir() { if let Err(e) = open_path(&path) { self.notice = Some(e); } }
            }
            if ui.add_enabled(self.selected_file.is_some(), egui::Button::new("Copy path")).clicked() {
                if let Some(path) = &self.selected_file { ui.ctx().copy_text(path.display().to_string()); }
            }
            unavailable(ui, "Rename / move / delete", "File modifications are not implemented.");
        });
        ui.horizontal(|ui| {
            ui.label("Sort by:");
            for (sort, label) in [(Sort::Name, "Name"), (Sort::Size, "Size"), (Sort::Date, "Modified"), (Sort::Type, "Type")] {
                if ui.selectable_label(self.sort == sort, label).clicked() { if self.sort == sort { self.descending = !self.descending; } else { self.sort = sort; self.descending = false; } self.sort_files(); }
            }
            ui.label(if self.descending { "Descending" } else { "Ascending" });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("files").show(ui, |ui| {
            if self.files.is_empty() { ui.label("No files in the Downloads folder."); }
            for file in &self.files {
                let name = file.path.file_name().unwrap_or_default().to_string_lossy();
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.selected_file.as_ref() == Some(&file.path), name.as_ref()).clicked() { self.selected_file = Some(file.path.clone()); }
                    ui.label(format_bytes(file.size));
                    ui.label(file.path.extension().unwrap_or_default().to_string_lossy().into_owned());
                });
                ui.separator();
            }
        });
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if self.add_open {
            let mut open = self.add_open;
            let mut submit = false;
            egui::Window::new("Add download").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label("Paste an HTTP or HTTPS URL");
                ui.add(egui::TextEdit::singleline(&mut self.url).hint_text("https://example.com/file.zip").desired_width(420.0));
                ui.label("Files are saved to Downloads. Existing files are not overwritten.");
                if ui.add_enabled(self.connected && !self.url.trim().is_empty(), egui::Button::new("Start download")).clicked() { submit = true; }
                if !self.connected { ui.label("Waiting for the daemon to connect."); }
            });
            if submit { let url = self.url.trim().to_string(); self.send(add_request(url)); self.url.clear(); open = false; self.view = View::Tasks; }
            self.add_open = open;
        }
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("Settings").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.heading("Appearance");
                let previous = self.theme;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.theme, Theme::System, "System");
                    ui.selectable_value(&mut self.theme, Theme::Light, "Light");
                    ui.selectable_value(&mut self.theme, Theme::Dark, "Dark");
                });
                if self.theme != previous {
                    ctx.set_theme(match self.theme {
                        Theme::System => egui::ThemePreference::System,
                        Theme::Light => egui::ThemePreference::Light,
                        Theme::Dark => egui::ThemePreference::Dark,
                    });
                }
                ui.separator();
                ui.heading("Download engine");
                ui.label("Connections: 1 · Segmented downloads not implemented");
                ui.label("Theme selection lasts for this session.");
            });
            self.settings_open = open;
        }
    }
}

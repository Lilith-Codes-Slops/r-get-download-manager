use super::palette;
use super::*;
use std::collections::HashMap;

type SegmentSummaries = HashMap<String, Option<(usize, u64, u64)>>;
type WorkerCounts = HashMap<String, Option<u8>>;

/// Render one task pane. Returns the id of a row the user selected.
fn render_task_rows(
    ui: &mut egui::Ui,
    tasks: &[DownloadInfo],
    summaries: &SegmentSummaries,
    workers: &WorkerCounts,
    selected: &Option<String>,
    filter: &str,
    empty_message: &str,
) -> Option<String> {
    let query = filter.trim().to_lowercase();
    let rows: Vec<&DownloadInfo> = tasks
        .iter()
        .filter(|task| query.is_empty() || task_name(task).to_lowercase().contains(query.as_str()))
        .collect();
    if rows.is_empty() {
        ui.label(empty_message);
        return None;
    }
    let mut clicked = None;
    for task in rows {
        let name = task_name(task);
        let is_selected = selected.as_deref() == Some(task.url.as_str());
        if ui
            .selectable_label(is_selected, RichText::new(name.as_str()).strong())
            .clicked()
        {
            clicked = Some(task.url.clone());
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(state_label(task)).weak());
            ui.label(format!("{}/s", format_bytes(task.speed_bps)));
            ui.label(format!(
                "Workers: {}",
                count(
                    workers
                        .get(&task.url)
                        .and_then(|value| *value)
                        .or(task.active_connections)
                )
            ));
        });
        if let Some(total) = task.total.filter(|total| *total > 0) {
            let downloaded = summaries
                .get(&task.url)
                .and_then(|value| *value)
                .map(|(_, bytes, _)| bytes)
                .unwrap_or(task.downloaded);
            ui.add(
                egui::ProgressBar::new((downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32)
                    .show_percentage()
                    .desired_width(ui.available_width()),
            );
        } else {
            ui.label(format!(
                "{} downloaded · total unknown",
                format_bytes(task.downloaded)
            ));
        }
        if let Some((segments, bytes, total)) = summaries.get(&task.url).and_then(|value| *value) {
            ui.label(format!(
                "{segments} segments · {:.1}%",
                bytes as f64 / total as f64 * 100.0
            ));
        }
        ui.separator();
    }
    clicked
}

impl AppState {
    pub(super) fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(
                RichText::new("↓ Downloads")
                    .strong()
                    .color(Color32::from_rgb(59, 130, 246)),
            );
            ui.add_space(16.0);
            if ui
                .selectable_label(self.view == View::Tasks, "Tasks")
                .clicked()
            {
                self.view = View::Tasks;
            }
            if ui
                .selectable_label(self.view == View::Downloads, "Downloads")
                .clicked()
            {
                self.view = View::Downloads;
                self.refresh_files();
            }
            ui.add_space(8.0);
            if ui.button("Help").clicked() {
                self.help_open = true;
            }
            if ui.button("Settings").clicked() {
                self.settings_open = true;
            }
            if ui
                .button("Commands")
                .on_hover_text("Open the command palette (Ctrl+K)")
                .clicked()
            {
                self.open_palette();
            }
        });
    }

    pub(super) fn action_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ URL").clicked() {
                self.add_open = true;
            }
            // Work on it later; TODO(state.md): magnet ingest is not supported by the daemon yet.
            unavailable(
                ui,
                "+ Magnet",
                "The daemon accepts HTTP/HTTPS URLs and local .torrent paths only.",
            );
            if ui.button("Pause All").clicked() {
                self.pause_all();
            }
            if ui.button("Resume All").clicked() {
                self.resume_all();
            }
            if ui.button("Remove Completed").clicked() {
                self.remove_completed();
            }
            if ui.button("··· More").clicked() {
                if self.selected_task.is_some() {
                    self.actions_open = true;
                } else {
                    self.notify("Select a download first");
                }
            }
            // Work on it later; TODO(state.md): native file picker is not wired yet.
            unavailable(
                ui,
                "Add file",
                "Use + URL and paste a local .torrent path; a native file picker is deferred.",
            );
        });
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        if ui
            .selectable_label(
                self.rail_pinned,
                if self.rail_pinned {
                    "📌 Pinned"
                } else {
                    "📌"
                },
            )
            .clicked()
        {
            self.rail_pinned = !self.rail_pinned;
        }
        ui.separator();
        let panels = [
            (Panel::Categories, "Categories", "C"),
            (Panel::Labels, "Labels", "L"),
            (Panel::Rss, "RSS Feeds", "R"),
            (Panel::Recent, "Recent", "T"),
            (Panel::Favorites, "Favorites", "F"),
        ];
        for (panel, label, short) in panels {
            let text = if self.rail_pinned {
                label.to_string()
            } else {
                short.to_string()
            };
            if ui
                .selectable_label(self.view == View::Side && self.panel == panel, text)
                .clicked()
            {
                // Work on it later; TODO(state.md): Labels, RSS, Recent and Favorites have no daemon storage yet.
                self.panel = panel;
                self.view = View::Side;
            }
        }
        ui.separator();
        let (color, text) = if self.connected {
            (Color32::from_rgb(16, 185, 129), "● Daemon connected")
        } else {
            (Color32::from_rgb(239, 68, 68), "● Offline")
        };
        ui.label(RichText::new(text).color(color).small());
        // Work on it later; TODO(state.md): manual daemon connect/disconnect is not supported; the bridge auto-reconnects.
        unavailable(
            ui,
            "Toggle daemon",
            "Connection is managed automatically by the IPC bridge.",
        );
    }

    pub(super) fn tasks_view(&mut self, ui: &mut egui::Ui) {
        let tasks = self.tasks.clone();
        let mut summaries: SegmentSummaries = HashMap::new();
        let mut workers: WorkerCounts = HashMap::new();
        for task in &tasks {
            summaries.insert(task.url.clone(), self.live_segments.summary(task));
            workers.insert(
                task.url.clone(),
                self.live_segments
                    .active_workers(task)
                    .or(task.active_connections),
            );
        }
        let mut active_filter = self.active_filter.clone();
        let mut transfer_filter = self.transfer_filter.clone();
        let selected = self.selected_task.clone();
        let active: Vec<DownloadInfo> = tasks
            .iter()
            .filter(|task| is_active(task))
            .cloned()
            .collect();
        let transfers: Vec<DownloadInfo> = tasks
            .iter()
            .filter(|task| !is_active(task))
            .cloned()
            .collect();
        let mut new_selection = None;
        let mut focus_filter = self.focus_filter;
        ui.columns(2, |columns| {
            columns[0].vertical(|ui| {
                ui.label(RichText::new(format!("Active ({})", active.len())).strong());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut active_filter)
                        .hint_text("Filter…")
                        .desired_width(160.0)
                        .id(egui::Id::new("rget_active_filter")),
                );
                if focus_filter {
                    response.request_focus();
                    focus_filter = false;
                }
                egui::ScrollArea::vertical()
                    .id_salt("active_list")
                    .show(ui, |ui| {
                        let clicked = render_task_rows(
                            ui,
                            &active,
                            &summaries,
                            &workers,
                            &selected,
                            &active_filter,
                            "No active downloads. Use + URL to add one.",
                        );
                        if let Some(id) = clicked {
                            new_selection = Some(id);
                        }
                    });
            });
            columns[1].vertical(|ui| {
                ui.label(RichText::new(format!("Transfers ({})", transfers.len())).strong());
                ui.add(
                    egui::TextEdit::singleline(&mut transfer_filter)
                        .hint_text("Filter…")
                        .desired_width(160.0),
                );
                // Work on it later; TODO(state.md): seeding / sharing controls are not available in the daemon yet.
                ui.label(RichText::new("Seeding controls are deferred; see state.md.").weak());
                egui::ScrollArea::vertical()
                    .id_salt("transfer_list")
                    .show(ui, |ui| {
                        let clicked = render_task_rows(
                            ui,
                            &transfers,
                            &summaries,
                            &workers,
                            &selected,
                            &transfer_filter,
                            "No completed or stopped transfers",
                        );
                        if let Some(id) = clicked {
                            new_selection = Some(id);
                        }
                    });
            });
        });
        self.focus_filter = focus_filter;
        self.active_filter = active_filter;
        self.transfer_filter = transfer_filter;
        if let Some(id) = new_selection {
            self.selected_task = Some(id);
        }
        ui.separator();
        let selected_task = self
            .selected_task
            .as_ref()
            .and_then(|id| self.tasks.iter().find(|task| &task.url == id))
            .cloned();
        if let Some(task) = selected_task {
            self.task_details(ui, &task);
        } else {
            ui.label(RichText::new("Select a download to view details and actions").weak());
        }
    }

    fn task_details(&mut self, ui: &mut egui::Ui, task: &DownloadInfo) {
        if self.edit_connection_task.as_deref() != Some(task.url.as_str()) {
            self.edit_connection_task = Some(task.url.clone());
            self.edit_connections = task.requested_connections.unwrap_or(1);
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.detail, Detail::Status, "Download status");
            ui.selectable_value(&mut self.detail, Detail::Network, "Network & splits");
            ui.selectable_value(&mut self.detail, Detail::Actions, "Post-download actions");
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("details")
            .show(ui, |ui| match self.detail {
                Detail::Status => {
                    ui.label(
                        RichText::new(
                            task.torrent
                                .as_ref()
                                .map(|torrent| torrent.source.as_str())
                                .unwrap_or(&task.url),
                        )
                        .small(),
                    );
                    egui::Grid::new("details_grid")
                        .num_columns(2)
                        .spacing([24.0, 6.0])
                        .show(ui, |ui| {
                            metric(ui, "Status", &task.state);
                            let (root, files) = torrent_display(task);
                            if let Some(root) = root {
                                metric(ui, "Torrent root", root);
                            }
                            if let Some(files) = files {
                                metric(ui, "Torrent files", &files.to_string());
                            }
                            if let Some(metrics) =
                                task.torrent.as_ref().and_then(|torrent| torrent.metrics.as_ref())
                            {
                                metric(
                                    ui,
                                    "Torrent stalls",
                                    &format!(
                                        "{}{}",
                                        metrics.stall_count,
                                        if metrics.stalled { " (stalled)" } else { "" }
                                    ),
                                );
                                metric(
                                    ui,
                                    "First verified piece",
                                    &metrics
                                        .time_to_first_piece_ms
                                        .map(|ms| format!("{ms} ms"))
                                        .unwrap_or_else(|| "Unknown".into()),
                                );
                            }
                            if auto_renamed(task) {
                                if let Some(name) = &task.suggested_filename {
                                    metric(ui, "Original suggestion", name);
                                }
                            }
                            metric(
                                ui,
                                "Progress",
                                &format!(
                                    "{} / {}",
                                    format_bytes(task.downloaded),
                                    task.total
                                        .map(format_bytes)
                                        .unwrap_or_else(|| "Unknown".into())
                                ),
                            );
                            metric(
                                ui,
                                "Download rate",
                                &format!("{}/s (average)", format_bytes(task.speed_bps)),
                            );
                            let eta = task.total.and_then(|total| {
                                total
                                    .saturating_sub(task.downloaded)
                                    .checked_div(task.speed_bps)
                            });
                            metric(
                                ui,
                                "Time remaining",
                                &eta.map(format_duration)
                                    .unwrap_or_else(|| "Unknown".into()),
                            );
                            let support = if task.resume_supported {
                                "Available (strong ETag and byte ranges)"
                            } else if task.filename.is_none() && task.state == "downloading" {
                                "Checking server response"
                            } else {
                                "Unavailable under rGet's safe-resume requirements"
                            };
                            metric(ui, "Resume support", support);
                            if let Some(path) = &task.final_path {
                                metric(ui, "Saved to", path);
                            } else if let Some(path) = &task.destination_path {
                                metric(ui, "Destination (may change)", path);
                            } else if let Some(name) = &task.filename {
                                metric(ui, "Destination name", name);
                            }
                        });
                    ui.horizontal_wrapped(|ui| {
                        let can_pause = self.connected
                            && task.state == "downloading"
                            && task.resume_supported;
                        if ui
                            .add_enabled(can_pause, egui::Button::new("Pause"))
                            .on_disabled_hover_text(
                                "Requires a downloading task with safe resume support.",
                            )
                            .clicked()
                        {
                            self.send(ClientRequest::Pause {
                                task_id: task.url.clone(),
                            });
                        }
                        let can_resume = self.connected && task.state == "paused";
                        if ui
                            .add_enabled(can_resume, egui::Button::new("Resume"))
                            .on_disabled_hover_text("Available when the task is paused.")
                            .clicked()
                        {
                            self.send(ClientRequest::Resume {
                                task_id: task.url.clone(),
                            });
                        }
                        let can_cancel = self.connected
                            && matches!(
                                task.state.as_str(),
                                "queued"
                                    | "metadata"
                                    | "peers"
                                    | "connecting"
                                    | "downloading"
                                    | "paused"
                                    | "pausing"
                                    | "resuming"
                                    | "finished"
                            );
                        if ui
                            .add_enabled(can_cancel, egui::Button::new("Cancel"))
                            .on_disabled_hover_text("Only active downloads can be canceled.")
                            .clicked()
                        {
                            self.send(ClientRequest::Cancel {
                                task_id: task.url.clone(),
                            });
                        }
                        let can_retry = self.connected
                            && (task.state.starts_with("failed:") || task.state == "canceled");
                        if ui
                            .add_enabled(can_retry, egui::Button::new("Retry"))
                            .clicked()
                        {
                            self.send(add_request(
                                task.torrent
                                    .as_ref()
                                    .map(|torrent| torrent.source.clone())
                                    .unwrap_or_else(|| task.url.clone()),
                                task.requested_connections.unwrap_or(1),
                            ));
                        }
                        if ui.button("Open Folder").clicked() {
                            self.open_folder_for(Some(task));
                        }
                        if ui.button("Copy Link").clicked() {
                            ui.ctx().copy_text(task.url.clone());
                            self.notify("Link copied");
                        }
                        if ui.button("Properties").clicked() {
                            self.properties_task = Some(task.clone());
                        }
                        if ui.button("Remove from list").clicked() {
                            self.send(ClientRequest::RemoveTask {
                                task_id: task.url.clone(),
                                delete_file: false,
                            });
                            self.notify("Removed from list");
                        }
                        if ui.button("Delete from disk").clicked() {
                            self.pending_delete = Some(PendingDelete {
                                task_id: task.url.clone(),
                                delete_file: true,
                            });
                        }
                        // Work on it later; TODO(state.md): seeding / upload controls are not implemented in the daemon yet.
                        unavailable(
                            ui,
                            "Start / Stop seeding",
                            "Seeding and upload are deferred; see state.md.",
                        );
                    });
                }
                Detail::Network => {
                    ui.heading("Network & splits");
                    egui::Grid::new("network_grid").num_columns(2).show(ui, |ui| {
                        metric(ui, "Requested maximum", &count(task.requested_connections));
                        metric(
                            ui,
                            "Active workers",
                            &count(
                                self.live_segments
                                    .active_workers(task)
                                    .or(task.active_connections),
                            ),
                        );
                        if let Some(policy) = self.policies.get(task) {
                            metric(ui, "Segment size target", &format_bytes(policy.target_size));
                            metric(ui, "Current steal floor", &format_bytes(policy.steal_floor));
                            metric(ui, "Last policy reason", &policy.reason);
                        }
                        if let Some((segments, bytes, total)) = self.live_segments.summary(task) {
                            metric(
                                ui,
                                "Segments",
                                &format!(
                                    "{segments} · {:.1}% (live)",
                                    bytes as f64 / total as f64 * 100.0
                                ),
                            );
                        } else if let Some(segments) = task.segment_count {
                            let overall = task
                                .total
                                .filter(|total| *total > 0)
                                .map(|total| {
                                    format!(
                                        "{:.1}% overall",
                                        (task.downloaded as f64 / total as f64 * 100.0)
                                            .clamp(0.0, 100.0)
                                    )
                                })
                                .unwrap_or_else(|| "progress unknown".into());
                            metric(ui, "Segments", &format!("{segments} · {overall} (queue sample)"));
                        }
                    });
                    ui.label(
                        "Active workers own a segment; this is not a TCP socket count. The count changes with ownership and can be lower during startup, throttling, or when work is limited.",
                    );
                    if let Some(note) = &task.connection_note {
                        ui.label(note);
                    }
                    ui.label("Global speed limit is in Settings → Downloads.");
                    ui.horizontal(|ui| {
                        ui.label("New maximum");
                        ui.add_enabled(
                            task.live_connections_supported,
                            egui::DragValue::new(&mut self.edit_connections).range(1..=32),
                        );
                        let can_change = self.connected
                            && task.live_connections_supported
                            && matches!(task.state.as_str(), "downloading" | "paused");
                        if ui
                            .add_enabled(can_change, egui::Button::new("Configure connections"))
                            .on_disabled_hover_text(
                                "Available for active or paused split tasks; single-stream conversion is not yet supported.",
                            )
                            .clicked()
                        {
                            self.send(ClientRequest::SetConnections {
                                task_id: task.url.clone(),
                                count: self.edit_connections,
                            });
                        }
                    });
                    if !task.live_connections_supported
                        && matches!(task.state.as_str(), "downloading" | "paused")
                    {
                        ui.label(
                            "Live changes require a split task; single-stream conversion is not yet supported.",
                        );
                    }
                }
                Detail::Actions => {
                    ui.heading("Post-download actions");
                    ui.label("One global action is available. Configure it in Settings.");
                    let enabled = self.post_download_settings.open_containing_folder;
                    let mut flag = enabled;
                    ui.add_enabled(
                        false,
                        egui::Checkbox::new(&mut flag, "Open containing folder after completion"),
                    );
                    if ui.button("Open Settings").clicked() {
                        self.settings_open = true;
                    }
                    ui.separator();
                    ui.label("Not implemented yet (see state.md):");
                    // Work on it later; TODO(state.md): close rGet, checksum, extraction, antivirus and power actions are stubs.
                    for label in [
                        "Close rGet",
                        "Verify checksum",
                        "Extract archive",
                        "Run antivirus scan",
                        "System power action",
                    ] {
                        ui.add_enabled(false, egui::Checkbox::new(&mut false, label));
                    }
                }
            });
    }

    pub(super) fn downloads_view(&mut self, ui: &mut egui::Ui) {
        if self.last_files_refresh.elapsed() >= Duration::from_secs(3) {
            self.refresh_files();
        }
        ui.heading("Downloads");
        ui.label(
            RichText::new(
                download_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Download directory unavailable".into()),
            )
            .weak(),
        );
        let mut selected_file = self.selected_file.clone();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh").clicked() {
                self.refresh_files();
            }
            if ui
                .add_enabled(selected_file.is_some(), egui::Button::new("Open file"))
                .clicked()
            {
                if let Some(path) = &selected_file {
                    if let Err(error) = open_path(path) {
                        self.notice = Some(error);
                    }
                }
            }
            if ui.button("Open folder").clicked() {
                self.open_folder_for(None);
            }
            if ui
                .add_enabled(selected_file.is_some(), egui::Button::new("Copy path"))
                .clicked()
            {
                if let Some(path) = &selected_file {
                    ui.ctx().copy_text(path.display().to_string());
                    self.notify("Path copied");
                }
            }
            // Work on it later; TODO(state.md): no daemon file-mutation API yet.
            unavailable(
                ui,
                "Rename / move / delete",
                "File mutations are deferred; see state.md.",
            );
        });
        let chips = self.category_chips();
        let categories = self.categories.clone();
        let mut category = self.category.clone();
        ui.horizontal_wrapped(|ui| {
            for (id, label) in chips {
                if ui.selectable_label(category == id, label).clicked() {
                    category = id;
                }
            }
            // Work on it later; TODO(state.md): category persistence and editing are not backed by the daemon yet.
            unavailable(ui, "+ New", "Category editing is deferred; see state.md.");
        });
        let mut library_filter = self.library_filter.clone();
        let mut sort = self.sort;
        let mut descending = self.descending;
        let mut focus_filter = self.focus_filter;
        ui.horizontal(|ui| {
            ui.label("Search:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut library_filter)
                    .hint_text("Search downloads…")
                    .desired_width(180.0)
                    .id(egui::Id::new("rget_library_filter")),
            );
            if focus_filter {
                response.request_focus();
                focus_filter = false;
            }
            ui.label("Sort by:");
            for (option, label) in [
                (Sort::Name, "Name"),
                (Sort::Size, "Size"),
                (Sort::Date, "Modified"),
                (Sort::Type, "Type"),
            ] {
                if ui.selectable_label(sort == option, label).clicked() {
                    if sort == option {
                        descending = !descending;
                    } else {
                        sort = option;
                        descending = false;
                    }
                }
            }
            ui.label(if descending {
                "Descending"
            } else {
                "Ascending"
            });
        });
        self.focus_filter = focus_filter;
        self.category = category;
        self.library_filter = library_filter;
        self.sort = sort;
        self.descending = descending;
        self.sort_files();
        let files = self.files.clone();
        let category = self.category.clone();
        let filter = self.library_filter.trim().to_lowercase();
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("files")
            .show(ui, |ui| {
                if files.is_empty() {
                    ui.label("No files in the Downloads folder.");
                    return;
                }
                let mut shown = 0usize;
                for file in &files {
                    let name = file
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if !filter.is_empty() && !name.to_lowercase().contains(filter.as_str()) {
                        continue;
                    }
                    if category != "all" && cat_of(&categories, &name) != category {
                        continue;
                    }
                    shown += 1;
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                selected_file.as_ref() == Some(&file.path),
                                name.as_str(),
                            )
                            .clicked()
                        {
                            selected_file = Some(file.path.clone());
                        }
                        ui.label(format_bytes(file.size));
                        ui.label(cat_name(&categories, &cat_of(&categories, &name)));
                        ui.label(
                            file.path
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        );
                    });
                    ui.separator();
                }
                if shown == 0 {
                    ui.label("No matching downloads");
                }
            });
        self.selected_file = selected_file;
    }

    pub(super) fn side_view(&mut self, ui: &mut egui::Ui) {
        let title = match self.panel {
            Panel::Categories => "Categories",
            Panel::Labels => "Labels",
            Panel::Rss => "RSS Feeds",
            Panel::Recent => "Recent",
            Panel::Favorites => "Favorites",
        };
        ui.heading(title);
        ui.separator();
        let files = self.files.clone();
        let categories = self.categories.clone();
        let mut category = self.category.clone();
        match self.panel {
            Panel::Categories => {
                for definition in &categories {
                    let count = files
                        .iter()
                        .filter(|file| {
                            let name = file.path.file_name().unwrap_or_default().to_string_lossy();
                            cat_of(&categories, name.as_ref()) == definition.id
                        })
                        .count();
                    if ui
                        .selectable_label(
                            category == definition.id,
                            format!("{} ({})", definition.name, count),
                        )
                        .clicked()
                    {
                        category = definition.id.clone();
                        self.view = View::Downloads;
                        self.refresh_files();
                    }
                }
            }
            Panel::Labels => {
                // Work on it later; TODO(state.md): labels have no daemon storage yet.
                ui.label("Labels are not backed by daemon storage yet.");
                unavailable(ui, "New label", "Deferred; see state.md.");
            }
            Panel::Rss => {
                // Work on it later; TODO(state.md): RSS feeds have no daemon fetcher or storage yet.
                ui.label("RSS feeds are not implemented yet.");
                unavailable(ui, "Add feed", "Deferred; see state.md.");
            }
            Panel::Recent => {
                // Work on it later; TODO(state.md): recent history is derived from daemon queue, not persisted separately.
                ui.label("Recent is derived from the live daemon queue; no separate history yet.");
                unavailable(ui, "Clear recent", "Deferred; see state.md.");
            }
            Panel::Favorites => {
                // Work on it later; TODO(state.md): favorites have no daemon storage yet.
                ui.label("Favorites are not implemented yet.");
                unavailable(ui, "Add favorite", "Deferred; see state.md.");
            }
        }
        self.category = category;
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let down: u64 = self.tasks.iter().map(|task| task.speed_bps).sum();
        let completed = self
            .tasks
            .iter()
            .filter(|task| task.state == "completed")
            .count();
        let active = self.tasks.iter().filter(|task| is_active(task)).count();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("↓ {} /s", format_bytes(down)));
            // Work on it later; TODO(state.md): upload / seeding telemetry is not implemented in the daemon yet.
            ui.label("↑ 0 /s");
            ui.label(format!("Completed: {completed}"));
            ui.label(format!("Active: {active}"));
            ui.label(format!(
                "Daemon: {}",
                if self.connected {
                    "connected"
                } else {
                    "offline"
                }
            ));
            if self.compact {
                ui.label("Compact");
            }
        });
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if self.add_open {
            let mut open = self.add_open;
            let mut submit = false;
            egui::Window::new("Add download")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Paste an HTTP/HTTPS URL or a local .torrent path");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.url)
                            .hint_text("https://example.com/file.zip")
                            .desired_width(420.0),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Maximum connections");
                        ui.add(egui::DragValue::new(&mut self.connections).range(1..=32));
                    });
                    ui.label(
                        "Actual worker count may be lower; unsupported or small resources use one stream.",
                    );
                    ui.label("Files are saved to Downloads. Existing files are not overwritten.");
                    if ui
                        .add_enabled(
                            self.connected && !self.url.trim().is_empty(),
                            egui::Button::new("Start download"),
                        )
                        .clicked()
                    {
                        submit = true;
                    }
                    if !self.connected {
                        ui.label("Waiting for the daemon to connect.");
                    }
                });
            if submit {
                let url = self.url.trim().to_string();
                self.send(add_request(url, self.connections));
                self.url.clear();
                open = false;
                self.view = View::Tasks;
                self.notify("Download added");
            }
            self.add_open = open;
        }

        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("Settings")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (tab, label) in [
                            (SettingsTab::Appearance, "Appearance"),
                            (SettingsTab::Behavior, "Behavior"),
                            (SettingsTab::Notifications, "Notifications"),
                            (SettingsTab::Downloads, "Downloads"),
                            (SettingsTab::Torrent, "Torrent"),
                        ] {
                            ui.selectable_value(&mut self.settings_tab, tab, label);
                        }
                    });
                    ui.separator();
                    match self.settings_tab {
                        SettingsTab::Appearance => {
                            ui.heading("Appearance");
                            let previous = self.theme;
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.theme, Theme::System, "System");
                                ui.selectable_value(&mut self.theme, Theme::Light, "Light");
                                ui.selectable_value(&mut self.theme, Theme::Dark, "Dark");
                            });
                            if self.theme != previous {
                                apply_theme(ctx, self.theme);
                            }
                            ui.checkbox(&mut self.compact, "Compact mode (tighter rows)");
                            ui.label(
                                "Appearance and behavior preferences are saved locally and restored on startup.",
                            );
                        }
                        SettingsTab::Behavior => {
                            ui.heading("Behavior");
                            ui.checkbox(&mut self.confirm_delete, "Confirm before delete");
                            ui.label("Applies to Delete from disk in task actions.");
                        }
                        SettingsTab::Notifications => {
                            ui.heading("Notifications");
                            // Work on it later; TODO(state.md): GUI audio backend is not implemented yet.
                            ui.add_enabled(
                                false,
                                egui::Checkbox::new(&mut self.sound, "Sound on completion"),
                            );
                            ui.add_enabled(
                                false,
                                egui::Checkbox::new(&mut self.fail_sound, "Sound on failure"),
                            );
                            ui.checkbox(&mut self.alerts, "Show alerts in app");
                            ui.label("Alerts gate the toast messages for completed actions.");
                        }
                        SettingsTab::Downloads => {
                            ui.heading("Downloads");
                            ui.label(
                                "Global speed limit uses daemon units: decimal kilobits per second. 0 = unlimited.",
                            );
                            ui.add(egui::Slider::new(&mut self.speed_limit_kbps, 0..=100_000));
                            ui.horizontal(|ui| {
                                ui.label(format!("{} kbps", self.speed_limit_kbps));
                                if ui.button("Apply limit").clicked() {
                                    self.set_limit();
                                }
                            });
                            // Work on it later; TODO(state.md): no daemon API for a global concurrent-download cap yet.
                            ui.horizontal(|ui| {
                                ui.label("Max concurrent");
                                ui.add_enabled(
                                    false,
                                    egui::DragValue::new(&mut self.max_concurrent).range(1..=10),
                                );
                            });
                            ui.label(
                                "Connections in Add download are per-task workers, not a global queue cap.",
                            );
                        }
                        SettingsTab::Torrent => {
                            ui.heading("Torrent");
                            // Work on it later; TODO(state.md): seeding / share ratio is not implemented in the daemon yet.
                            ui.horizontal(|ui| {
                                ui.label("Seed until ratio");
                                ui.add_enabled(
                                    false,
                                    egui::DragValue::new(&mut self.ratio)
                                        .range(0.1f32..=10.0f32),
                                );
                            });
                            ui.label("Upload and seeding remain out of scope; see state.md.");
                        }
                    }
                    ui.separator();
                    ui.heading("Post-download actions");
                    if !self.post_download_loaded {
                        ui.label("Loading daemon setting…");
                    } else {
                        let mut enabled = self.post_download_settings.open_containing_folder;
                        let response = ui.add_enabled(
                            self.connected,
                            egui::Checkbox::new(
                                &mut enabled,
                                "Open containing folder after a download completes",
                            ),
                        );
                        if response.clicked() {
                            self.post_download_settings.open_containing_folder = enabled;
                            self.send(ClientRequest::SetPostDownloadSettings {
                                settings: self.post_download_settings,
                            });
                        }
                        ui.label(
                            "The daemon opens the folder in your desktop file manager, even if the GUI is closed. It is skipped on headless daemons without a graphical session and is not applied retroactively to tasks completed before daemon startup.",
                        );
                    }
                });
            self.settings_open = open;
        }

        if let Some(pending) = self.pending_delete.clone() {
            if !self.confirm_delete {
                self.send(ClientRequest::RemoveTask {
                    task_id: pending.task_id.clone(),
                    delete_file: pending.delete_file,
                });
                self.pending_delete = None;
                self.notify(if pending.delete_file {
                    "Deleted from disk"
                } else {
                    "Removed from list"
                });
            } else {
                let mut confirm = false;
                let mut dismiss = false;
                let title = if pending.delete_file {
                    "Delete from disk?"
                } else {
                    "Remove from list?"
                };
                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(if pending.delete_file {
                            "This cancels the transfer and removes its history and its recorded downloaded and partial files."
                        } else {
                            "This removes the task from the daemon queue. Files already on disk are kept."
                        });
                        ui.label(RichText::new(pending.task_id.as_str()).small());
                        ui.horizontal(|ui| {
                            if ui.button("Confirm").clicked() {
                                confirm = true;
                            }
                            if ui.button("Cancel").clicked() {
                                dismiss = true;
                            }
                        });
                    });
                if confirm {
                    self.send(ClientRequest::RemoveTask {
                        task_id: pending.task_id.clone(),
                        delete_file: pending.delete_file,
                    });
                    self.pending_delete = None;
                    self.notify(if pending.delete_file {
                        "Deleted from disk"
                    } else {
                        "Removed from list"
                    });
                } else if dismiss {
                    self.pending_delete = None;
                }
            }
        }

        if self.actions_open {
            let mut open = self.actions_open;
            let mut close_clicked = false;
            let task = self
                .selected_task
                .as_ref()
                .and_then(|id| self.tasks.iter().find(|task| &task.url == id))
                .cloned();
            egui::Window::new("Actions")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    let Some(task) = task else {
                        ui.label("Select a download first.");
                        return;
                    };
                    ui.label(RichText::new(task_name(&task)).strong());
                    ui.separator();
                    if ui.button("Pause").clicked() {
                        self.send(ClientRequest::Pause {
                            task_id: task.url.clone(),
                        });
                    }
                    if ui.button("Resume").clicked() {
                        self.send(ClientRequest::Resume {
                            task_id: task.url.clone(),
                        });
                    }
                    if ui.button("Retry").clicked() {
                        self.send(add_request(
                            task.torrent
                                .as_ref()
                                .map(|torrent| torrent.source.clone())
                                .unwrap_or_else(|| task.url.clone()),
                            task.requested_connections.unwrap_or(1),
                        ));
                    }
                    if ui.button("Cancel").clicked() {
                        self.send(ClientRequest::Cancel {
                            task_id: task.url.clone(),
                        });
                    }
                    if ui.button("Open Folder").clicked() {
                        self.open_folder_for(Some(&task));
                    }
                    if ui.button("Copy Link").clicked() {
                        ui.ctx().copy_text(task.url.clone());
                        self.notify("Link copied");
                    }
                    if ui.button("Properties").clicked() {
                        self.properties_task = Some(task.clone());
                    }
                    if ui.button("Remove from list").clicked() {
                        self.send(ClientRequest::RemoveTask {
                            task_id: task.url.clone(),
                            delete_file: false,
                        });
                        self.notify("Removed from list");
                        close_clicked = true;
                    }
                    if ui.button("Delete from disk").clicked() {
                        self.pending_delete = Some(PendingDelete {
                            task_id: task.url.clone(),
                            delete_file: true,
                        });
                        close_clicked = true;
                    }
                    // Work on it later; TODO(state.md): seeding / upload controls are not implemented in the daemon yet.
                    unavailable(
                        ui,
                        "Start / Stop seeding",
                        "Seeding and upload are deferred; see state.md.",
                    );
                });
            self.actions_open = open && !close_clicked;
        }

        if let Some(task) = self.properties_task.clone() {
            let mut open = true;
            let mut close_clicked = false;
            egui::Window::new("Properties")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    egui::Grid::new("props_grid")
                        .num_columns(2)
                        .spacing([24.0, 6.0])
                        .show(ui, |ui| {
                            metric(ui, "Name", &task_name(&task));
                            metric(ui, "Source", &task.url);
                            metric(ui, "Status", &task.state);
                            metric(
                                ui,
                                "Progress",
                                &format!(
                                    "{} / {}",
                                    format_bytes(task.downloaded),
                                    task.total
                                        .map(format_bytes)
                                        .unwrap_or_else(|| "Unknown".into())
                                ),
                            );
                            metric(ui, "Speed", &format!("{}/s", format_bytes(task.speed_bps)));
                            if let Some(path) = &task.final_path {
                                metric(ui, "Saved to", path);
                            }
                            let (root, files) = torrent_display(&task);
                            if let Some(root) = root {
                                metric(ui, "Torrent root", root);
                            }
                            if let Some(files) = files {
                                metric(ui, "Torrent files", &files.to_string());
                            }
                        });
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
            if !open || close_clicked {
                self.properties_task = None;
            }
        }

        if self.palette_open {
            let mut open = self.palette_open;
            let mut chosen: Option<palette::PaletteAction> = None;
            let query = self.palette_query.clone();
            let matches = palette::matching(&query);
            let selected = self.palette_selected.min(matches.len().saturating_sub(1));
            let mut focus = self.palette_focus;
            egui::Window::new("Command palette")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
                .show(ctx, |ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .hint_text("Search commands…")
                            .desired_width(400.0)
                            .id(egui::Id::new("rget_palette_query")),
                    );
                    if focus {
                        response.request_focus();
                        focus = false;
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            if matches.is_empty() {
                                ui.label("No matching commands.");
                            }
                            for (index, command) in matches.iter().enumerate() {
                                let label = if command.shortcut.is_empty() {
                                    command.label.to_string()
                                } else {
                                    format!("{}   {}", command.label, command.shortcut)
                                };
                                if ui.selectable_label(index == selected, label).clicked() {
                                    chosen = Some(command.action);
                                }
                            }
                        });
                    ui.separator();
                    ui.label("↑ ↓ move · Enter run · Esc close");
                });
            self.palette_focus = focus;
            self.palette_open = open;
            if let Some(action) = chosen {
                self.run_palette_action(action, ctx);
            }
        }

        if self.help_open {
            let mut open = self.help_open;
            egui::Window::new("Help")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.heading("Keyboard shortcuts");
                    ui.label("Ctrl+N / Ctrl+O  Add download");
                    ui.label("Ctrl+S / Ctrl+,  Settings");
                    ui.label("Ctrl+P  Pause all");
                    ui.label("Ctrl+R  Resume all");
                    ui.label("Ctrl+B  Pin / unpin the left rail");
                    ui.label("Ctrl+K  Command palette");
                    ui.label("Ctrl+1  Tasks view");
                    ui.label("Ctrl+2  Downloads library");
                    ui.label("Ctrl+F  Focus the active search filter");
                    ui.label("Ctrl+Shift+Enter  Open Downloads folder");
                    ui.label("Esc  Close the open dialog");
                    ui.separator();
                    ui.label(
                        "Command palette: type to filter, ↑ ↓ to move, Enter to run, Esc to close.",
                    );
                    ui.label(
                        "It lists the wired actions only; no disabled control is exposed there.",
                    );
                    ui.separator();
                    ui.label(
                        "Design-only shortcuts: Ctrl+Shift+M (Add Magnet), Space (pause/resume selected) and Delete (remove selected) are not wired because those controls are disabled; see state.md.",
                    );
                    ui.label(
                        "Category editing, RSS, labels and favorites are tracked in state.md.",
                    );
                });
            self.help_open = open;
        }
    }
}

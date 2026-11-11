mod palette;
mod policies;
mod preferences;
mod progress;
mod segments;
mod views;
use crate::ipc_bridge_worker::{BridgeMessage, IpcBridgeWorker};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo, PostDownloadSettings};
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, PartialEq)]
enum View {
    Tasks,
    Downloads,
    Side,
}
#[derive(Clone, Copy, PartialEq)]
enum Detail {
    Status,
    Network,
    Actions,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sort {
    Name,
    Size,
    Date,
    Type,
}
#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Categories,
    Labels,
    Rss,
    Recent,
    Favorites,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsTab {
    Appearance,
    Behavior,
    Notifications,
    Downloads,
    Torrent,
}

#[derive(Clone)]
struct DownloadFile {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

/// GUI-only category filter. The daemon does not persist or accept categories.
#[derive(Clone)]
struct Category {
    id: String,
    name: String,
    exts: Vec<String>,
}

#[derive(Clone)]
struct PendingDelete {
    task_id: String,
    delete_file: bool,
}

pub struct AppState {
    bridge: IpcBridgeWorker,
    tasks: Vec<DownloadInfo>,
    live_segments: segments::LiveSegments,
    policies: policies::Policies,
    selected_task: Option<String>,
    selected_file: Option<PathBuf>,
    files: Vec<DownloadFile>,
    last_files_refresh: Instant,
    view: View,
    detail: Detail,
    panel: Panel,
    settings_tab: SettingsTab,
    theme: Theme,
    sort: Sort,
    descending: bool,
    compact: bool,
    confirm_delete: bool,
    alerts: bool,
    rail_pinned: bool,
    active_filter: String,
    transfer_filter: String,
    library_filter: String,
    category: String,
    categories: Vec<Category>,
    add_open: bool,
    settings_open: bool,
    help_open: bool,
    actions_open: bool,
    palette_open: bool,
    palette_query: String,
    palette_selected: usize,
    palette_focus: bool,
    focus_filter: bool,
    saved_prefs: preferences::GuiPreferences,
    properties_task: Option<DownloadInfo>,
    pending_delete: Option<PendingDelete>,
    url: String,
    connections: u8,
    edit_connection_task: Option<String>,
    edit_connections: u8,
    speed_limit_kbps: u64,
    sound: bool,
    fail_sound: bool,
    max_concurrent: u8,
    ratio: f32,
    notice: Option<String>,
    connected: bool,
    progress_active: bool,
    post_download_settings: PostDownloadSettings,
    post_download_loaded: bool,
}

fn default_categories() -> Vec<Category> {
    let defs = [
        ("images", "Images", "jpg jpeg png gif webp bmp dng heic"),
        ("videos", "Videos", "mkv mp4 avi mov webm"),
        ("pdfs", "PDFs", "pdf"),
        ("compressed", "Compressed", "zip tar gz 7z rar bz2 xz"),
        ("documents", "Documents", "docx doc txt xlsx pptx"),
        ("audio", "Audio", "mp3 flac wav m4a ogg"),
        ("software", "Software", "iso exe msi dmg deb apk"),
    ];
    defs.iter()
        .map(|(id, name, exts)| Category {
            id: (*id).into(),
            name: (*name).into(),
            exts: exts.split_whitespace().map(str::to_string).collect(),
        })
        .collect()
}

fn cat_of(categories: &[Category], name: &str) -> String {
    if name.contains('.') {
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if let Some(category) = categories
            .iter()
            .find(|category| category.exts.contains(&ext))
        {
            return category.id.clone();
        }
    }
    "other".into()
}

fn cat_name(categories: &[Category], id: &str) -> String {
    match id {
        "all" => "All".into(),
        "other" => "Uncategorized".into(),
        _ => categories
            .iter()
            .find(|category| category.id == id)
            .map(|category| category.name.clone())
            .unwrap_or_else(|| id.into()),
    }
}

fn task_name(task: &DownloadInfo) -> String {
    task.filename
        .clone()
        .or_else(|| {
            task.torrent
                .as_ref()
                .and_then(|torrent| torrent.root_name.clone())
        })
        .unwrap_or_else(|| {
            task.url
                .rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())
                .unwrap_or(&task.url)
                .to_string()
        })
}

fn state_label(task: &DownloadInfo) -> String {
    let label = match task.state.as_str() {
        "downloading" => "Downloading",
        "paused" => "Paused",
        "pausing" => "Pausing",
        "resuming" => "Resuming",
        "completed" => "Completed",
        "canceled" => "Canceled",
        "queued" => "Queued",
        "metadata" | "peers" | "connecting" | "finished" => "Preparing",
        state if state.starts_with("failed:") => "Failed",
        _ => "Unknown",
    };
    label.into()
}

fn is_active(task: &DownloadInfo) -> bool {
    !matches!(task.state.as_str(), "completed" | "canceled") && !task.state.starts_with("failed:")
}

impl AppState {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs = preferences::load();
        apply_theme(&cc.egui_ctx, prefs.theme);
        Self {
            bridge: IpcBridgeWorker::start(cc.egui_ctx.clone()),
            tasks: Vec::new(),
            live_segments: segments::LiveSegments::default(),
            policies: policies::Policies::default(),
            selected_task: None,
            selected_file: None,
            files: Vec::new(),
            last_files_refresh: Instant::now() - Duration::from_secs(10),
            view: View::Tasks,
            detail: Detail::Status,
            panel: Panel::Categories,
            settings_tab: prefs.settings_tab,
            theme: prefs.theme,
            sort: prefs.sort,
            descending: prefs.descending,
            compact: prefs.compact,
            confirm_delete: prefs.confirm_delete,
            alerts: prefs.alerts,
            rail_pinned: prefs.rail_pinned,
            active_filter: String::new(),
            transfer_filter: String::new(),
            library_filter: String::new(),
            category: "all".into(),
            categories: default_categories(),
            add_open: false,
            settings_open: false,
            help_open: false,
            actions_open: false,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_focus: false,
            focus_filter: false,
            saved_prefs: prefs,
            properties_task: None,
            pending_delete: None,
            url: String::new(),
            connections: 8,
            edit_connection_task: None,
            edit_connections: 1,
            speed_limit_kbps: 0,
            sound: true,
            fail_sound: true,
            max_concurrent: 5,
            ratio: 2.0,
            notice: None,
            connected: false,
            progress_active: false,
            post_download_settings: PostDownloadSettings::default(),
            post_download_loaded: false,
        }
    }

    fn poll(&mut self) {
        while let Ok(message) = self.bridge.updates.try_recv() {
            match message {
                BridgeMessage::ProgressDisconnected => self.progress_active = false,
                BridgeMessage::ProgressEvent(event) => {
                    if matches!(event, DaemonEvent::ProgressReset) {
                        self.progress_active = true;
                        self.connected = true;
                    }
                    progress::apply(&mut self.tasks, event);
                }
                BridgeMessage::PoliciesDisconnected => self.policies.clear(),
                BridgeMessage::PolicyEvent(event) => self.policies.apply(event),
                BridgeMessage::SegmentsDisconnected => self.live_segments.clear(),
                BridgeMessage::SegmentEvent(event) => self.live_segments.apply(event),
                BridgeMessage::ConnectionError(error) => {
                    self.connected = false;
                    self.post_download_loaded = false;
                    self.notice = Some(format!("Daemon disconnected: {error}"));
                }
                BridgeMessage::Event(DaemonEvent::DetailedQueue { tasks }) => {
                    self.connected = true;
                    if !self.progress_active {
                        self.tasks = tasks;
                    }
                }
                BridgeMessage::Event(DaemonEvent::ConnectionsChanged { task_id, requested }) => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.url == task_id) {
                        task.requested_connections = Some(requested);
                    }
                    self.notify(format!("Requested maximum set to {requested} workers."));
                }
                BridgeMessage::Event(DaemonEvent::TaskFailed { error, .. }) => {
                    self.notice = Some(error);
                }
                BridgeMessage::Event(DaemonEvent::TaskStateChanged { task_id, state, .. }) => {
                    if state.starts_with("existing:") {
                        self.selected_task = Some(task_id);
                        self.view = View::Tasks;
                    }
                    self.notify(format!("Task: {state}."));
                }
                BridgeMessage::Event(DaemonEvent::PostDownloadSettings { settings }) => {
                    self.post_download_settings = settings;
                    self.post_download_loaded = true;
                }
                BridgeMessage::Event(DaemonEvent::ChecksumVerified { algorithm, .. }) => {
                    self.notify(format!("Checksum verified ({algorithm})."));
                }
                BridgeMessage::Event(_) => {}
            }
        }
        if self.connected && !self.post_download_loaded {
            self.send(ClientRequest::GetPostDownloadSettings);
            self.post_download_loaded = true;
        }
    }

    /// User-facing toasts obey the in-app alerts toggle. Daemon errors and
    /// disconnects are always shown through direct `notice` assignments.
    fn notify(&mut self, message: impl Into<String>) {
        if self.alerts {
            self.notice = Some(message.into());
        }
    }

    fn send(&mut self, req: ClientRequest) {
        if self.bridge.requests.send(req).is_err() {
            self.notice = Some("The background connection worker stopped.".into());
        }
    }

    fn refresh_files(&mut self) {
        self.last_files_refresh = Instant::now();
        let Some(dir) = download_dir() else { return };
        let Ok(entries) = std::fs::read_dir(dir) else {
            self.files.clear();
            return;
        };
        self.files = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if !meta.is_file() || entry.file_name().to_string_lossy().starts_with(".rget-") {
                    return None;
                }
                Some(DownloadFile {
                    path: entry.path(),
                    size: meta.len(),
                    modified: meta.modified().ok(),
                })
            })
            .collect();
        self.sort_files();
    }

    fn sort_files(&mut self) {
        let sort = self.sort;
        self.files.sort_by(|a, b| match sort {
            Sort::Name => a.path.file_name().cmp(&b.path.file_name()),
            Sort::Size => a.size.cmp(&b.size),
            Sort::Date => a.modified.cmp(&b.modified),
            Sort::Type => a.path.extension().cmp(&b.path.extension()),
        });
        if self.descending {
            self.files.reverse();
        }
    }

    fn category_chips(&self) -> Vec<(String, String)> {
        let mut chips = vec![
            ("all".to_string(), "All".to_string()),
            ("other".to_string(), "Uncategorized".to_string()),
        ];
        for category in &self.categories {
            chips.push((category.id.clone(), category.name.clone()));
        }
        chips
    }

    fn pause_all(&mut self) {
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.state == "downloading" && task.resume_supported)
            .map(|task| task.url.clone())
            .collect();
        for id in &ids {
            self.send(ClientRequest::Pause {
                task_id: id.clone(),
            });
        }
        self.notify(format!("Paused {} eligible download(s)", ids.len()));
    }

    fn resume_all(&mut self) {
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.state == "paused")
            .map(|task| task.url.clone())
            .collect();
        for id in &ids {
            self.send(ClientRequest::Resume {
                task_id: id.clone(),
            });
        }
        self.notify(format!("Resumed {} paused download(s)", ids.len()));
    }

    fn remove_completed(&mut self) {
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.state == "completed")
            .map(|task| task.url.clone())
            .collect();
        for id in &ids {
            self.send(ClientRequest::RemoveTask {
                task_id: id.clone(),
                delete_file: false,
            });
        }
        self.notify(format!(
            "Removed {} completed item(s) from the list (files kept)",
            ids.len()
        ));
    }

    fn set_limit(&mut self) {
        self.send(ClientRequest::SetLimit {
            task_id: None,
            kbps: self.speed_limit_kbps,
        });
    }

    /// GUI-side folder opening. The daemon's global post-download action is a
    /// separate, persisted setting and is not invoked from here.
    fn open_folder_for(&mut self, task: Option<&DownloadInfo>) {
        let candidate = task
            .and_then(|task| task.final_path.as_ref())
            .map(PathBuf::from);
        let target = match candidate {
            Some(path) => {
                if path.is_dir() {
                    Some(path)
                } else {
                    path.parent().map(|parent| parent.to_path_buf())
                }
            }
            None => download_dir(),
        };
        match target {
            Some(path) => {
                if let Err(error) = open_path(&path) {
                    self.notice = Some(error);
                }
            }
            None => self.notice = Some("Download directory unavailable".into()),
        }
    }

    /// Persist only when a preference actually changed. Called once per frame
    /// after all panels and dialogs have had a chance to mutate the settings.
    fn save_prefs_if_changed(&mut self) {
        let current = preferences::GuiPreferences {
            theme: self.theme,
            compact: self.compact,
            confirm_delete: self.confirm_delete,
            alerts: self.alerts,
            rail_pinned: self.rail_pinned,
            sort: self.sort,
            descending: self.descending,
            settings_tab: self.settings_tab,
        };
        if current == self.saved_prefs {
            return;
        }
        if let Err(error) = preferences::save(&current) {
            self.notice = Some(format!("Could not save GUI preferences: {error}"));
        }
        self.saved_prefs = current;
    }

    fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.palette_focus = true;
    }

    /// Palette entries delegate to actions that already exist elsewhere in the
    /// UI; no new `ClientRequest` variant is introduced for the palette.
    fn run_palette_action(&mut self, action: palette::PaletteAction, ctx: &egui::Context) {
        use palette::PaletteAction;
        match action {
            PaletteAction::AddDownload => self.add_open = true,
            PaletteAction::OpenSettings => self.settings_open = true,
            PaletteAction::OpenHelp => self.help_open = true,
            PaletteAction::PauseAll => self.pause_all(),
            PaletteAction::ResumeAll => self.resume_all(),
            PaletteAction::RemoveCompleted => self.remove_completed(),
            PaletteAction::ToggleRail => self.rail_pinned = !self.rail_pinned,
            PaletteAction::ShowTasks => self.view = View::Tasks,
            PaletteAction::ShowDownloads => {
                self.view = View::Downloads;
                self.refresh_files();
            }
            PaletteAction::ShowCategories => {
                self.panel = Panel::Categories;
                self.view = View::Side;
            }
            PaletteAction::Theme(theme) => {
                self.theme = theme;
                apply_theme(ctx, theme);
            }
            PaletteAction::ToggleCompact => self.compact = !self.compact,
            PaletteAction::ToggleConfirmDelete => {
                self.confirm_delete = !self.confirm_delete;
            }
            PaletteAction::ToggleAlerts => self.alerts = !self.alerts,
            PaletteAction::FocusFilter => self.focus_filter = true,
            PaletteAction::OpenDownloadsFolder => self.open_folder_for(None),
        }
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.palette_open {
            let (escape, enter, up, down, toggle) = ctx.input(|input| {
                let ctrl = input.modifiers.ctrl;
                (
                    input.key_pressed(egui::Key::Escape),
                    input.key_pressed(egui::Key::Enter),
                    input.key_pressed(egui::Key::ArrowUp),
                    input.key_pressed(egui::Key::ArrowDown),
                    ctrl && input.key_pressed(egui::Key::K),
                )
            });
            if escape || toggle {
                self.palette_open = false;
                return;
            }
            let matches = palette::matching(&self.palette_query);
            if matches.is_empty() {
                self.palette_selected = 0;
                return;
            }
            let last = matches.len() - 1;
            if up {
                self.palette_selected = self.palette_selected.saturating_sub(1);
            }
            if down {
                self.palette_selected = (self.palette_selected + 1).min(last);
            }
            self.palette_selected = self.palette_selected.min(last);
            if enter {
                let action = matches[self.palette_selected].action;
                self.run_palette_action(action, ctx);
            }
            return;
        }
        #[derive(Default)]
        struct Pressed {
            add: bool,
            settings: bool,
            pause: bool,
            resume: bool,
            pin: bool,
            palette: bool,
            tasks: bool,
            downloads: bool,
            focus_filter: bool,
            open_folder: bool,
            escape: bool,
        }
        let pressed = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            Pressed {
                add: ctrl && (input.key_pressed(egui::Key::N) || input.key_pressed(egui::Key::O)),
                settings: ctrl
                    && (input.key_pressed(egui::Key::S) || input.key_pressed(egui::Key::Comma)),
                pause: ctrl && input.key_pressed(egui::Key::P),
                resume: ctrl && input.key_pressed(egui::Key::R),
                pin: ctrl && input.key_pressed(egui::Key::B),
                palette: ctrl && input.key_pressed(egui::Key::K),
                tasks: ctrl && input.key_pressed(egui::Key::Num1),
                downloads: ctrl && input.key_pressed(egui::Key::Num2),
                focus_filter: ctrl && input.key_pressed(egui::Key::F),
                open_folder: ctrl && shift && input.key_pressed(egui::Key::Enter),
                escape: input.key_pressed(egui::Key::Escape),
            }
        });
        if pressed.palette {
            self.open_palette();
            return;
        }
        if pressed.escape {
            self.add_open = false;
            self.settings_open = false;
            self.help_open = false;
            self.actions_open = false;
            self.properties_task = None;
            self.pending_delete = None;
            return;
        }
        if pressed.add {
            self.add_open = true;
        }
        if pressed.settings {
            self.settings_open = true;
        }
        if pressed.pause {
            self.pause_all();
        }
        if pressed.resume {
            self.resume_all();
        }
        if pressed.pin {
            self.rail_pinned = !self.rail_pinned;
        }
        if pressed.tasks {
            self.view = View::Tasks;
        }
        if pressed.downloads {
            self.view = View::Downloads;
            self.refresh_files();
        }
        if pressed.focus_filter {
            self.focus_filter = true;
        }
        if pressed.open_folder {
            self.open_folder_for(None);
        }
    }
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
}

impl eframe::App for AppState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();
        self.handle_shortcuts(&ctx);
        ctx.request_repaint_after(Duration::from_millis(500));
        if self.compact {
            let spacing = ui.spacing_mut();
            spacing.item_spacing.y = 2.0;
            spacing.button_padding.y = 1.0;
        }
        egui::Panel::top("ribbon").show(ui, |ui| self.ribbon(ui));
        egui::Panel::top("actionbar").show(ui, |ui| self.action_bar(ui));
        let rail_width = if self.rail_pinned { 200.0 } else { 60.0 };
        egui::Panel::left("rail")
            .resizable(false)
            .exact_size(rail_width)
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(notice) = self.notice.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(notice);
                    if ui.small_button("Dismiss").clicked() {
                        self.notice = None;
                    }
                });
                ui.separator();
            }
            match self.view {
                View::Tasks => self.tasks_view(ui),
                View::Downloads => self.downloads_view(ui),
                View::Side => self.side_view(ui),
            }
        });
        self.dialogs(&ctx);
        self.save_prefs_if_changed();
    }
}

fn add_request(url: String, connections: u8) -> ClientRequest {
    ClientRequest::AddTask {
        url,
        mirrors: vec![],
        connections: connections.clamp(1, 32),
        extract: false,
        category: None,
        not_before: None,
    }
}
fn auto_renamed(task: &DownloadInfo) -> bool {
    matches!((&task.suggested_filename, &task.filename), (Some(original), Some(actual)) if original != actual)
}
fn torrent_display(task: &DownloadInfo) -> (Option<&str>, Option<usize>) {
    match &task.torrent {
        Some(torrent) => (
            torrent.root_name.as_deref(),
            torrent.files.as_ref().map(Vec::len),
        ),
        None => (None, None),
    }
}
fn count(value: Option<u8>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "Unknown".into())
}
// Work on it later; see state.md for the long-term backlog.
fn unavailable(ui: &mut egui::Ui, label: &str, reason: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_hover_text(reason);
}
fn metric(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.label(RichText::new(name).weak());
    ui.label(value);
    ui.end_row();
}
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut i = 0;
    while value >= 1024.0 && i < units.len() - 1 {
        value /= 1024.0;
        i += 1;
    }
    format!("{value:.1} {}", units[i])
}
fn format_duration(seconds: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn download_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| PathBuf::from(home).join("Downloads"))
}
fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    return Err("Opening files is unsupported on this platform".into());
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    result
        .map(|_| ())
        .map_err(|e| format!("Could not open {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn add_request_uses_selected_and_clamped_connections() {
        assert!(matches!(
            add_request("https://example.test/file".into(), 8),
            ClientRequest::AddTask { connections: 8, .. }
        ));
        assert!(matches!(
            add_request("https://example.test/file".into(), 0),
            ClientRequest::AddTask { connections: 1, .. }
        ));
        assert!(matches!(
            add_request("https://example.test/file".into(), 33),
            ClientRequest::AddTask {
                connections: 32,
                ..
            }
        ));
    }
    #[test]
    fn renamed_badge_requires_a_known_different_suggestion() {
        let mut task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"downloading","filename":"file (1).bin","downloaded":0,"total":null,"speed_bps":0,"final_path":null}"#).unwrap();
        assert!(!auto_renamed(&task));
        task.suggested_filename = Some("file.bin".into());
        assert!(auto_renamed(&task));
        task.filename = Some("file.bin".into());
        assert!(!auto_renamed(&task));
        task.filename = None;
        assert!(!auto_renamed(&task));
    }
    #[test]
    fn torrent_display_uses_persisted_root_and_file_count() {
        let task: DownloadInfo = serde_json::from_str(r#"{"url":"task","state":"downloading","filename":null,"downloaded":0,"total":null,"speed_bps":0,"final_path":null,"torrent":{"source":"file.torrent","info_hash":"hash","created_at":1,"root_name":"bundle","files":[{"path":"a","length":3,"priority":1},{"path":"empty","length":0,"priority":1}]}}"#).unwrap();
        assert_eq!(torrent_display(&task), (Some("bundle"), Some(2)));
        let mut legacy = task;
        legacy.torrent.as_mut().unwrap().root_name = None;
        legacy.torrent.as_mut().unwrap().files = None;
        assert_eq!(torrent_display(&legacy), (None, None));
    }
    #[test]
    fn formats_transfer_values() {
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_duration(3661), "01:01:01");
    }
    #[test]
    fn category_lookup_is_extension_based() {
        let categories = default_categories();
        assert_eq!(cat_of(&categories, "photo.JPG"), "images");
        assert_eq!(cat_of(&categories, "archive.tar.gz"), "compressed");
        assert_eq!(cat_of(&categories, "noextension"), "other");
        assert_eq!(cat_name(&categories, "other"), "Uncategorized");
    }
}

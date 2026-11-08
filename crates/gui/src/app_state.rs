mod views;
mod segments;
mod policies;
use crate::ipc_bridge_worker::{BridgeMessage, IpcBridgeWorker};
use downloader_ipc::{ClientRequest, DaemonEvent, DownloadInfo};
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, PartialEq)]
enum View { Tasks, Downloads }
#[derive(Clone, Copy, PartialEq)]
enum Detail { Status, Network, Actions }
#[derive(Clone, Copy, PartialEq)]
enum Theme { System, Light, Dark }
#[derive(Clone, Copy, PartialEq)]
enum Sort { Name, Size, Date, Type }

#[derive(Clone)]
struct DownloadFile { path: PathBuf, size: u64, modified: Option<SystemTime> }

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
    theme: Theme,
    sort: Sort,
    descending: bool,
    add_open: bool,
    settings_open: bool,
    pending_delete: Option<String>,
    url: String,
    connections: u8,
    edit_connection_task: Option<String>,
    edit_connections: u8,
    notice: Option<String>,
    connected: bool,
}

impl AppState {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            bridge: IpcBridgeWorker::start(cc.egui_ctx.clone()), tasks: Vec::new(), live_segments: segments::LiveSegments::default(), policies: policies::Policies::default(), selected_task: None,
            selected_file: None, files: Vec::new(), last_files_refresh: Instant::now() - Duration::from_secs(10),
            view: View::Tasks, detail: Detail::Status, theme: Theme::System,
            sort: Sort::Name, descending: false, add_open: false, settings_open: false,
            pending_delete: None, url: String::new(), connections: 8,
            edit_connection_task: None, edit_connections: 1, notice: None, connected: false,
        }
    }

    fn poll(&mut self) {
        while let Ok(message) = self.bridge.updates.try_recv() {
            match message {
                BridgeMessage::PoliciesDisconnected => self.policies.clear(),
                BridgeMessage::PolicyEvent(event) => self.policies.apply(event),
                BridgeMessage::SegmentsDisconnected => self.live_segments.clear(),
                BridgeMessage::SegmentEvent(event) => self.live_segments.apply(event),
                BridgeMessage::ConnectionError(error) => { self.connected = false; self.notice = Some(format!("Daemon disconnected: {error}")); }
                BridgeMessage::Event(DaemonEvent::DetailedQueue { tasks }) => {
                    self.connected = true;
                    self.tasks = tasks;
                }
                BridgeMessage::Event(DaemonEvent::ConnectionsChanged { task_id, requested }) => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.url == task_id) { task.requested_connections = Some(requested); }
                    self.notice = Some(format!("Requested maximum set to {requested} workers."));
                }
                BridgeMessage::Event(DaemonEvent::TaskFailed { error, .. }) => self.notice = Some(error),
                BridgeMessage::Event(DaemonEvent::TaskStateChanged { state, .. }) => {
                    self.notice = Some(format!("Task: {state}."));
                }
                BridgeMessage::Event(_) => {}
            }
        }
    }

    fn send(&mut self, req: ClientRequest) {
        if self.bridge.requests.send(req).is_err() { self.notice = Some("The background connection worker stopped.".into()); }
    }

    fn refresh_files(&mut self) {
        self.last_files_refresh = Instant::now();
        let Some(dir) = download_dir() else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { self.files.clear(); return };
        self.files = entries.filter_map(Result::ok).filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() || entry.file_name().to_string_lossy().starts_with(".rget-") { return None; }
            Some(DownloadFile { path: entry.path(), size: meta.len(), modified: meta.modified().ok() })
        }).collect();
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
        if self.descending { self.files.reverse(); }
    }


}

impl eframe::App for AppState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::left("sidebar").resizable(false).exact_size(172.0).show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(notice) = self.notice.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(notice);
                    if ui.small_button("Dismiss").clicked() { self.notice = None; }
                });
                ui.separator();
            }
            match self.view { View::Tasks => self.tasks_view(ui), View::Downloads => self.downloads_view(ui) }
        });
        self.dialogs(ui.ctx());
    }
}

fn add_request(url: String, connections: u8) -> ClientRequest {
    ClientRequest::AddTask { url, mirrors: vec![], connections: connections.clamp(1, 32), extract: false, category: None, not_before: None }
}
fn count(value: Option<u8>) -> String { value.map(|n| n.to_string()).unwrap_or_else(|| "Unknown".into()) }
fn unavailable(ui: &mut egui::Ui, label: &str, reason: &str) {
    ui.add_enabled(false, egui::Button::new(label)).on_hover_text(reason);
}
fn metric(ui: &mut egui::Ui, name: &str, value: &str) { ui.label(RichText::new(name).weak()); ui.label(value); ui.end_row(); }
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 { return format!("{bytes} B"); }
    let mut value = bytes as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut i = 0;
    while value >= 1024.0 && i < units.len() - 1 { value /= 1024.0; i += 1; }
    format!("{value:.1} {}", units[i])
}
fn format_duration(seconds: u64) -> String { format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60) }
fn download_dir() -> Option<PathBuf> { std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|home| PathBuf::from(home).join("Downloads")) }
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
    result.map(|_| ()).map_err(|e| format!("Could not open {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn add_request_uses_selected_and_clamped_connections() {
        assert!(matches!(add_request("https://example.test/file".into(), 8), ClientRequest::AddTask { connections: 8, .. }));
        assert!(matches!(add_request("https://example.test/file".into(), 0), ClientRequest::AddTask { connections: 1, .. }));
        assert!(matches!(add_request("https://example.test/file".into(), 33), ClientRequest::AddTask { connections: 32, .. }));
    }
    #[test]
    fn formats_transfer_values() {
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_duration(3661), "01:01:01");
    }
}

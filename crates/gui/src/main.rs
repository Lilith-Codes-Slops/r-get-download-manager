#![forbid(unsafe_code)]

mod categories;
mod chrome;
mod clipboard;
#[path = "dialogs.rs"]
mod dialogs;
mod download_link;
mod downloads;
mod fake_backend;
mod flyout_logic;
mod graphics;
mod icons;
mod menu_model;
mod menus;
mod modal_style;
mod native_file_dialog;
mod properties;
mod rail;
mod row_layout;
mod row_ui;
mod search_model;
mod settings;
mod settings_layout;
mod settings_logic;
mod shortcut_logic;
mod simulation;
mod simulation_clock;
mod task_filter;
mod tasks;
mod theme;
mod toast_logic;
mod toolbar_layout;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use fake_backend::{Command, FakeBackend, Scope, Selection, State, View};

/// This process controls only the simulated backend, never the real daemon.
struct DownloadsApp {
    backend: FakeBackend,
    chrome: chrome::Chrome,
    downloads: downloads::DownloadsView,
    dialogs: dialogs::Dialogs,
    rail: rail::Rail,
    settings: settings::SettingsModal,
    clipboard: clipboard::Clipboard,
    torrent_picker: Option<(Receiver<native_file_dialog::PickResult>, u64)>,
    message: Option<String>,
    toast: Option<(String, Instant, bool)>,
    simulation_clock: simulation_clock::SimulationClock,
    simulation_seed: u64,
}

impl DownloadsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> std::io::Result<Self> {
        let backend = FakeBackend::open(PathBuf::from("data/state.json"))?;
        let chrome = chrome::Chrome {
            view: backend.snapshot().view,
            theme: backend.snapshot().settings.theme,
            picking_torrent: false,
            search_open: false,
        };
        theme::apply_theme(&cc.egui_ctx, chrome.theme.preference());
        Ok(Self {
            backend,
            chrome,
            downloads: downloads::DownloadsView::default(),
            dialogs: dialogs::Dialogs::default(),
            rail: rail::Rail::new(&cc.egui_ctx),
            settings: settings::SettingsModal::default(),
            clipboard: clipboard::Clipboard::default(),
            torrent_picker: None,
            message: None,
            toast: None,
            simulation_clock: simulation_clock::SimulationClock::new(Instant::now()),
            simulation_seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0x9e3779b97f4a7c15),
        })
    }

    fn dispatch(&mut self, command: Command) -> bool {
        let toast = toast_logic::action_message(&command, self.backend.snapshot());
        match self.backend.dispatch(command) {
            Ok(()) => {
                self.message = None;
                if let Some(text) = toast {
                    self.notify(text);
                }
                true
            }
            Err(error) => {
                self.message = Some(format!("Prototype operation failed: {error}"));
                false
            }
        }
    }

    fn notify(&mut self, text: String) {
        if self.backend.snapshot().settings.alerts || toast_logic::visible_with_alerts_off(&text) {
            let forced = toast_logic::visible_with_alerts_off(&text);
            self.toast = Some((text, Instant::now(), forced));
        }
    }

    fn modal_open(&self) -> bool {
        self.downloads.has_dialog() || self.settings.is_open() || self.dialogs.is_open()
    }

    fn open_torrent_picker(&mut self, ctx: &egui::Context) {
        if self.chrome.picking_torrent || self.modal_open() {
            return;
        }
        self.message = None;
        let generation = self.dialogs.open_add(ctx);
        self.chrome.picking_torrent = true;
        self.torrent_picker = Some((native_file_dialog::pick_torrent(ctx.clone()), generation));
    }

    fn copy_items(&mut self, items: &[Selection], link_only: bool, cut: bool) {
        let text = menu_model::clipboard_text(self.backend.snapshot(), items, link_only);
        let result = text
            .ok_or_else(|| {
                "No actual path or source is available for every selected item".to_owned()
            })
            .and_then(|text| self.clipboard.write(text));
        match result {
            Ok(()) => self.notify(
                if link_only {
                    "Link copied"
                } else if cut {
                    "Cut to clipboard (no files moved in preview)"
                } else {
                    "Copied to clipboard"
                }
                .into(),
            ),
            Err(error) => {
                self.message = Some(format!("Clipboard error: {error}"));
                self.notify("Copy is blocked in this preview".into());
            }
        }
    }

    fn remove_items(&mut self, items: Vec<Selection>) {
        if items.is_empty() {
            return;
        }
        let name = if items.len() == 1 {
            selected_name(self.backend.snapshot(), &items[0])
        } else {
            None
        };
        let count = items.len();
        if self.dispatch(Command::RemoveItems(items)) {
            if let Some(name) = name {
                self.notify(format!("Removed {name} from the list"));
            } else {
                self.notify(format!("Removed {count} items from the list"));
            }
        }
    }

    fn handle_menu(&mut self, action: menus::Action, ctx: &egui::Context) {
        use menu_model::RowCommand;
        use menus::Action;
        match action {
            Action::Add => {
                self.message = None;
                self.dialogs.open_add(ctx);
            }
            Action::Torrent => self.open_torrent_picker(ctx),
            Action::Settings => self.settings.open(ctx, &self.backend.snapshot().settings),
            Action::Search => {
                self.chrome.search_open = !self.backend.snapshot().library_filter.trim().is_empty();
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("global_search")));
            }
            Action::PauseAll => {
                self.dispatch(Command::PauseAll);
            }
            Action::ResumeAll => {
                self.dispatch(Command::ResumeAll);
            }
            Action::ToggleSelected => {
                let state = self.backend.snapshot();
                if let Some(selection) = state.selections.first().cloned() {
                    let command = match selection.scope {
                        Scope::Task => Some(Command::ToggleTask(selection.id)),
                        Scope::Seed => Some(Command::ToggleSeed(selection.id)),
                        Scope::Library => None,
                    };
                    if let Some(command) = command {
                        self.dispatch(command);
                    }
                }
            }
            Action::RemoveSelected => {
                let items = self.backend.snapshot().selections.clone();
                self.remove_items(items);
            }
            Action::ClearCompleted => {
                let count = self.backend.snapshot().library.len();
                if count > 0 && self.dispatch(Command::ClearCompleted) {
                    self.notify(format!(
                        "Removed {count} completed item{} from the list (files kept)",
                        if count == 1 { "" } else { "s" }
                    ));
                }
            }
            Action::CopySelected | Action::CutSelected => {
                let cut = matches!(action, Action::CutSelected);
                let items = self.backend.snapshot().selections.clone();
                self.copy_items(&items, false, cut);
            }
            Action::View(view) => {
                if self.dispatch(Command::SetView(view)) {
                    self.chrome.view = view;
                    self.rail.close_panel();
                }
            }
            Action::Pin => self.rail.toggle_pin(),
            Action::OpenFolder => {
                self.notify("Opening folders is not available in this browser preview".into())
            }
            Action::Row(item, command) => {
                if !menu_model::allowed(self.backend.snapshot(), &item, command) {
                    return;
                }
                match command {
                    RowCommand::Pause | RowCommand::Resume | RowCommand::Retry => {
                        self.dispatch(Command::ToggleTask(item.id));
                    }
                    RowCommand::ToggleSeed => {
                        self.dispatch(Command::ToggleSeed(item.id));
                    }
                    RowCommand::OpenFolder => self
                        .notify("Opening folders is not available in this browser preview".into()),
                    RowCommand::CopyLink => self.copy_items(&[item], true, false),
                    RowCommand::Copy | RowCommand::Cut => {
                        let items = menu_model::targets(self.backend.snapshot(), &item);
                        self.copy_items(&items, false, command == RowCommand::Cut);
                    }
                    RowCommand::Properties => self.dialogs.open_properties(ctx, item),
                    RowCommand::Remove => self.remove_items(vec![item]),
                    RowCommand::Delete => {
                        if self.backend.snapshot().settings.confirm_delete {
                            if let Some(name) = selected_name(self.backend.snapshot(), &item) {
                                self.dialogs.open_delete(ctx, item, name);
                            }
                        } else {
                            self.remove_items(vec![item]);
                        }
                    }
                }
            }
        }
    }

    fn poll_picker(&mut self, ctx: &egui::Context) {
        let Some((receiver, generation)) = &self.torrent_picker else {
            return;
        };
        let generation = *generation;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The file dialog closed unexpectedly.".into()),
        };
        self.chrome.picking_torrent = false;
        self.torrent_picker = None;
        match result {
            Ok(Some(path)) => {
                // A closed or replaced modal must never receive an old picker result.
                self.dialogs.set_torrent(generation, path);
            }
            Ok(None) => self.dialogs.cancel_add(ctx, generation),
            Err(error) => {
                self.message = None;
                if !self.dialogs.set_add_error(generation, error.clone()) {
                    self.toast = Some((error, Instant::now(), true));
                }
            }
        }
    }
}

impl eframe::App for DownloadsApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        // Preserve a smaller limit reported by the device. This value persists
        // in egui between input batches and applies to the on-demand atlas.
        if let Some(max_side) = &mut input.max_texture_side {
            *max_side = (*max_side).min(graphics::FONT_ATLAS_MAX_SIDE);
        }
    }

    // eframe calls logic even for a minimized/occluded viewport, unlike ui().
    // Schedule the next logic pass regardless of focus, visibility, or modals.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let mut steps = 0;
        while self.simulation_clock.due(now) && steps < 8 {
            let before = self.backend.shared_snapshot();
            self.simulation_seed = self.simulation_seed.wrapping_add(0x9e3779b97f4a7c15);
            match self.backend.dispatch(Command::Tick {
                seed: self.simulation_seed,
            }) {
                Ok(()) => {
                    let messages = simulation::notifications(&before, self.backend.snapshot());
                    for message in messages {
                        self.notify(message);
                    }
                    self.simulation_clock.advance();
                }
                Err(error) => {
                    self.message = Some(format!("Simulation could not save or advance: {error}"));
                    self.simulation_clock.retry_after_error(Instant::now());
                    break;
                }
            }
            steps += 1;
        }
        ctx.request_repaint_after(self.simulation_clock.delay(Instant::now()));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_picker(ui.ctx());
        // A popup takes Escape first; an open modal otherwise handles it itself.
        if self.chrome.search_open
            && !self.modal_open()
            && ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.chrome.search_open = false;
        } else if egui::Popup::is_any_open(ui.ctx())
            && ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            egui::Popup::close_all(ui.ctx());
        } else if !self.modal_open() {
            if let Some(shortcut) = take_shortcut(ui.ctx()) {
                use shortcut_logic::Action;
                match shortcut {
                    Action::Add => self.handle_menu(menus::Action::Add, ui.ctx()),
                    Action::Settings => self.handle_menu(menus::Action::Settings, ui.ctx()),
                    Action::Search => self.handle_menu(menus::Action::Search, ui.ctx()),
                    Action::PauseAll => self.handle_menu(menus::Action::PauseAll, ui.ctx()),
                    Action::ResumeAll => self.handle_menu(menus::Action::ResumeAll, ui.ctx()),
                    Action::Tasks => self.handle_menu(menus::Action::View(View::Tasks), ui.ctx()),
                    Action::Downloads => {
                        self.handle_menu(menus::Action::View(View::Downloads), ui.ctx())
                    }
                    Action::Pin => self.handle_menu(menus::Action::Pin, ui.ctx()),
                    Action::OpenFolder => self.handle_menu(menus::Action::OpenFolder, ui.ctx()),
                    Action::Toggle => self.handle_menu(menus::Action::ToggleSelected, ui.ctx()),
                    Action::Remove => self.handle_menu(menus::Action::RemoveSelected, ui.ctx()),
                    Action::Escape => self.rail.close_panel(),
                }
            }
            if !ui.ctx().text_edit_focused() && !egui::Popup::is_any_open(ui.ctx()) {
                let event = ui.ctx().input(|input| {
                    if input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Copy))
                    {
                        Some(menus::Action::CopySelected)
                    } else if input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Cut))
                    {
                        Some(menus::Action::CutSelected)
                    } else {
                        None
                    }
                });
                if let Some(action) = event {
                    if !self.backend.snapshot().selections.is_empty() {
                        self.handle_menu(action, ui.ctx());
                    }
                }
            }
        }
        let palette = if ui.ctx().theme() == egui::Theme::Dark {
            theme::Theme::DARK
        } else {
            theme::Theme::LIGHT
        };
        let previous_view = self.backend.snapshot().view;
        let previous_theme = self.backend.snapshot().settings.theme;
        let modal_open = self.modal_open();
        let (view_clicked, ribbon_action, search_update) =
            self.chrome
                .show_ribbon(ui, palette, self.backend.snapshot(), modal_open);
        if view_clicked {
            self.rail.close_panel();
        }
        if self.chrome.view != previous_view && !self.dispatch(Command::SetView(self.chrome.view)) {
            self.chrome.view = previous_view;
        }
        if self.chrome.theme != previous_theme
            && !self.dispatch(Command::SetTheme(self.chrome.theme))
        {
            self.chrome.theme = previous_theme;
            theme::apply_theme(ui.ctx(), self.chrome.theme.preference());
        }
        if let Some(query) = search_update {
            self.dispatch(Command::SetLibraryFilter(query));
        }
        let bar_action = if self.backend.snapshot().view == View::Tasks {
            self.chrome
                .show_actions(ui, palette, self.backend.snapshot())
        } else {
            let snapshot = self.backend.shared_snapshot();
            for command in self.downloads.show_categories(ui, &snapshot, palette) {
                self.dispatch(command);
            }
            None
        };
        match ribbon_action.or(bar_action) {
            Some(chrome::ChromeAction::Menu(action)) => {
                if !self.modal_open() {
                    self.handle_menu(action, ui.ctx());
                }
            }
            Some(chrome::ChromeAction::OpenAdd) => {
                if !self.downloads.has_dialog()
                    && !self.settings.is_open()
                    && !self.dialogs.is_open()
                {
                    self.message = None;
                    self.dialogs.open_add(ui.ctx());
                }
            }
            Some(chrome::ChromeAction::OpenSettings) => {
                if !self.dialogs.is_open()
                    && !self.downloads.has_dialog()
                    && !self.settings.is_open()
                {
                    self.settings
                        .open(ui.ctx(), &self.backend.snapshot().settings);
                }
            }
            Some(chrome::ChromeAction::PickTorrent) => self.open_torrent_picker(ui.ctx()),
            Some(chrome::ChromeAction::PauseAll) => {
                self.dispatch(Command::PauseAll);
            }
            Some(chrome::ChromeAction::ResumeAll) => {
                self.dispatch(Command::ResumeAll);
            }
            Some(chrome::ChromeAction::RetryAll) => {
                self.dispatch(Command::RetryAll);
            }
            Some(chrome::ChromeAction::SelectSearchResult(item)) => {
                if self.dispatch(Command::SelectSearchResult(item)) {
                    self.chrome.view = self.backend.snapshot().view;
                    self.rail.close_panel();
                    // The view-specific row was laid out earlier in this frame.
                    ui.ctx().request_repaint();
                }
            }
            None => {}
        }
        let snapshot = self.backend.shared_snapshot();
        self.chrome.show_status(ui, palette, &snapshot);
        let rail_rect = self.rail.reserve(ui, palette);
        let mut view_actions = Vec::new();
        let mut row_menu_action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette.bg))
            .show(ui, |ui| {
                if let Some(message) = &self.message {
                    ui.colored_label(palette.red, message);
                }
                view_actions = match snapshot.view {
                    View::Tasks => tasks::show(ui, &snapshot, palette, &mut row_menu_action),
                    View::Downloads => {
                        self.downloads
                            .show(ui, &snapshot, palette, &mut row_menu_action)
                    }
                };
            });
        for command in view_actions {
            self.dispatch(command);
        }
        if let Some(action) = row_menu_action {
            if !self.modal_open() {
                self.handle_menu(action, ui.ctx());
            }
        }
        match self.rail.show(ui.ctx(), rail_rect, palette, &snapshot) {
            Some(rail::RailAction::ToggleDaemon) => {
                if self.dispatch(Command::ToggleDaemon) {
                    let text = if self.backend.snapshot().daemon_connected {
                        "Daemon connected"
                    } else {
                        "Daemon disconnected: downloads paused until reconnected"
                    };
                    self.notify(text.into());
                }
            }
            Some(rail::RailAction::OpenPanel(panel)) => {
                self.dispatch(Command::SetSidePanel(panel.to_owned()));
            }
            Some(rail::RailAction::SelectCategory(category)) => {
                if self.dispatch(Command::SelectCategory(category)) {
                    self.chrome.view = self.backend.snapshot().view;
                }
            }
            None => {}
        }
        if let Some(command) = self.downloads.show_dialog(ui.ctx(), palette) {
            let toast = toast_logic::action_message(&command, self.backend.snapshot());
            match self.backend.dispatch(command) {
                Ok(()) => {
                    self.message = None;
                    self.chrome.view = self.backend.snapshot().view;
                    self.downloads.complete_dialog(ui.ctx(), Ok(()));
                    if let Some(text) = toast {
                        self.notify(text);
                    }
                }
                Err(error) => self
                    .downloads
                    .complete_dialog(ui.ctx(), Err(error.to_string())),
            }
        }
        let current_settings = self.backend.snapshot().settings.clone();
        if let Some(updated) = self.settings.show(ui.ctx(), &current_settings, palette) {
            let choice = updated.theme;
            match self.backend.dispatch(Command::SetSettings(updated)) {
                Ok(()) => {
                    self.message = None;
                    self.chrome.theme = choice;
                    theme::apply_theme(ui.ctx(), choice.preference());
                    ui.ctx().request_repaint();
                }
                Err(error) => {
                    self.settings
                        .save_error(error.to_string(), &current_settings);
                }
            }
        }
        if let Some(command) = self
            .dialogs
            .show(ui.ctx(), self.backend.snapshot(), palette)
        {
            let removed_name = match &command {
                Command::RemoveItems(items) if items.len() == 1 => {
                    selected_name(self.backend.snapshot(), &items[0])
                }
                _ => None,
            };
            let adding = matches!(
                &command,
                Command::AddDownload(_) | Command::AddLinks(_) | Command::AddInputs { .. }
            );
            match self.backend.dispatch(command) {
                Ok(()) => {
                    self.message = None;
                    self.chrome.view = self.backend.snapshot().view;
                    if adding {
                        self.rail.close_panel();
                    }
                    let added_name = if adding {
                        self.backend
                            .snapshot()
                            .tasks
                            .last()
                            .map(|task| task.name.clone())
                    } else {
                        None
                    };
                    self.dialogs.finish_add(ui.ctx(), Ok(()));
                    if let Some(name) = added_name {
                        self.notify(format!("Added {name}"));
                    }
                    if let Some(name) = removed_name {
                        self.notify(format!("Removed {name} from the list"));
                    }
                }
                Err(error) => self.dialogs.finish_add(ui.ctx(), Err(error.to_string())),
            }
        }
        if !self.backend.snapshot().settings.alerts
            && self.toast.as_ref().is_some_and(|(_, _, forced)| !forced)
        {
            self.toast = None;
        }
        if let Some((text, since, _)) = &self.toast {
            let remaining = Duration::from_secs(2).saturating_sub(since.elapsed());
            if remaining.is_zero() {
                self.toast = None;
            } else {
                ui.ctx().request_repaint_after(remaining);
                egui::Area::new(egui::Id::unique("prototype_toast"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -60.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(text);
                        });
                    });
            }
        }
    }
}

/// Take only recognized key events, leaving normal editing/navigation to egui.
fn take_shortcut(ctx: &egui::Context) -> Option<shortcut_logic::Action> {
    use shortcut_logic::Key;
    let typing = ctx.text_edit_focused();
    let control = ctx.memory(|memory| memory.focused().is_some());
    let popup = egui::Popup::is_any_open(ctx);
    ctx.input_mut(|input| {
        let mut action = None;
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let key = match key {
                egui::Key::N => Key::N,
                egui::Key::O => Key::O,
                egui::Key::M => Key::M,
                egui::Key::Comma => Key::Comma,
                egui::Key::F => Key::F,
                egui::Key::P => Key::P,
                egui::Key::R => Key::R,
                egui::Key::Num1 => Key::One,
                egui::Key::Num2 => Key::Two,
                egui::Key::B => Key::B,
                egui::Key::Enter => Key::Enter,
                egui::Key::Space => Key::Space,
                egui::Key::Delete => Key::Delete,
                egui::Key::Escape => Key::Escape,
                _ => return true,
            };
            let found = shortcut_logic::resolve(
                key,
                modifiers.command || modifiers.ctrl || modifiers.mac_cmd,
                modifiers.shift,
                typing,
                control || popup,
            );
            if let Some(found) = found {
                if action.is_none() {
                    action = Some(found);
                }
                return false;
            }
            true
        });
        action
    })
}

fn selected_name(state: &State, selection: &Selection) -> Option<String> {
    match selection.scope {
        Scope::Task => state
            .tasks
            .iter()
            .find(|item| item.id == selection.id)
            .map(|item| item.name.clone()),
        Scope::Seed => state
            .seeds
            .iter()
            .find(|item| item.id == selection.id)
            .map(|item| item.name.clone()),
        Scope::Library => state
            .library
            .iter()
            .find(|item| item.id == selection.id)
            .map(|item| item.name.clone()),
    }
}

fn main() -> Result<(), eframe::Error> {
    eframe::run_native(
        "Downloads",
        graphics::native_options(),
        Box::new(|cc| Ok(Box::new(DownloadsApp::new(cc)?))),
    )
}

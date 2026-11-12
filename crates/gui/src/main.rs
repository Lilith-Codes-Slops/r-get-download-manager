#![forbid(unsafe_code)]

mod categories;
mod chrome;
mod clipboard;
mod config;
mod confirmation_flow;
#[path = "dialogs.rs"]
mod dialogs;
mod directory_scan;
mod download_confirmation;
mod download_confirmation_logic;
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
mod settings_client;
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

/// The prototype's task model remains simulated; settings are daemon-owned.
struct DownloadsApp {
    backend: FakeBackend,
    chrome: chrome::Chrome,
    downloads: downloads::DownloadsView,
    dialogs: dialogs::Dialogs,
    confirmation: confirmation_flow::ConfirmationFlow,
    rail: rail::Rail,
    settings: settings::SettingsModal,
    clipboard: clipboard::Clipboard,
    torrent_picker: Option<(Receiver<native_file_dialog::PickResult>, u64)>,
    download_directory: PathBuf,
    directory_picker: Option<Receiver<native_file_dialog::PickResult>>,
    directory_scan: Option<
        Receiver<std::io::Result<(PathBuf, std::io::Result<Vec<fake_backend::LibraryItem>>)>>,
    >,
    message: Option<String>,
    toast: Option<(String, Instant, bool)>,
    simulation_clock: simulation_clock::SimulationClock,
    simulation_seed: u64,
}

impl DownloadsApp {
    fn new(
        ctx: &egui::Context,
        overrides: settings_client::Overrides,
        download_directory: PathBuf,
    ) -> std::io::Result<Self> {
        let backend = FakeBackend::open_daemon(PathBuf::from("data/state.json"), overrides)?;
        let chrome = chrome::Chrome {
            view: backend.snapshot().view,
            theme: backend.snapshot().settings.theme,
            picking_torrent: false,
            search_open: false,
        };
        theme::apply_theme(ctx, chrome.theme.preference());
        let scan = Self::start_directory_scan(ctx, download_directory.clone(), false);
        Ok(Self {
            backend,
            chrome,
            downloads: downloads::DownloadsView::default(),
            dialogs: dialogs::Dialogs::default(),
            confirmation: confirmation_flow::ConfirmationFlow::default(),
            rail: rail::Rail::new(ctx),
            settings: settings::SettingsModal::default(),
            clipboard: clipboard::Clipboard::default(),
            torrent_picker: None,
            download_directory,
            directory_picker: None,
            directory_scan: Some(scan),
            message: None,
            toast: None,
            simulation_clock: simulation_clock::SimulationClock::new(Instant::now()),
            simulation_seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0x9e3779b97f4a7c15),
        })
    }

    fn start_directory_scan(
        ctx: &egui::Context,
        path: PathBuf,
        save: bool,
    ) -> Receiver<std::io::Result<(PathBuf, std::io::Result<Vec<fake_backend::LibraryItem>>)>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = if save {
                settings_client::set_directory(&path)
            } else {
                Ok(path)
            };
            let result = result.map(|path| {
                let files = directory_scan::scan(&path);
                (path, files)
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        receiver
    }

    fn refresh_directory(&mut self, ctx: &egui::Context) {
        if self.directory_picker.is_some() || self.directory_scan.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = settings_client::load_directory().map(|path| {
                let files = directory_scan::scan(&path);
                (path, files)
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.directory_scan = Some(receiver);
    }

    fn poll_directory(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.directory_picker {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("The folder dialog closed unexpectedly.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.directory_picker = None;
                match result {
                    Ok(Some(path)) => {
                        self.directory_scan = Some(Self::start_directory_scan(ctx, path, true))
                    }
                    Ok(None) => {}
                    Err(error) => self
                        .settings
                        .save_error(error, &self.backend.snapshot().settings),
                }
            }
        }
        if let Some(receiver) = &self.directory_scan {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err(std::io::Error::other("The directory worker stopped")))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.directory_scan = None;
                match result {
                    Ok((path, files)) => {
                        self.download_directory = path;
                        match files {
                            Ok(files) => {
                                let mut state = self.backend.snapshot().clone();
                                directory_scan::merge(&mut state, files);
                                if let Err(error) =
                                    self.backend.dispatch(Command::ReplaceState(state))
                                {
                                    self.message =
                                        Some(format!("Could not update Downloads: {error}"));
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                let mut state = self.backend.snapshot().clone();
                                directory_scan::merge(&mut state, Vec::new());
                                if let Err(error) =
                                    self.backend.dispatch(Command::ReplaceState(state))
                                {
                                    self.message =
                                        Some(format!("Could not update Downloads: {error}"));
                                }
                            }
                            Err(error) => {
                                self.message =
                                    Some(format!("Could not scan download folder: {error}"))
                            }
                        }
                    }
                    Err(error) => self
                        .settings
                        .save_error(error.to_string(), &self.backend.snapshot().settings),
                }
            }
        }
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
        self.downloads.has_dialog()
            || self.settings.is_open()
            || self.dialogs.is_open()
            || self.confirmation.is_open()
    }

    fn confirm_inputs(
        &mut self,
        ctx: &egui::Context,
        mut links: Vec<String>,
        torrent: Option<PathBuf>,
    ) {
        if let Some(path) = torrent {
            let Some(source) = path.to_str() else {
                self.dialogs
                    .finish_add(ctx, Err("The torrent path is not valid UTF-8.".into()));
                return;
            };
            links.push(source.to_owned());
        }
        if links.is_empty() {
            return;
        }
        self.dialogs.finish_add(ctx, Ok(()));
        self.rail.close_panel();
        self.confirmation.begin(links, ctx);
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
            Action::Settings => {
                self.refresh_directory(ctx);
                self.settings.open(ctx, &self.backend.snapshot().settings);
            }
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
        self.poll_directory(ui.ctx());
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
                if !self.modal_open() {
                    self.message = None;
                    self.dialogs.open_add(ui.ctx());
                }
            }
            Some(chrome::ChromeAction::OpenSettings) => {
                if !self.modal_open() {
                    self.refresh_directory(ui.ctx());
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
        let directory_busy = self.directory_picker.is_some() || self.directory_scan.is_some();
        if let Some(updated) = self.settings.show(
            ui.ctx(),
            &current_settings,
            &self.download_directory,
            directory_busy,
            palette,
        ) {
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
        if self.settings.take_browse_request() && !directory_busy {
            self.directory_picker = Some(native_file_dialog::pick_directory(ui.ctx().clone()));
        }
        if self.settings.take_reset_request() {
            if self.directory_picker.is_some() || self.directory_scan.is_some() {
                self.settings.save_error(
                    "Wait for the folder selection to finish before resetting.".into(),
                    &current_settings,
                );
            } else {
                match settings_client::reset() {
                    Ok(()) => {
                        let restart = std::env::current_exe().and_then(|exe| {
                            std::process::Command::new(exe)
                                .args(std::env::args_os().skip(1))
                                .spawn()
                        });
                        match restart {
                        Ok(_) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(error) => self.settings.save_error(
                            format!("Settings were reset, but the GUI could not restart: {error}. Please restart it manually."),
                            &current_settings,
                        ),
                    }
                    }
                    Err(error) => self
                        .settings
                        .save_error(error.to_string(), &current_settings),
                }
            }
        }
        if let Some(command) = self
            .dialogs
            .show(ui.ctx(), self.backend.snapshot(), palette)
        {
            match command {
                Command::AddInputs { links, torrent } => {
                    self.confirm_inputs(ui.ctx(), links, torrent)
                }
                Command::AddDownload(link) => self.confirm_inputs(ui.ctx(), vec![link], None),
                Command::AddLinks(links) => self.confirm_inputs(ui.ctx(), links, None),
                Command::AddTorrent(path) => self.confirm_inputs(ui.ctx(), Vec::new(), Some(path)),
                command => {
                    let removed_name = match &command {
                        Command::RemoveItems(items) if items.len() == 1 => {
                            selected_name(self.backend.snapshot(), &items[0])
                        }
                        _ => None,
                    };
                    match self.backend.dispatch(command) {
                        Ok(()) => {
                            self.message = None;
                            self.chrome.view = self.backend.snapshot().view;
                            self.dialogs.finish_add(ui.ctx(), Ok(()));
                            if let Some(name) = removed_name {
                                self.notify(format!("Removed {name} from the list"));
                            }
                        }
                        Err(error) => self.dialogs.finish_add(ui.ctx(), Err(error.to_string())),
                    }
                }
            }
        }
        if let Some(error) = self.confirmation.show(ui.ctx(), palette) {
            self.message = Some(error);
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

enum StartupMessage {
    Ready(settings_client::Overrides, PathBuf),
    Unavailable,
}

struct StartupApp {
    started: Instant,
    updates: Receiver<StartupMessage>,
    app: Option<DownloadsApp>,
    unavailable: bool,
    initialization_error: Option<String>,
}

impl StartupApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (sender, updates) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => {
                    let _ = sender.send(StartupMessage::Unavailable);
                    ctx.request_repaint();
                    return;
                }
            };
            let binary = std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    exe.parent().map(|dir| {
                        dir.join(if cfg!(windows) {
                            "downloader-core.exe"
                        } else {
                            "downloader-core"
                        })
                    })
                })
                .filter(|path| path.exists())
                .unwrap_or_else(|| PathBuf::from("downloader-core"));
            let spawner = downloader_ipc::DaemonAutospawner::new(binary);
            loop {
                let result = runtime
                    .block_on(spawner.ensure_running())
                    .ok()
                    .and_then(|()| {
                        Some((
                            settings_client::load().ok()?,
                            settings_client::load_directory().ok()?,
                        ))
                    });
                let ready = result.is_some();
                let message = match result {
                    Some((settings, path)) => StartupMessage::Ready(settings, path),
                    None => StartupMessage::Unavailable,
                };
                if sender.send(message).is_err() {
                    return;
                }
                ctx.request_repaint();
                if ready {
                    return;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        Self {
            started: Instant::now(),
            updates,
            app: None,
            unavailable: false,
            initialization_error: None,
        }
    }
}

impl eframe::App for StartupApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(app) = &mut self.app {
            eframe::App::raw_input_hook(app, ctx, input);
        } else if let Some(max_side) = &mut input.max_texture_side {
            *max_side = (*max_side).min(graphics::FONT_ATLAS_MAX_SIDE);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.app.is_none() {
            while let Ok(message) = self.updates.try_recv() {
                match message {
                    StartupMessage::Ready(overrides, path) => {
                        match DownloadsApp::new(ctx, overrides, path) {
                            Ok(app) => self.app = Some(app),
                            Err(error) => self.initialization_error = Some(error.to_string()),
                        }
                    }
                    StartupMessage::Unavailable => self.unavailable = true,
                }
            }
            if self.app.is_none() {
                if self.started.elapsed() >= Duration::from_secs(38) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    ctx.request_repaint_after(Duration::from_millis(250));
                }
            }
        }
        if let Some(app) = &mut self.app {
            eframe::App::logic(app, ctx, frame);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.app {
            eframe::App::ui(app, ui, frame);
            return;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(error) = &self.initialization_error {
                    ui.label(format!("Could not initialize the GUI: {error}"));
                } else if self.unavailable {
                    ui.label("Downloader daemon is not running");
                    #[cfg(not(target_os = "linux"))]
                    ui.label("The downloader daemon is currently supported on Linux only.");
                } else {
                    ui.label("Starting downloader daemon…");
                }
            });
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    eframe::run_native(
        "Downloads",
        graphics::native_options(),
        Box::new(|cc| Ok(Box::new(StartupApp::new(cc)))),
    )
}

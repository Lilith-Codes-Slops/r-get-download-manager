//! Native context menus and command dropdowns.
use crate::fake_backend::{Selection, State, View};
use crate::menu_model::{self, RowCommand};
use crate::theme::Theme;
use eframe::egui::{self, RichText};

#[derive(Clone, Debug)]
pub enum Action {
    Row(Selection, RowCommand),
    Add,
    Torrent,
    Settings,
    Search,
    PauseAll,
    ResumeAll,
    ToggleSelected,
    RemoveSelected,
    ClearCompleted,
    CopySelected,
    CutSelected,
    View(View),
    Pin,
    OpenFolder,
}

pub fn row(
    ui: &mut egui::Ui,
    state: &State,
    selected: &Selection,
    palette: Theme,
    action: &mut Option<Action>,
) {
    ui.set_min_width(210.0_f32.min((ui.ctx().content_rect().width() - 24.0).max(0.0)));
    let max_height = (ui.ctx().content_rect().height() * 0.7).max(40.0);
    egui::ScrollArea::vertical()
        .id_salt("row_menu_scroll")
        .max_height(max_height)
        .show(ui, |ui| {
            for (label, command) in [
                ("Pause", RowCommand::Pause),
                ("Resume", RowCommand::Resume),
                ("Retry", RowCommand::Retry),
                (
                    if selected.scope == crate::fake_backend::Scope::Seed
                        && state.seeds.iter().any(|s| s.id == selected.id && s.running)
                    {
                        "Stop seeding"
                    } else if selected.scope == crate::fake_backend::Scope::Seed {
                        "Start seeding"
                    } else {
                        "Start / Stop seeding"
                    },
                    RowCommand::ToggleSeed,
                ),
                ("Open Folder", RowCommand::OpenFolder),
                ("Copy Link", RowCommand::CopyLink),
                ("Copy", RowCommand::Copy),
                ("Cut", RowCommand::Cut),
                ("Properties", RowCommand::Properties),
                ("Remove from list", RowCommand::Remove),
                ("Delete from disk", RowCommand::Delete),
            ] {
                if matches!(command, RowCommand::OpenFolder | RowCommand::Remove) {
                    ui.separator();
                }
                let text = RichText::new(label).color(if command == RowCommand::Delete {
                    palette.red
                } else {
                    palette.text
                });
                if ui
                    .add_enabled(
                        menu_model::allowed(state, selected, command),
                        egui::Button::new(text).frame(false),
                    )
                    .clicked()
                {
                    *action = Some(Action::Row(selected.clone(), command));
                    ui.close();
                }
            }
        });
}

pub fn dropdown(
    ui: &mut egui::Ui,
    state: &State,
    commands: bool,
    palette: Theme,
    action: &mut Option<Action>,
) {
    ui.set_min_width(300.0_f32.min((ui.ctx().content_rect().width() - 24.0).max(0.0)));
    let max_height = (ui.ctx().content_rect().height() * 0.7).max(40.0);
    egui::ScrollArea::vertical()
        .id_salt(if commands {
            "commands_menu_scroll"
        } else {
            "help_menu_scroll"
        })
        .max_height(max_height)
        .show(ui, |ui| {
            section(ui, "General", palette);
            item(ui, "Add URL      Ctrl+N", true, Action::Add, action);
            item(
                ui,
                "Add Magnet      Ctrl+M / Ctrl+Shift+M",
                true,
                Action::Add,
                action,
            );
            item(ui, "Add torrent…", true, Action::Torrent, action);
            item(ui, "Search      Ctrl+F", true, Action::Search, action);
            item(ui, "Settings      Ctrl+,", true, Action::Settings, action);
            ui.separator();
            section(ui, "Transfers", palette);
            item(
                ui,
                "Pause / Resume      Space",
                state.selections.len() == 1,
                Action::ToggleSelected,
                action,
            );
            item(
                ui,
                "Pause All      Ctrl+P",
                !state.tasks.is_empty(),
                Action::PauseAll,
                action,
            );
            item(
                ui,
                "Resume All      Ctrl+R",
                !state.tasks.is_empty(),
                Action::ResumeAll,
                action,
            );
            item(
                ui,
                "Remove      Delete",
                !state.selections.is_empty(),
                Action::RemoveSelected,
                action,
            );
            if commands {
                item(
                    ui,
                    "Remove Completed",
                    !state.library.is_empty(),
                    Action::ClearCompleted,
                    action,
                );
                item(
                    ui,
                    "Copy      Ctrl+C",
                    !state.selections.is_empty(),
                    Action::CopySelected,
                    action,
                );
                item(
                    ui,
                    "Cut      Ctrl+X",
                    !state.selections.is_empty(),
                    Action::CutSelected,
                    action,
                );
            }
            item(
                ui,
                "Open Folder      Ctrl+Shift+Enter",
                !state.selections.is_empty(),
                Action::OpenFolder,
                action,
            );
            ui.separator();
            section(ui, "Navigation", palette);
            item(
                ui,
                "Tasks      Ctrl+1",
                true,
                Action::View(View::Tasks),
                action,
            );
            item(
                ui,
                "Downloads      Ctrl+2",
                true,
                Action::View(View::Downloads),
                action,
            );
            item(ui, "Toggle Panel      Ctrl+B", true, Action::Pin, action);
        });
}
fn section(ui: &mut egui::Ui, title: &str, palette: Theme) {
    ui.label(RichText::new(title).size(11.0).color(palette.text_muted));
}
fn item(ui: &mut egui::Ui, label: &str, enabled: bool, next: Action, action: &mut Option<Action>) {
    let (title, shortcut) = label.split_once("      ").unwrap_or((label, ""));
    let mut button = egui::Button::new(title).frame_when_inactive(false);
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    if ui.add_enabled(enabled, button).clicked() {
        *action = Some(next);
        ui.close();
    }
}

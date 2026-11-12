//! Add and Properties dialogs for the fake-backend prototype.
use crate::fake_backend::{Command, Selection, State};
use crate::theme::Theme;
use crate::{modal_style, properties};
use eframe::egui::{
    self, Align, Button, Color32, Id, Layout, RichText, ScrollArea, Stroke, TextEdit,
};
use std::path::PathBuf;

enum Dialog {
    Add {
        links: Vec<String>,
        torrent: Option<PathBuf>,
        error: Option<String>,
        focus: bool,
    },
    Properties {
        selection: Selection,
        focus: bool,
    },
    Delete {
        selection: Selection,
        name: String,
        error: Option<String>,
        focus: bool,
    },
}

#[derive(Default)]
pub struct Dialogs {
    dialog: Option<Dialog>,
    previous_focus: Option<Id>,
    add_generation: u64,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.dialog.is_some()
    }
    pub fn open_add(&mut self, ctx: &egui::Context) -> u64 {
        self.add_generation = self.add_generation.wrapping_add(1);
        self.previous_focus = ctx.memory(|memory| memory.focused());
        self.dialog = Some(Dialog::Add {
            links: vec![String::new()],
            torrent: None,
            error: None,
            focus: true,
        });
        self.add_generation
    }
    pub fn set_torrent(&mut self, generation: u64, path: PathBuf) -> bool {
        if generation == self.add_generation {
            if let Some(Dialog::Add { torrent, error, .. }) = &mut self.dialog {
                *torrent = Some(path);
                *error = None;
                return true;
            }
        }
        false
    }
    pub fn cancel_add(&mut self, ctx: &egui::Context, generation: u64) {
        if generation == self.add_generation
            && matches!(&self.dialog, Some(Dialog::Add { links, torrent: None, .. }) if links.iter().all(|link| link.trim().is_empty()))
        {
            self.close(ctx);
        }
    }
    pub fn set_add_error(&mut self, generation: u64, message: String) -> bool {
        if generation == self.add_generation {
            if let Some(Dialog::Add { error, .. }) = &mut self.dialog {
                *error = Some(message);
                return true;
            }
        }
        false
    }
    pub fn open_properties(&mut self, ctx: &egui::Context, selection: Selection) {
        self.previous_focus = ctx.memory(|memory| memory.focused());
        self.dialog = Some(Dialog::Properties {
            selection,
            focus: true,
        });
    }
    pub fn open_delete(&mut self, ctx: &egui::Context, selection: Selection, name: String) {
        self.previous_focus = ctx.memory(|memory| memory.focused());
        self.dialog = Some(Dialog::Delete {
            selection,
            name,
            error: None,
            focus: true,
        });
    }
    fn close(&mut self, ctx: &egui::Context) {
        self.dialog = None;
        if let Some(id) = self.previous_focus.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
    }
    pub fn finish_add(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        match result {
            Ok(()) => self.close(ctx),
            Err(message) => {
                if let Some(Dialog::Add { error, .. } | Dialog::Delete { error, .. }) =
                    &mut self.dialog
                {
                    *error = Some(message);
                    ctx.request_repaint();
                }
            }
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, state: &State, palette: Theme) -> Option<Command> {
        let dialog = self.dialog.as_mut()?;
        let mut command = None;
        let mut close = false;
        let (title, max_width) = match dialog {
            Dialog::Add { .. } => ("Add download", 480.0),
            Dialog::Properties { .. } => ("Properties", 420.0),
            Dialog::Delete { .. } => ("Delete from disk?", 420.0),
        };
        let is_add = matches!(dialog, Dialog::Add { .. });
        let modal = modal_style::modal("download_dialog", palette)
            .show(ctx, |ui| {
                let width = modal_style::width(ctx, max_width);
                ui.set_width(width);
                let mut contents = |ui: &mut egui::Ui| {
                        ui.label(RichText::new(title).size(16.0).strong().color(palette.text));
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(10.0);
                        match dialog {
                            Dialog::Add { links, torrent, error, focus } => {
                                ui.label(RichText::new("Links").strong().color(palette.text));
                                let mut enter = false;
                                for (index, link) in links.iter_mut().enumerate() {
                                    let response = ui.add(TextEdit::singleline(&mut *link)
                                        .id(Id::unique(("add_download_link", index)))
                                        .desired_width(f32::INFINITY)
                                        .hint_text("https://example.com/file.iso or magnet link"));
                                    response.widget_info(|| {
                                        let mut info = egui::WidgetInfo::text_edit(ui.is_enabled(), &*link, &*link, "Link");
                                        info.label = Some(format!("Link {}", index + 1)); info
                                    });
                                    if *focus && index == 0 { response.request_focus(); *focus = false; }
                                    if response.changed() { *error = None; }
                                    enter |= response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                }
                                if let Some(path) = torrent.as_ref() {
                                    let text = format!("Torrent: {}", path.display());
                                    ui.add(egui::Label::new(&text).truncate()).on_hover_text(&text);
                                }
                                if links.len() < 5 && ui.button("+ Add another link").clicked() {
                                    links.push(String::new());
                                }
                                if let Some(message) = error.as_ref() { ui.colored_label(palette.red, message); }
                                ui.add_space(10.0);
                                let mut submit = enter;
                                ui.horizontal(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.add(Button::new(RichText::new("Add").color(Color32::WHITE))
                                            .fill(palette.accent).stroke(Stroke::NONE).corner_radius(8)).clicked() { submit = true; }
                                        if ui.button("Cancel").clicked() { close = true; }
                                    });
                                });
                                if submit && !close {
                                    let values: Vec<_> = links.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned).collect();
                                    if !values.is_empty() || torrent.is_some() { command = Some(Command::AddInputs { links: values, torrent: torrent.clone() }); }
                                }
                            }
                            Dialog::Delete { selection, name, error, focus } => {
                                ui.label(format!("Permanently delete {name}?"));
                                ui.add(egui::Label::new(RichText::new("Preview mode: this only removes the item from the list. No files are deleted.").size(12.0).color(palette.text_dim)).wrap());
                                if let Some(message) = error.as_ref() { ui.colored_label(palette.red, message); }
                                ui.add_space(10.0);
                                ui.horizontal(|ui| { ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(palette.red).stroke(Stroke::NONE)).clicked() {
                                        command = Some(Command::RemoveItems(vec![selection.clone()]));
                                    }
                                    let cancel = ui.button("Cancel");
                                    if *focus { cancel.request_focus(); *focus = false; }
                                    if cancel.clicked() { close = true; }
                                }); });
                            }
                            Dialog::Properties { selection, focus } => {
                                if let Some(rows) = properties::rows(state, selection) {
                                    egui::Grid::new("property_fields").num_columns(2).spacing([14.0, 12.0])
                                        .min_col_width(0.0).max_col_width((width - 100.0).max(30.0))
                                        .show(ui, |ui| { for (label, value) in rows {
                                            ui.label(RichText::new(label).color(palette.text_muted));
                                            ui.add(egui::Label::new(RichText::new(value).color(palette.text)).wrap().selectable(true));
                                            ui.end_row();
                                        }});
                                } else { ui.label("This item is no longer available."); }
                                ui.add_space(10.0);
                                ui.horizontal(|ui| { ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let button = ui.button("Close");
                                    if *focus { button.request_focus(); *focus = false; }
                                    if button.clicked() { close = true; }
                                }); });
                            }
                        }
                    };
                if is_add {
                    contents(ui);
                } else {
                    ScrollArea::vertical()
                        .id_salt("download_dialog_scroll")
                        .min_scrolled_height(0.0)
                        .max_height(modal_style::max_height(ctx))
                        .auto_shrink([false, true])
                        .show(ui, contents);
                }
            });
        if close || modal.should_close() {
            self.close(ctx);
            command = None;
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_results_are_scoped_to_the_open_add_dialog() {
        let ctx = egui::Context::default();
        let mut dialogs = Dialogs::default();
        let first = dialogs.open_add(&ctx);
        dialogs.close(&ctx);
        assert!(!dialogs.set_torrent(first, PathBuf::from("old.torrent")));
        let second = dialogs.open_add(&ctx);
        assert!(!dialogs.set_torrent(first, PathBuf::from("old.torrent")));
        assert!(!dialogs.set_add_error(first, "old error".into()));
        dialogs.cancel_add(&ctx, first);
        assert!(dialogs.is_open());
        if let Some(Dialog::Add { links, .. }) = &mut dialogs.dialog {
            links[0] = "draft".into();
        }
        dialogs.cancel_add(&ctx, second);
        assert!(
            dialogs.is_open(),
            "cancelling a picker must not discard a draft"
        );
        assert!(dialogs.set_torrent(second, PathBuf::from("new.torrent")));
        assert!(
            matches!(dialogs.dialog, Some(Dialog::Add { torrent: Some(ref path), error: None, .. }) if path == &PathBuf::from("new.torrent"))
        );
        assert!(dialogs.set_add_error(second, "new error".into()));
        assert!(
            matches!(dialogs.dialog, Some(Dialog::Add { error: Some(ref error), .. }) if error == "new error")
        );
    }
}

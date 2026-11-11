//! Add and Properties dialogs for the fake-backend prototype.
use crate::download_link;
use crate::fake_backend::{Command, Selection, State};
use crate::icons::{self, Icon};
use crate::properties;
use crate::theme::Theme;
use eframe::egui::{
    self, Align, Button, Color32, Frame, Id, Layout, Margin, RichText, ScrollArea, Stroke, TextEdit,
};

enum Dialog {
    Add {
        link: String,
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
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.dialog.is_some()
    }
    pub fn open_add(&mut self, ctx: &egui::Context) {
        self.previous_focus = ctx.memory(|memory| memory.focused());
        self.dialog = Some(Dialog::Add {
            link: String::new(),
            error: None,
            focus: true,
        });
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
        let frame = Frame::NONE
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(14)
            .inner_margin(Margin::same(20));
        let modal = egui::Modal::new(Id::new("download_dialog"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(153))
            .show(ctx, |ui| {
                let width = (ctx.content_rect().width() - 72.0).clamp(0.0, max_width);
                ui.set_width(width);
                let max_height = (ctx.content_rect().height() * 0.82 - 40.0).max(0.0);
                ScrollArea::vertical()
                    .id_salt("download_dialog_scroll")
                    .min_scrolled_height(0.0)
                    .max_height(max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.label(RichText::new(title).size(16.0).strong().color(palette.text));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let response = ui
                                    .add(
                                        Button::new(icons::atom(
                                            Icon::Close,
                                            16.0,
                                            palette.text_dim,
                                        ))
                                        .frame(false),
                                    )
                                    .on_hover_text("Close");
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::Button,
                                        ui.is_enabled(),
                                        "Close dialog",
                                    )
                                });
                                if let Dialog::Properties { focus, .. } | Dialog::Delete { focus, .. } = dialog {
                                    if *focus {
                                        response.request_focus();
                                        *focus = false;
                                    }
                                }
                                if response.clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(12.0);
                        match dialog {
                            Dialog::Add { link, error, focus } => {
                                ui.label(RichText::new("Link").strong().color(palette.text));
                                let response = ui.add(
                                    TextEdit::singleline(link)
                                        .id(Id::new("add_download_link"))
                                        .desired_width(f32::INFINITY)
                                        .hint_text("https://example.com/file.iso"),
                                );
                                response.widget_info(|| {
                                    let mut info = egui::WidgetInfo::text_edit(
                                        ui.is_enabled(),
                                        &*link,
                                        &*link,
                                        "https://example.com/file.iso",
                                    );
                                    info.label = Some("Link".into());
                                    info
                                });
                                if *focus {
                                    response.request_focus();
                                    *focus = false;
                                }
                                if response.changed() {
                                    *error = None;
                                }
                                let enter = response.has_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                if let Some(message) = error.as_ref() {
                                    ui.colored_label(palette.red, message);
                                }
                                ui.label(
                                    RichText::new(
                                        "Sizes for new downloads are simulated in this preview.",
                                    )
                                    .size(12.0)
                                    .color(palette.text_muted),
                                );
                                ui.add_space(16.0);
                                let mut submit = enter;
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            Button::new(RichText::new("Add").color(Color32::WHITE))
                                                .fill(palette.accent)
                                                .stroke(Stroke::NONE)
                                                .corner_radius(8),
                                        )
                                        .clicked()
                                    {
                                        submit = true;
                                    }
                                    if ui.button("Cancel").clicked() {
                                        close = true;
                                    }
                                });
                                if submit && !close {
                                    match download_link::parse(link) {
                                        Ok((source, _)) => {
                                            command = Some(Command::AddDownload(source))
                                        }
                                        Err(message) => {
                                            *error = Some(message.to_owned());
                                            ctx.request_repaint();
                                        }
                                    }
                                }
                            }
                            Dialog::Delete { selection, name, error, .. } => {
                                ui.label(format!("Permanently delete {name}?"));
                                ui.add(egui::Label::new(RichText::new("Preview mode: this only removes the item from the list. No files are deleted.").size(12.0).color(palette.text_dim)).wrap());
                                if let Some(message) = error.as_ref() { ui.colored_label(palette.red, message); }
                                ui.add_space(16.0);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(palette.red).stroke(Stroke::NONE)).clicked() {
                                        command = Some(Command::RemoveItems(vec![selection.clone()]));
                                    }
                                    if ui.button("Cancel").clicked() { close = true; }
                                });
                            }
                            Dialog::Properties { selection, .. } => {
                                if let Some(rows) = properties::rows(state, selection) {
                                    egui::Grid::new("property_fields")
                                        .num_columns(2)
                                        .spacing([14.0, 12.0])
                                        .min_col_width(0.0)
                                        .max_col_width((width - 100.0).max(30.0))
                                        .show(ui, |ui| {
                                            for (label, value) in rows {
                                                ui.label(
                                                    RichText::new(label).color(palette.text_muted),
                                                );
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(value).color(palette.text),
                                                    )
                                                    .wrap()
                                                    .selectable(true),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                } else {
                                    ui.label("This item is no longer available.");
                                }
                                ui.add_space(16.0);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.button("Close").clicked() {
                                        close = true;
                                    }
                                });
                            }
                        }
                    });
            });
        if close || modal.should_close() {
            self.close(ctx);
            command = None;
        }
        command
    }
}

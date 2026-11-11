//! Downloads library and editable category chips.
use eframe::egui::{
    self, Align2, Button, Color32, FontId, Id, Layout, Margin, Pos2, Rect, RichText, ScrollArea,
    Sense, Stroke, TextEdit, UiBuilder, Vec2,
};

use crate::categories::{category_name, category_of, filtered_library};
use crate::fake_backend::{Command, LibraryItem, Scope, Selection, SelectionMode, State};
use crate::icons::{self, Icon};
use crate::row_layout::{self, LIBRARY_COLUMNS};
use crate::theme::Theme;

const TOOLBAR_HEIGHT: f32 = 62.0;

#[derive(Default)]
pub struct DownloadsView {
    dialog: Option<Dialog>,
    previous_focus: Option<Id>,
    focus_first: bool,
}

enum Dialog {
    Edit {
        id: Option<String>,
        name: String,
        extensions: String,
        error: Option<String>,
    },
    Delete {
        id: String,
        name: String,
        error: Option<String>,
    },
}

impl DownloadsView {
    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    /// Keep the dialog open on validation or persistence errors.
    pub fn complete_dialog(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        match result {
            Ok(()) => self.close_dialog(ctx),
            Err(message) => match self.dialog.as_mut() {
                Some(Dialog::Edit { error, .. } | Dialog::Delete { error, .. }) => {
                    *error = Some(message)
                }
                None => {}
            },
        }
    }

    fn close_dialog(&mut self, ctx: &egui::Context) {
        self.dialog = None;
        if let Some(id) = self.previous_focus.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &State,
        palette: Theme,
        menu_action: &mut Option<crate::menus::Action>,
    ) -> Vec<Command> {
        let mut actions = Vec::new();
        let size = ui.available_size_before_wrap().max(Vec2::ZERO);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let toolbar = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), TOOLBAR_HEIGHT.min(rect.height())),
        );
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(toolbar, 0, palette.surface);
        painter.line_segment(
            [toolbar.left_bottom(), toolbar.right_bottom()],
            Stroke::new(1.0, palette.border),
        );

        let search_width = 180.0_f32.min((toolbar.width() * 0.34).max(0.0));
        let search_right = (toolbar.right() - 14.0).max(toolbar.left());
        let search_rect = Rect::from_min_max(
            Pos2::new(
                (search_right - search_width).max(toolbar.left()),
                toolbar.center().y - 14.0,
            ),
            Pos2::new(search_right, toolbar.center().y + 14.0),
        )
        .intersect(toolbar);
        let chips_rect = Rect::from_min_max(
            Pos2::new((toolbar.left() + 18.0).min(toolbar.right()), toolbar.top()),
            Pos2::new(
                (search_rect.left() - 14.0).max((toolbar.left() + 18.0).min(toolbar.right())),
                toolbar.bottom(),
            ),
        );
        if chips_rect.is_positive() {
            let mut chips_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("category_toolbar")
                    .max_rect(chips_rect),
            );
            chips_ui.set_clip_rect(chips_rect.intersect(ui.clip_rect()));
            ScrollArea::horizontal()
                .id_salt("category_chips")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(&mut chips_ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.set_min_height(toolbar.height());
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.spacing_mut().button_padding = egui::vec2(16.0, 7.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        for (id, name) in [("all", "All"), ("other", "Uncategorized")] {
                            if chip(
                                ui,
                                name,
                                if id == "all" {
                                    Icon::Grid
                                } else {
                                    Icon::Question
                                },
                                state.selected_category == id,
                                palette,
                            )
                            .clicked()
                            {
                                actions.push(Command::SelectCategory(id.into()));
                            }
                        }
                        for category in &state.categories {
                            let response = chip(
                                ui,
                                &category.name,
                                icons::category(&category.icon),
                                state.selected_category == category.id,
                                palette,
                            );
                            if response.clicked() {
                                actions.push(Command::SelectCategory(category.id.clone()));
                            }
                            response.context_menu(|ui| {
                                if ui.button("Edit").clicked() {
                                    self.previous_focus = Some(response.id);
                                    self.focus_first = true;
                                    self.dialog = Some(Dialog::Edit {
                                        id: Some(category.id.clone()),
                                        name: category.name.clone(),
                                        extensions: category.extensions.join(", "),
                                        error: None,
                                    });
                                    ui.close();
                                }
                                if ui.button("Delete").clicked() {
                                    self.previous_focus = Some(response.id);
                                    self.focus_first = true;
                                    self.dialog = Some(Dialog::Delete {
                                        id: category.id.clone(),
                                        name: category.name.clone(),
                                        error: None,
                                    });
                                    ui.close();
                                }
                            });
                        }
                        let new_category = chip(ui, "New", Icon::Plus, false, palette);
                        if new_category.clicked() {
                            self.previous_focus = Some(new_category.id);
                            self.focus_first = true;
                            self.dialog = Some(Dialog::Edit {
                                id: None,
                                name: String::new(),
                                extensions: String::new(),
                                error: None,
                            });
                        }
                    });
                });
        }
        if search_rect.is_positive() {
            let mut search_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("library_search_ui")
                    .max_rect(search_rect),
            );
            search_ui.set_clip_rect(search_rect.intersect(ui.clip_rect()));
            let mut search = state.library_filter.clone();
            let response = search_ui.add_sized(
                search_rect.size(),
                TextEdit::singleline(&mut search)
                    .id(Id::new("library_filter"))
                    .hint_text("Search downloads…"),
            );
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::text_edit(
                    search_ui.is_enabled(),
                    &search,
                    &search,
                    "Search downloads…",
                );
                info.label = Some("Search downloads".into());
                info
            });
            if response.changed() {
                actions.push(Command::SetLibraryFilter(search));
            }
        }
        let body = Rect::from_min_max(Pos2::new(rect.left(), toolbar.bottom()), rect.max);
        if body.is_positive() {
            let mut list_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("library_list")
                    .max_rect(body)
                    .layout(Layout::top_down(egui::Align::Min)),
            );
            list_ui.set_clip_rect(body.intersect(ui.clip_rect()));
            ScrollArea::vertical()
                .id_salt("library_scroll")
                .min_scrolled_height(0.0)
                .max_height(body.height())
                .auto_shrink([false, false])
                .show(&mut list_ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let items = filtered_library(state);
                    if items.is_empty() {
                        ui.add_space(40.0);
                        let text = if state.library.is_empty() {
                            "No completed downloads yet"
                        } else {
                            "Nothing matches the filter"
                        };
                        ui.vertical_centered(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(text).color(palette.text_muted))
                                    .wrap(),
                            );
                        });
                        return;
                    }
                    let visible: Vec<_> = items
                        .iter()
                        .map(|item| Selection {
                            scope: Scope::Library,
                            id: item.id.clone(),
                        })
                        .collect();
                    for item in items {
                        library_row(
                            ui,
                            item,
                            state,
                            &visible,
                            palette,
                            &mut actions,
                            menu_action,
                        );
                    }
                });
        }
        actions
    }

    /// Render above the rail so the modal backdrop blocks the entire interface.
    pub fn show_dialog(&mut self, ctx: &egui::Context, palette: Theme) -> Option<Command> {
        let dialog = self.dialog.as_mut()?;
        let mut command = None;
        let mut close = false;
        let frame = egui::Frame::NONE
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(14)
            .inner_margin(Margin::same(20));
        let response = egui::Modal::new(Id::new("category_dialog"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(153))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 72.0).clamp(0.0, 360.0));
                let max_height = (ctx.content_rect().height() * 0.82 - 40.0).max(0.0);
                ScrollArea::vertical()
                    .id_salt("category_dialog_scroll")
                    .min_scrolled_height(0.0)
                    .max_height(max_height)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        match dialog {
                            Dialog::Edit { id, name, extensions, error } => {
                                ui.heading(if id.is_some() { "Edit Category" } else { "New Category" });
                                ui.label("Name");
                                let name_response = ui.add(TextEdit::singleline(name)
                                    .id(Id::new("category_name"))
                                    .desired_width(f32::INFINITY)
                                    .hint_text("e.g. Ebooks"));
                                name_response.widget_info(|| {
                                    let mut info = egui::WidgetInfo::text_edit(ui.is_enabled(), &*name, &*name, "e.g. Ebooks");
                                    info.label = Some("Category name".into()); info
                                });
                                if self.focus_first { name_response.request_focus(); self.focus_first = false; }
                                ui.label("File extensions");
                                let extensions_response = ui.add(TextEdit::singleline(extensions)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("epub, mobi, azw3"));
                                extensions_response.widget_info(|| {
                                    let mut info = egui::WidgetInfo::text_edit(ui.is_enabled(), &*extensions, &*extensions, "epub, mobi, azw3");
                                    info.label = Some("File extensions".into()); info
                                });
                                if let Some(error) = error.as_ref() {
                                    ui.colored_label(palette.red, error);
                                }
                                let enter = (extensions_response.has_focus() || name_response.has_focus())
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                ui.horizontal(|ui| {
                                    if ui.button("Cancel").clicked() { close = true; }
                                    let button = Button::new(if id.is_some() { "Save" } else { "Create" })
                                        .fill(palette.accent)
                                        .stroke(Stroke::new(1.0, palette.accent));
                                    if ui.add(button).clicked() || enter {
                                        let extensions = extensions
                                            .split(|ch: char| ch == ',' || ch.is_whitespace())
                                            .filter(|s| !s.is_empty())
                                            .map(str::to_owned)
                                            .collect();
                                        command = Some(if let Some(id) = id {
                                            Command::UpdateCategory { id: id.clone(), name: name.clone(), extensions }
                                        } else {
                                            Command::CreateCategory { name: name.clone(), extensions }
                                        });
                                    }
                                });
                            }
                            Dialog::Delete { id, name, error } => {
                                ui.heading("Delete category?");
                                ui.label(format!("Delete the {name} category?"));
                                ui.label(RichText::new("Its files will move to Uncategorized. No files are deleted from disk.").color(palette.text_dim));
                                if let Some(error) = error.as_ref() { ui.colored_label(palette.red, error); }
                                ui.horizontal(|ui| {
                                    let cancel = ui.button("Cancel");
                                    if self.focus_first { cancel.request_focus(); self.focus_first = false; }
                                    if cancel.clicked() { close = true; }
                                    if ui.add(Button::new("Delete").fill(palette.red)).clicked() {
                                        command = Some(Command::DeleteCategory(id.clone()));
                                    }
                                });
                            }
                        }
                    });
            });
        if close || response.should_close() {
            self.close_dialog(ctx);
            command = None;
        }
        command
    }
}

fn chip(ui: &mut egui::Ui, name: &str, icon: Icon, active: bool, palette: Theme) -> egui::Response {
    let response = ui.add(
        Button::new((
            icons::atom(
                icon,
                16.0,
                if active {
                    Color32::WHITE
                } else {
                    palette.text_dim
                },
            ),
            RichText::new(name).color(if active { Color32::WHITE } else { palette.text }),
        ))
        .fill(if active {
            palette.accent
        } else {
            palette.surface_2
        })
        .stroke(Stroke::new(
            1.0,
            if active {
                palette.accent
            } else {
                palette.border
            },
        ))
        .corner_radius(20)
        .min_size(Vec2::new(0.0, 34.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, name)
    });
    crate::theme::focus_outline(ui, &response, palette);
    response
}

fn library_row(
    ui: &mut egui::Ui,
    file: &LibraryItem,
    state: &State,
    visible: &[Selection],
    palette: Theme,
    actions: &mut Vec<Command>,
    menu_action: &mut Option<crate::menus::Action>,
) {
    let height = if state.settings.compact { 29.0 } else { 37.0 };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let selection = Selection {
        scope: Scope::Library,
        id: file.id.clone(),
    };
    let response = response.on_hover_text(&file.name);
    if response.secondary_clicked() && !state.selections.contains(&selection) {
        actions.push(Command::Select {
            item: selection.clone(),
            mode: SelectionMode::Replace,
            visible: visible.to_vec(),
        });
    }
    let menu_was_empty = menu_action.is_none();
    response.context_menu(|ui| crate::menus::row(ui, state, &selection, palette, menu_action));
    if menu_was_empty && menu_action.is_some() {
        response.request_focus();
    }
    let mut keyboard_activation = false;
    if response.has_focus() && !egui::Popup::is_any_open(ui.ctx()) {
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            keyboard_activation = true;
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            && !state.selections.is_empty()
        {
            *menu_action = Some(crate::menus::Action::RemoveSelected);
        }
    }
    let selected = state.selections.contains(&selection);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, &file.name)
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if selected {
        painter.rect_filled(rect, 0, palette.surface_3);
        painter.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, height)),
            0,
            palette.accent,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 0, palette.hover);
    }
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, palette.border),
    );
    crate::theme::focus_outline(ui, &response, palette);
    let horizontal_padding = if state.settings.compact { 12.0 } else { 14.0 };
    let columns = row_layout::columns(rect.width(), horizontal_padding, LIBRARY_COLUMNS);
    let col = |index: usize| {
        Rect::from_min_size(
            Pos2::new(rect.left() + columns[index].0, rect.top()),
            Vec2::new(columns[index].1, height),
        )
    };
    let first = col(0);
    let mut checkbox_rect = None;
    if !state.selections.is_empty() && first.width() > 0.0 {
        let bounds =
            Rect::from_center_size(first.center(), Vec2::new(22.0_f32.min(first.width()), 18.0))
                .intersect(first);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(("library_checkbox", &file.id))
                .max_rect(bounds)
                .layout(Layout::left_to_right(egui::Align::Center)),
        );
        child.set_clip_rect(bounds.intersect(ui.clip_rect()));
        let mut checked = selected;
        let checkbox = child.add(egui::Checkbox::new(&mut checked, ""));
        checkbox.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                ui.is_enabled(),
                checked,
                format!("Select {}", file.name),
            )
        });
        if checkbox.changed() {
            actions.push(Command::Select {
                item: selection.clone(),
                mode: SelectionMode::Toggle,
                visible: visible.to_vec(),
            });
        }
        checkbox_rect = Some(bounds);
    } else if state.selections.is_empty() && first.width() >= 16.0 {
        let category_id = category_of(state, &file.name);
        let icon = state
            .categories
            .iter()
            .find(|category| category.id == category_id)
            .map(|category| icons::category(&category.icon))
            .unwrap_or(Icon::Question);
        icons::paint(
            &painter,
            Rect::from_center_size(first.center(), Vec2::splat(16.0)),
            icon,
            palette.text_dim,
        );
    }
    let name_col = col(1);
    crate::row_ui::filename(
        &painter,
        name_col.intersect(rect),
        rect.center().y,
        &file.name,
        palette.text,
    );
    let values = [
        format_size(file.size_mb),
        category_name(state, category_of(state, &file.name)).to_owned(),
        file.finished.clone(),
    ];
    for (index, text) in values.into_iter().enumerate() {
        let column = col(index + 2).intersect(rect);
        if column.width() <= 0.0 {
            continue;
        }
        let right = index == 2;
        painter.with_clip_rect(column).text(
            Pos2::new(
                if right { column.right() } else { column.left() },
                rect.center().y,
            ),
            if right {
                Align2::RIGHT_CENTER
            } else {
                Align2::LEFT_CENTER
            },
            text,
            FontId::proportional(12.0),
            if right {
                palette.text_muted
            } else {
                palette.text_dim
            },
        );
    }
    let on_checkbox = ui.input(|input| {
        input
            .pointer
            .interact_pos()
            .is_some_and(|pos| checkbox_rect.is_some_and(|r| r.contains(pos)))
    });
    if response.clicked()
        && !keyboard_activation
        && (!on_checkbox || !response.clicked_by(egui::PointerButton::Primary))
    {
        let mods = ui.input(|input| input.modifiers);
        let mode = match (mods.shift, mods.command) {
            (false, false) => SelectionMode::Replace,
            (false, true) => SelectionMode::Toggle,
            (true, false) => SelectionMode::Range,
            (true, true) => SelectionMode::AddRange,
        };
        actions.push(Command::Select {
            item: selection,
            mode,
            visible: visible.to_vec(),
        });
    }
}

fn format_size(mb: f64) -> String {
    if mb >= 10_000.0 {
        format!("{:.0} GB", mb / 1000.0)
    } else if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", mb * 1000.0)
    }
}

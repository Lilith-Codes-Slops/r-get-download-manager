//! Native ribbon and status chrome for the new GUI prototype.
use eframe::egui::{
    self, scroll_area::ScrollBarVisibility, Align, Button, Color32, Frame, Id, Layout, Margin,
    Pos2, Rect, RichText, ScrollArea, Stroke, UiBuilder, Vec2,
};

use crate::icons::{self, Icon};
use crate::theme::Theme;

use crate::fake_backend::{State, ThemeChoice, View};

impl ThemeChoice {
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
        }
    }
}

pub enum ChromeAction {
    PickTorrent,
    PauseAll,
    ResumeAll,
    RetryAll,
    OpenSettings,
    OpenAdd,
    Menu(crate::menus::Action),
}

fn format_speed(kbps: f64) -> String {
    if kbps < 1000.0 {
        format!("{kbps:.0} KB/s")
    } else if kbps < 10_000.0 {
        format!("{:.1} MB/s", kbps / 1000.0)
    } else if kbps < 1_000_000.0 {
        format!("{:.0} MB/s", kbps / 1000.0)
    } else {
        format!("{:.1} GB/s", kbps / 1_000_000.0)
    }
}

/// UI-only chrome state is synchronized with the fake backend.
#[derive(Default)]
pub struct Chrome {
    pub view: View,
    pub theme: ThemeChoice,
    pub picking_torrent: bool,
    pub search_open: bool,
}

impl Chrome {
    /// Minimal title and view tabs.
    pub fn show_ribbon(
        &mut self,
        ui: &mut egui::Ui,
        palette: Theme,
        _state: &State,
    ) -> (bool, Option<ChromeAction>) {
        let mut view_clicked = false;
        egui::Panel::top("ribbon")
            .exact_size(52.0)
            .frame(
                Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(16, 0)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_min_height(52.0);
                ScrollArea::horizontal()
                    .id_salt("ribbon_scroll")
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.set_min_height(52.0);
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 6, palette.accent);
                            icons::paint(
                                ui.painter(),
                                rect.shrink(3.0),
                                Icon::Download,
                                Color32::WHITE,
                            );
                            ui.label(
                                RichText::new("Downloads")
                                    .size(14.0)
                                    .strong()
                                    .color(palette.text),
                            );
                            ui.add_space(24.0);
                            for (label, view) in
                                [("Tasks", View::Tasks), ("Downloads", View::Downloads)]
                            {
                                let active = self.view == view;
                                let mut button =
                                    Button::new(RichText::new(label).color(if active {
                                        palette.text
                                    } else {
                                        palette.text_dim
                                    }))
                                    .frame_when_inactive(active)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(8)
                                    .min_size(Vec2::new(74.0, 32.0));
                                if active {
                                    button = button.fill(palette.surface_2);
                                }
                                let response = ui.add(button);
                                crate::theme::focus_outline(ui, &response, palette);
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::Role::Button,
                                        ui.is_enabled(),
                                        active,
                                        label,
                                    )
                                });
                                if response.clicked() {
                                    self.view = view;
                                    view_clicked = true;
                                }
                            }
                        });
                    });
                ui.painter().hline(
                    (ui.max_rect().left() - 16.0)..=(ui.max_rect().right() + 16.0),
                    ui.max_rect().bottom() - 0.5,
                    Stroke::new(1.0, palette.border),
                );
            });
        (view_clicked, None)
    }

    /// The right group has its own pinned region; only the left actions scroll.
    pub fn show_actions(
        &mut self,
        ui: &mut egui::Ui,
        palette: Theme,
        state: &State,
        modal_open: bool,
    ) -> (Option<ChromeAction>, Option<String>) {
        let mut action = None;
        let mut search_update = None;
        let layout = crate::toolbar_layout::ToolbarLayout::new(
            (ui.available_width() - 32.0).max(0.0),
            state.view == View::Tasks,
        );
        if modal_open {
            self.search_open = false;
        }
        egui::Panel::top("actions")
            .exact_size(layout.height())
            .frame(Frame::NONE.fill(palette.surface).inner_margin(Margin::symmetric(16, 6)))
            .show_separator_line(false)
            .show(ui, |ui| {
                let origin = ui.max_rect().min;
                let region = |x: f32, row: usize, width: f32| Rect::from_min_size(
                    Pos2::new(origin.x + x, origin.y + crate::toolbar_layout::ToolbarLayout::row_y(row)),
                    Vec2::new(width, 36.0),
                );
                if state.view == View::Tasks && layout.actions_width > 0.0 {
                    let rect = region(0.0, layout.actions_row, layout.actions_width);
                    let mut left = ui.new_child(UiBuilder::new().id_salt("toolbar_left").max_rect(rect));
                    left.set_clip_rect(rect.intersect(ui.clip_rect()));
                    ScrollArea::horizontal()
                        .id_salt("action_scroll")
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
                        .show(&mut left, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                for (label, icon, enabled, width) in [
                                    ("+ URL", Icon::Link, true, 76.0),
                                    ("+ Magnet", Icon::Magnet, true, 96.0),
                                    ("Add torrent", Icon::FilePlus, !self.picking_torrent, 112.0),
                                    ("Pause All", Icon::Pause, !state.tasks.is_empty(), 99.0),
                                    ("Resume All", Icon::Play, !state.tasks.is_empty(), 108.0),
                                    ("Retry All", Icon::Retry, state.tasks.iter().any(|t| matches!(t.status, crate::fake_backend::TaskStatus::Failed)), 90.0),
                                    ("Remove Download History", Icon::Trash, !state.library.is_empty(), 184.0),
                                    ("Remove", Icon::Trash, !state.selections.is_empty(), 88.0),
                                    ("Remove Completed", Icon::Trash, !state.library.is_empty(), 155.0),
                                ] {
                                    let primary = label == "+ URL";
                                    let button = Button::new((
                                        icons::atom(icon, 16.0, if primary { Color32::WHITE } else { palette.text_dim }),
                                        RichText::new(label).size(12.0).color(if primary { Color32::WHITE } else { palette.text }),
                                    )).fill(if primary { palette.accent } else { palette.surface_2 })
                                      .stroke(Stroke::new(1.0, if primary { palette.accent } else { palette.border }))
                                      .corner_radius(8);
                                    let response = ui.add_enabled_ui(enabled, |ui| ui.add_sized([width, 30.0], button)).inner;
                                    let response = if label == "Remove Download History" {
                                        response.on_hover_text("Remove completed entries from the list; files are kept")
                                    } else { response };
                                    crate::theme::focus_outline(ui, &response, palette);
                                    if response.clicked() {
                                        action = match label {
                                            "+ URL" | "+ Magnet" => Some(ChromeAction::OpenAdd),
                                            "Add torrent" => Some(ChromeAction::PickTorrent),
                                            "Pause All" => Some(ChromeAction::PauseAll),
                                            "Resume All" => Some(ChromeAction::ResumeAll),
                                            "Retry All" => Some(ChromeAction::RetryAll),
                                            "Remove" => Some(ChromeAction::Menu(crate::menus::Action::RemoveSelected)),
                                            "Remove Download History" | "Remove Completed" => Some(ChromeAction::Menu(crate::menus::Action::ClearCompleted)),
                                            _ => None,
                                        };
                                    }
                                }
                            });
                        });
                }
                let search_rect = region(layout.search_x, layout.search_row, layout.search_width).shrink2(egui::vec2(0.0, 3.0));
                if search_rect.is_positive() {
                    let mut search_ui = ui.new_child(UiBuilder::new().id_salt("toolbar_search").max_rect(search_rect));
                    search_ui.set_clip_rect(search_rect.intersect(ui.clip_rect()));
                    let mut query = state.library_filter.clone();
                    let response = search_ui.add_sized(search_rect.size(), egui::TextEdit::singleline(&mut query)
                        .id(Id::unique("global_search"))
                        .hint_text("Search…"));
                    response.widget_info(|| {
                        let mut info = egui::WidgetInfo::text_edit(search_ui.is_enabled(), &query, &query, "Search…");
                        info.label = Some("Search downloads and tasks".into());
                        info
                    });
                    if response.changed() {
                        search_update = Some(query.clone());
                    }
                    if !modal_open && (response.changed() || response.clicked()) {
                        self.search_open = !query.trim().is_empty();
                    }
                    if query.trim().is_empty() { self.search_open = false; }
                    let suggestions = if self.search_open {
                        crate::search_model::suggestions(state, &query)
                    } else {
                        Vec::new()
                    };
                    let mut chosen = None;
                    let popup_width = search_rect.width().max(220.0).min((ui.ctx().content_rect().width() - 24.0).max(0.0));
                    egui::Popup::from_response(&response)
                        .open_bool(&mut self.search_open)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .gap(5.0)
                        .width(popup_width)
                        .frame(Frame::NONE.fill(palette.surface).stroke(Stroke::new(1.0, palette.border)).corner_radius(8).inner_margin(Margin::same(6)))
                        .accessible_name("Search suggestions")
                        .show(|ui| {
                            ui.set_width((popup_width - 12.0).max(0.0));
                            if suggestions.is_empty() {
                                ui.label(RichText::new("No matching items").size(12.0).color(palette.text_muted));
                            } else {
                                let max_height = (ui.ctx().content_rect().height() * 0.5).clamp(30.0, 180.0);
                                ScrollArea::vertical().id_salt("search_suggestions").max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    for name in suggestions {
                                        let item = ui.add_sized([ui.available_width(), 28.0], Button::new(RichText::new(name).size(12.0).color(palette.text)).truncate().frame(false));
                                        if item.clicked() { chosen = Some(name.to_owned()); }
                                    }
                                });
                            }
                        });
                    if let Some(name) = chosen {
                        search_update = Some(name);
                        self.search_open = false;
                        response.request_focus();
                    }
                }
                let right_rect = region(layout.right_x, 0, layout.right_width);
                if right_rect.is_positive() {
                    let mut right = ui.new_child(UiBuilder::new().id_salt("toolbar_right").max_rect(right_rect));
                    right.set_clip_rect(right_rect.intersect(ui.clip_rect()));
                    right.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().button_padding = egui::vec2(7.0, 5.0);
                        for (label, icon, width) in [("Settings", Icon::Settings, 94.0), ("Help", Icon::Help, 70.0), ("Commands", Icon::Command, 104.0)] {
                            let button = if layout.compact_right {
                                Button::new(icons::atom(icon, 16.0, palette.text_dim))
                            } else {
                                Button::new((icons::atom(icon, 16.0, palette.text_dim), RichText::new(label).size(12.0).color(palette.text)))
                            }.fill(palette.surface_2).stroke(Stroke::new(1.0, palette.border)).corner_radius(8);
                            let response = ui.add_sized([if layout.compact_right { 36.0 } else { width }, 30.0], button).on_hover_text(label);
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
                            crate::theme::focus_outline(ui, &response, palette);
                            if label == "Settings" {
                                if response.clicked() { action = Some(ChromeAction::OpenSettings); }
                            } else {
                                let mut selected = None;
                                egui::Popup::menu(&response).show(|ui| crate::menus::dropdown(ui, state, label == "Commands", palette, &mut selected));
                                if let Some(selected) = selected {
                                    response.request_focus();
                                    action = Some(ChromeAction::Menu(selected));
                                }
                            }
                        }
                    });
                }
                ui.painter().hline((ui.max_rect().left() - 16.0)..=(ui.max_rect().right() + 16.0), ui.max_rect().bottom() + 5.5, Stroke::new(1.0, palette.border));
            });
        (action, search_update)
    }

    pub fn show_status(&self, ui: &mut egui::Ui, palette: Theme, state: &State) {
        egui::Panel::bottom("status")
            .exact_size(40.0)
            .frame(
                Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(20, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_min_height(28.0);
                ui.painter().hline(
                    (ui.max_rect().left() - 20.0)..=(ui.max_rect().right() + 20.0),
                    ui.max_rect().top() - 5.5,
                    Stroke::new(1.0, palette.border),
                );
                ScrollArea::horizontal()
                    .id_salt("status_scroll")
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_height(28.0);
                            ui.spacing_mut().item_spacing.x = 28.0;
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            let down: f64 = if state.daemon_connected {
                                state.tasks.iter().map(|task| task.speed_kbps).sum()
                            } else {
                                0.0
                            };
                            let up: f64 = if state.daemon_connected {
                                state
                                    .seeds
                                    .iter()
                                    .filter(|seed| seed.running)
                                    .map(|seed| seed.speed_kbps)
                                    .sum()
                            } else {
                                0.0
                            };
                            let limit = state
                                .settings
                                .speed_limit_kbps
                                .map(format_speed)
                                .unwrap_or_else(|| "Unlimited".into());
                            for (icon, value) in [
                                (Some(Icon::Download), format_speed(down)),
                                (Some(Icon::Upload), format_speed(up)),
                                (None, format!("Completed: {}", state.library.len())),
                                (
                                    None,
                                    format!(
                                        "Seeding: {}",
                                        state
                                            .seeds
                                            .iter()
                                            .filter(|s| s.running && state.daemon_connected)
                                            .count()
                                    ),
                                ),
                                (None, format!("Active: {}", state.tasks.len())),
                                (None, format!("Speed limit: {limit}")),
                            ] {
                                ui.horizontal_centered(|ui| {
                                    ui.spacing_mut().item_spacing.x = 5.0;
                                    if let Some(icon) = icon {
                                        let (rect, _) = ui.allocate_exact_size(
                                            Vec2::splat(16.0),
                                            egui::Sense::hover(),
                                        );
                                        icons::paint(ui.painter(), rect, icon, palette.text_dim);
                                    }
                                    ui.label(
                                        RichText::new(value).size(12.0).color(palette.text_dim),
                                    );
                                });
                            }
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_toggle_can_return_to_system() {
        assert_eq!(
            ThemeChoice::System.next().next().next(),
            ThemeChoice::System
        );
    }
}

//! Native ribbon and status chrome for the new GUI prototype.
use eframe::egui::{
    self, scroll_area::ScrollBarVisibility, Align, Button, Color32, Frame, Layout, Margin,
    RichText, ScrollArea, Stroke, Vec2,
};

use crate::icons::{self, Icon};
use crate::theme::Theme;

use crate::fake_backend::{State, ThemeChoice, View};

impl ThemeChoice {
    fn next(self) -> Self {
        match self {
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::System,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "Theme: System",
            Self::Dark => "Theme: Dark",
            Self::Light => "Theme: Light",
        }
    }

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
    ToggleDaemon,
    PauseAll,
    ResumeAll,
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
}

impl Chrome {
    /// Reference-style ribbon; Help/Commands remain available in both views.
    pub fn show_ribbon(
        &mut self,
        ui: &mut egui::Ui,
        palette: Theme,
        state: &State,
    ) -> (bool, Option<ChromeAction>) {
        let mut view_clicked = false;
        let mut action = None;
        let width = ui.available_width();
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
                            ui.add_space((width - 1040.0).max(12.0));
                            let (dot, status) = if state.daemon_connected {
                                (palette.green, "Daemon connected")
                            } else {
                                (palette.red, "Daemon disconnected")
                            };
                            if ui
                                .add(
                                    Button::new(
                                        RichText::new(format!("● {status}")).color(dot).size(12.0),
                                    )
                                    .frame_when_inactive(false),
                                )
                                .on_hover_text("Toggle simulated daemon connection")
                                .clicked()
                            {
                                action = Some(ChromeAction::ToggleDaemon);
                            }
                            if ui
                                .add(
                                    Button::new(RichText::new("Settings").color(palette.text_dim))
                                        .frame_when_inactive(false),
                                )
                                .clicked()
                            {
                                action = Some(ChromeAction::OpenSettings);
                            }
                            if ui
                                .add(
                                    Button::new((
                                        icons::atom(Icon::Theme, 16.0, palette.text_dim),
                                        self.theme.label(),
                                    ))
                                    .frame_when_inactive(false),
                                )
                                .on_hover_text("Cycle between System, Dark and Light")
                                .clicked()
                            {
                                self.theme = self.theme.next();
                                crate::theme::apply_theme(ui.ctx(), self.theme.preference());
                            }
                            for (label, commands) in [("Help", false), ("Commands", true)] {
                                let response = ui.add(
                                    Button::new(RichText::new(label).color(palette.text_dim))
                                        .frame_when_inactive(false),
                                );
                                crate::theme::focus_outline(ui, &response, palette);
                                let mut selected = None;
                                egui::Popup::menu(&response).show(|ui| {
                                    crate::menus::dropdown(
                                        ui,
                                        state,
                                        commands,
                                        palette,
                                        &mut selected,
                                    )
                                });
                                if let Some(selected) = selected {
                                    response.request_focus();
                                    action = Some(ChromeAction::Menu(selected));
                                }
                            }
                            for (icon, tooltip) in [
                                (Icon::Minimize, "Minimize"),
                                (Icon::Maximize, "Maximize"),
                                (Icon::Close, "Close"),
                            ] {
                                ui.add_enabled(
                                    false,
                                    Button::new(icons::atom(icon, 16.0, palette.text_muted))
                                        .frame(false),
                                )
                                .on_hover_text(format!(
                                    "{tooltip}: use the system window controls"
                                ));
                            }
                        });
                    });
                ui.painter().hline(
                    (ui.max_rect().left() - 16.0)..=(ui.max_rect().right() + 16.0),
                    ui.max_rect().bottom() - 0.5,
                    Stroke::new(1.0, palette.border),
                );
            });
        (view_clicked, action)
    }

    pub fn show_actions(
        &mut self,
        ui: &mut egui::Ui,
        palette: Theme,
        state: &State,
    ) -> Option<ChromeAction> {
        if state.view != View::Tasks {
            return None;
        }
        let mut action = None;
        egui::Panel::top("actions")
            .exact_size(48.0)
            .frame(
                Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(16, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_min_height(36.0);
                ScrollArea::horizontal()
                    .id_salt("action_scroll")
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.set_min_height(36.0);
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.spacing_mut().button_padding = egui::vec2(14.0, 7.0);
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            for (label, icon, enabled) in [
                                ("+ URL", Icon::Link, true),
                                ("+ Magnet", Icon::Magnet, true),
                                ("Add torrent", Icon::FilePlus, !self.picking_torrent),
                                ("Pause All", Icon::Pause, !state.tasks.is_empty()),
                                ("Resume All", Icon::Play, !state.tasks.is_empty()),
                                ("Remove", Icon::Trash, !state.selections.is_empty()),
                                ("Remove Completed", Icon::Trash, !state.library.is_empty()),
                                ("Settings", Icon::Settings, true),
                            ] {
                                let primary = label == "+ URL";
                                let mut button = Button::new((
                                    icons::atom(
                                        icon,
                                        16.0,
                                        if primary {
                                            Color32::WHITE
                                        } else {
                                            palette.text_dim
                                        },
                                    ),
                                    RichText::new(label).color(if primary {
                                        Color32::WHITE
                                    } else {
                                        palette.text
                                    }),
                                ))
                                .stroke(Stroke::new(
                                    1.0,
                                    if primary {
                                        palette.accent
                                    } else {
                                        palette.border
                                    },
                                ))
                                .corner_radius(8)
                                .min_size(Vec2::new(0.0, 32.0));
                                if primary {
                                    button = button.fill(palette.accent);
                                }
                                let response = ui.add_enabled(enabled, button);
                                crate::theme::focus_outline(ui, &response, palette);
                                if response.clicked() {
                                    action = match label {
                                        "+ URL" | "+ Magnet" => Some(ChromeAction::OpenAdd),
                                        "Add torrent" => Some(ChromeAction::PickTorrent),
                                        "Pause All" => Some(ChromeAction::PauseAll),
                                        "Resume All" => Some(ChromeAction::ResumeAll),
                                        "Remove" => Some(ChromeAction::Menu(
                                            crate::menus::Action::RemoveSelected,
                                        )),
                                        "Settings" => Some(ChromeAction::OpenSettings),
                                        "Remove Completed" => Some(ChromeAction::Menu(
                                            crate::menus::Action::ClearCompleted,
                                        )),
                                        _ => None,
                                    };
                                }
                            }
                            let more = ui
                                .add_enabled(
                                    !state.selections.is_empty(),
                                    Button::new("···")
                                        .fill(palette.surface_2)
                                        .stroke(Stroke::new(1.0, palette.border))
                                        .corner_radius(8),
                                )
                                .on_hover_text("More actions for selected download");
                            crate::theme::focus_outline(ui, &more, palette);
                            more.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    ui.is_enabled() && !state.selections.is_empty(),
                                    "More actions for selected download",
                                )
                            });
                            let mut selected_action = None;
                            if let Some(item) = state.selections.first() {
                                egui::Popup::menu(&more).show(|ui| {
                                    crate::menus::row(
                                        ui,
                                        state,
                                        item,
                                        palette,
                                        &mut selected_action,
                                    )
                                });
                            }
                            if let Some(selected) = selected_action {
                                more.request_focus();
                                action = Some(ChromeAction::Menu(selected));
                            }
                            for (label, icon, commands) in [
                                ("Help", Icon::Help, false),
                                ("Commands", Icon::Command, true),
                            ] {
                                let response = ui.add(
                                    Button::new((icons::atom(icon, 16.0, palette.text_dim), label))
                                        .stroke(Stroke::new(1.0, palette.border))
                                        .corner_radius(8),
                                );
                                crate::theme::focus_outline(ui, &response, palette);
                                let mut selected = None;
                                egui::Popup::menu(&response).show(|ui| {
                                    crate::menus::dropdown(
                                        ui,
                                        state,
                                        commands,
                                        palette,
                                        &mut selected,
                                    )
                                });
                                if let Some(selected) = selected {
                                    response.request_focus();
                                    action = Some(ChromeAction::Menu(selected));
                                }
                            }
                        });
                    });
                ui.painter().hline(
                    (ui.max_rect().left() - 16.0)..=(ui.max_rect().right() + 16.0),
                    ui.max_rect().bottom() + 5.5,
                    Stroke::new(1.0, palette.border),
                );
            });
        action
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

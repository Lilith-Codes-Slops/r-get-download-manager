//! The native settings modal, backed exclusively by the persisted fake state.
use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Id, Layout, Margin, RichText, ScrollArea, Sense,
    Stroke, UiBuilder, Vec2,
};

use crate::fake_backend::{Settings, ThemeChoice};
use crate::modal_style;
use crate::settings_layout::{RowLayout, CONTROL_MAX_WIDTH, SPEED_STACK_MIN_WIDTH};
use crate::settings_logic::{format_speed, limit_to_slider, parse_speed, slider_to_limit};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Appearance,
    Downloads,
    Connections,
    Network,
    Notifications,
    Advanced,
}
impl Tab {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Appearance, "Appearance"),
        (Self::Downloads, "Downloads"),
        (Self::Connections, "Connections"),
        (Self::Network, "Network"),
        (Self::Notifications, "Notifications"),
        (Self::Advanced, "Advanced"),
    ];
}

pub struct SettingsModal {
    open: bool,
    tab: Tab,
    focus_first: bool,
    previous_focus: Option<Id>,
    speed_text: String,
    speed_dirty: bool,
    error: Option<String>,
}

impl Default for SettingsModal {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Appearance,
            focus_first: false,
            previous_focus: None,
            speed_text: String::new(),
            speed_dirty: false,
            error: None,
        }
    }
}

impl SettingsModal {
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn open(&mut self, ctx: &egui::Context, settings: &Settings) {
        self.previous_focus = ctx.memory(|memory| memory.focused());
        self.open = true;
        self.tab = Tab::Appearance;
        self.focus_first = true;
        self.speed_text = format_speed(settings.speed_limit_kbps);
        self.speed_dirty = false;
        self.error = None;
    }
    pub fn save_error(&mut self, message: String, settings: &Settings) {
        self.open = true;
        self.focus_first = true;
        self.error = Some(format!("Could not save settings: {message}"));
        self.speed_text = format_speed(settings.speed_limit_kbps);
        self.speed_dirty = false;
    }
    fn commit_speed(&mut self, settings: &mut Settings) {
        if !self.speed_dirty {
            return;
        }
        match parse_speed(&self.speed_text) {
            Some(limit) => {
                settings.speed_limit_kbps = limit;
                self.error = None;
            }
            None => {
                self.error =
                    Some("Use 10 KB/s to 1 GB/s, e.g. 500 KB/s or 2 MB/s, or Unlimited".into());
            }
        }
        self.speed_text = format_speed(settings.speed_limit_kbps);
        self.speed_dirty = false;
    }

    /// Return only changed settings; the caller persists the transaction before
    /// publishing a theme change, so a failed save cannot leave a false state.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        current: &Settings,
        palette: Theme,
    ) -> Option<Settings> {
        if !self.open {
            return None;
        }
        let mut settings = current.clone();
        let mut close = false;
        let response = modal_style::modal("settings_modal", palette).show(ctx, |ui| {
            let width = modal_style::width(ctx, 600.0);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                RichText::new("Settings")
                    .size(17.0)
                    .strong()
                    .color(palette.text),
            );
            ui.add_space(12.0);
            self.tabs(ui, palette, &mut settings);
            ui.add_space(6.0);
            divider(ui, palette);
            let body_height = (modal_style::max_height(ctx) - 120.0).max(40.0);
            ScrollArea::vertical()
                .id_salt(("settings_body", self.tab as u8))
                .min_scrolled_height(0.0)
                .max_height(body_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match self.tab {
                        Tab::Appearance => {
                            setting_row(
                                ui,
                                palette,
                                "Theme",
                                "Choose a theme or follow your system",
                                false,
                                false,
                                |ui| {
                                    theme_control(ui, palette, &mut settings.theme);
                                },
                            );
                            setting_row(
                                ui,
                                palette,
                                "Compact mode",
                                "Tighter rows",
                                false,
                                false,
                                |ui| {
                                    switch(ui, palette, &mut settings.compact, "Compact mode");
                                },
                            );
                        }
                        Tab::Downloads => {
                            setting_row(
                                ui,
                                palette,
                                "Max concurrent",
                                "Downloads running at once (1–10)",
                                false,
                                false,
                                |ui| {
                                    let response = ui.add(
                                        egui::DragValue::new(&mut settings.max_concurrent)
                                            .range(1..=10)
                                            .speed(1.0)
                                            .update_while_editing(false),
                                    );
                                    response.widget_info(|| {
                                        let mut info = egui::WidgetInfo::drag_value(
                                            ui.is_enabled(),
                                            settings.max_concurrent as f64,
                                        );
                                        info.label = Some("Max concurrent downloads".into());
                                        info
                                    });
                                },
                            );
                            setting_row(
                                ui,
                                palette,
                                "Download speed limit",
                                "Shared across running downloads · 10 KB/s to 1 GB/s",
                                false,
                                true,
                                |ui| self.speed_control(ui, palette, &mut settings),
                            );
                        }
                        Tab::Connections => {
                            setting_row(
                                ui,
                                palette,
                                "Seed until ratio",
                                "Stop seeding when this ratio is reached",
                                false,
                                false,
                                |ui| {
                                    let response = ui.add(
                                        egui::DragValue::new(&mut settings.seed_ratio)
                                            .range(0.1..=f64::MAX)
                                            .speed(0.1)
                                            .fixed_decimals(1)
                                            .update_while_editing(false),
                                    );
                                    response.widget_info(|| {
                                        let mut info = egui::WidgetInfo::drag_value(
                                            ui.is_enabled(),
                                            settings.seed_ratio,
                                        );
                                        info.label = Some("Seeding ratio".into());
                                        info
                                    });
                                },
                            );
                        }
                        Tab::Network => {
                            setting_row(
                                ui,
                                palette,
                                "Download speed limit",
                                "Shared across running downloads · 10 KB/s to 1 GB/s",
                                false,
                                true,
                                |ui| self.speed_control(ui, palette, &mut settings),
                            );
                        }
                        Tab::Notifications => {
                            for label in ["Sound on completion", "Sound on failure"] {
                                setting_row(
                                    ui,
                                    palette,
                                    label,
                                    "Unavailable in this prototype",
                                    true,
                                    false,
                                    |ui| {
                                        ui.add_enabled_ui(false, |ui| {
                                            let mut off = false;
                                            switch(ui, palette, &mut off, label);
                                        });
                                    },
                                );
                            }
                            setting_row(
                                ui,
                                palette,
                                "Show alerts in app",
                                "Messages when downloads finish or fail",
                                false,
                                false,
                                |ui| {
                                    switch(ui, palette, &mut settings.alerts, "Show alerts in app");
                                },
                            );
                        }
                        Tab::Advanced => {
                            setting_row(
                                ui,
                                palette,
                                "Confirm before delete",
                                "Ask before deleting files from disk",
                                false,
                                false,
                                |ui| {
                                    switch(
                                        ui,
                                        palette,
                                        &mut settings.confirm_delete,
                                        "Confirm before delete",
                                    );
                                },
                            );
                        }
                    }
                    if let Some(error) = &self.error {
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(RichText::new(error).size(12.0).color(palette.red))
                                .wrap(),
                        );
                    }
                });
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 32.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    if ui
                        .add_sized(
                            [76.0, 30.0],
                            Button::new(RichText::new("Done").strong().color(palette.text))
                                .fill(palette.surface_2)
                                .stroke(Stroke::new(1.0, palette.border))
                                .corner_radius(8),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                },
            );
        });
        if close || response.should_close() {
            self.commit_speed(&mut settings);
            self.open = false;
            if let Some(id) = self.previous_focus.take() {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
        }
        if settings != *current {
            Some(settings)
        } else {
            None
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui, palette: Theme, settings: &mut Settings) {
        ScrollArea::horizontal()
            .id_salt("settings_tabs")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
                    for (tab, label) in Tab::ALL {
                        let selected = self.tab == tab;
                        let text = RichText::new(label).size(12.0).color(if selected {
                            palette.text
                        } else {
                            palette.text_dim
                        });
                        let response = ui.add(
                            Button::new(if selected { text.strong() } else { text })
                                .fill(if selected {
                                    palette.surface_3
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE)
                                .corner_radius(7)
                                .min_size(Vec2::new(0.0, 32.0)),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                ui.is_enabled(),
                                selected,
                                label,
                            )
                        });
                        if self.focus_first && tab == Tab::Appearance {
                            response.request_focus();
                        }
                        crate::theme::focus_outline(ui, &response, palette);
                        if selected {
                            ui.painter().line_segment(
                                [
                                    egui::pos2(
                                        response.rect.left() + 10.0,
                                        response.rect.bottom() - 2.0,
                                    ),
                                    egui::pos2(
                                        response.rect.right() - 10.0,
                                        response.rect.bottom() - 2.0,
                                    ),
                                ],
                                Stroke::new(2.0, palette.accent),
                            );
                        }
                        if response.clicked() && !selected {
                            self.commit_speed(settings);
                            self.tab = tab;
                            response.scroll_to_me(Some(Align::Center));
                        }
                    }
                });
            });
        self.focus_first = false;
    }

    fn speed_control(&mut self, ui: &mut egui::Ui, palette: Theme, settings: &mut Settings) {
        let mut slider = limit_to_slider(settings.speed_limit_kbps);
        let width = ui.available_width().clamp(0.0, CONTROL_MAX_WIDTH);
        let stacked = width < SPEED_STACK_MIN_WIDTH;
        ui.allocate_ui_with_layout(
            Vec2::new(width, if stacked { 64.0 } else { 32.0 }),
            if stacked {
                Layout::top_down(Align::RIGHT)
            } else {
                Layout::left_to_right(Align::Center)
            },
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                ui.spacing_mut().slider_width = if stacked {
                    (width - 4.0).max(1.0)
                } else {
                    (width - 116.0).max(1.0)
                };
                ui.spacing_mut().slider_rail_height = 4.0;
                let response = ui.add(
                    egui::Slider::new(&mut slider, 0..=100)
                        .show_value(false)
                        .trailing_fill(true)
                        .handle_shape(egui::style::HandleShape::Circle),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::slider(ui.is_enabled(), slider as f64, "Speed limit slider")
                });
                if response.changed() {
                    settings.speed_limit_kbps = slider_to_limit(slider);
                    self.speed_text = format_speed(settings.speed_limit_kbps);
                    self.speed_dirty = false;
                    self.error = None;
                }
                Frame::NONE
                    .fill(palette.surface_2)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        let response = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.speed_text)
                                    .desired_width((width - 18.0).clamp(0.0, 88.0))
                                    .frame(false)
                                    .hint_text("Unlimited"),
                            )
                            .on_hover_text("e.g. 500 KB/s, 2 MB/s, or Unlimited");
                        response.widget_info(|| {
                            let mut info = egui::WidgetInfo::text_edit(
                                ui.is_enabled(),
                                &self.speed_text,
                                &self.speed_text,
                                "Unlimited",
                            );
                            info.label = Some("Speed limit value".into());
                            info
                        });
                        crate::theme::focus_outline(ui, &response, palette);
                        if response.changed() {
                            self.speed_dirty = true;
                        }
                        if response.lost_focus()
                            || (response.has_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        {
                            self.commit_speed(settings);
                            if response.has_focus() {
                                response.surrender_focus();
                            }
                        }
                    });
            },
        );
    }
}

fn divider(ui: &mut egui::Ui, palette: Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, palette.border),
    );
}

fn theme_control(ui: &mut egui::Ui, palette: Theme, theme: &mut ThemeChoice) {
    let width = ui.available_width().clamp(0.0, 156.0);
    let segment_width = ((width - 6.0) / 3.0).max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, 30.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            Frame::NONE
                .fill(palette.surface_2)
                .corner_radius(8)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(segment_width * 3.0, 24.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.spacing_mut().button_padding.x = 2.0;
                            for (choice, label) in [
                                (ThemeChoice::Light, "Light"),
                                (ThemeChoice::System, "System"),
                                (ThemeChoice::Dark, "Dark"),
                            ] {
                                let selected = *theme == choice;
                                let response = ui.add_sized(
                                    [segment_width, 24.0],
                                    Button::new(RichText::new(label).size(12.0).color(
                                        if selected {
                                            Color32::WHITE
                                        } else {
                                            palette.text_dim
                                        },
                                    ))
                                    .fill(if selected {
                                        palette.accent
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(Stroke::NONE)
                                    .corner_radius(6),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::Role::RadioButton,
                                        ui.is_enabled(),
                                        selected,
                                        label,
                                    )
                                });
                                crate::theme::focus_outline(ui, &response, palette);
                                if response.clicked() {
                                    *theme = choice;
                                }
                            }
                        },
                    );
                });
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn setting_row(
    ui: &mut egui::Ui,
    palette: Theme,
    title: &str,
    description: &str,
    disabled: bool,
    speed: bool,
    control: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width().max(0.0);
    let row = RowLayout::new(width, speed);
    let label_width = row.label_width;
    let control_width = row.control_width;
    let control_height = row.control_height;
    let stacked = row.stacked;
    let title_color = if disabled {
        palette.text_muted
    } else {
        palette.text
    };
    let description_color = palette.text_muted;
    let title_galley = ui.painter().layout(
        title.to_owned(),
        FontId::proportional(13.0),
        title_color,
        label_width.max(1.0),
    );
    let description_galley = ui.painter().layout(
        description.to_owned(),
        FontId::proportional(12.0),
        description_color,
        label_width.max(1.0),
    );
    let text_height = title_galley.size().y + 2.0 + description_galley.size().y;
    let inner_height = row.inner_height(text_height);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, inner_height + 24.0), Sense::hover());
    let text_top = if stacked {
        rect.top() + 12.0
    } else {
        rect.center().y - text_height / 2.0
    };
    let text_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), text_top),
        Vec2::new(label_width, text_height + 2.0),
    );
    let mut labels = ui.new_child(
        UiBuilder::new()
            .id_salt(("setting_labels", title))
            .max_rect(text_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    labels.set_clip_rect(text_rect.intersect(ui.clip_rect()));
    labels.spacing_mut().item_spacing.y = 2.0;
    labels
        .add(egui::Label::new(RichText::new(title).size(13.0).strong().color(title_color)).wrap());
    labels.add(
        egui::Label::new(
            RichText::new(description)
                .size(12.0)
                .color(description_color),
        )
        .wrap(),
    );
    let control_top = if stacked {
        text_top + text_height + 8.0
    } else {
        rect.center().y - control_height / 2.0
    };
    let control_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - control_width, control_top),
        Vec2::new(control_width, control_height),
    );
    let mut control_ui = ui.new_child(
        UiBuilder::new()
            .id_salt(("setting_control", title))
            .max_rect(control_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    control_ui.set_clip_rect(control_rect.intersect(ui.clip_rect()));
    control(&mut control_ui);
    divider(ui, palette);
}

fn switch(ui: &mut egui::Ui, palette: Theme, value: &mut bool, label: &str) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *value, label)
    });
    let enabled = ui.is_enabled();
    ui.painter().rect_filled(
        rect,
        12,
        if !enabled {
            palette.surface_3
        } else if *value {
            palette.accent
        } else {
            palette.track
        },
    );
    if !enabled {
        ui.painter().rect_stroke(
            rect,
            12,
            Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
    }
    crate::theme::focus_outline(ui, &response, palette);
    ui.painter().circle_filled(
        egui::pos2(
            if *value {
                rect.right() - 12.0
            } else {
                rect.left() + 12.0
            },
            rect.center().y,
        ),
        8.0,
        if enabled {
            Color32::WHITE
        } else {
            palette.text_muted
        },
    );
}

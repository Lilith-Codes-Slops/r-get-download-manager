//! The native settings modal, backed exclusively by the persisted fake state.
use eframe::egui::{
    self, Align, Button, Color32, Frame, Id, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    Vec2,
};

use crate::fake_backend::{Settings, ThemeChoice};
use crate::icons::{self, Icon};
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
        let frame = Frame::NONE
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(14)
            .inner_margin(Margin::same(20));
        let response = egui::Modal::new(Id::new("settings_modal")).frame(frame)
            .backdrop_color(Color32::from_black_alpha(153))
            .show(ctx, |ui| {
                let width = (ctx.content_rect().width() - 72.0).clamp(0.0, 600.0);
                ui.set_width(width);
                let max_height = (ctx.content_rect().height() * 0.82 - 40.0).max(0.0);
                ScrollArea::vertical().id_salt("settings_scroll").min_scrolled_height(0.0)
                    .max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.label(RichText::new("Settings").size(16.0).strong().color(palette.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let response = ui.add(Button::new(icons::atom(Icon::Close, 16.0, palette.text_dim)).frame(false)).on_hover_text("Close settings");
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Close settings"));
                            if response.clicked() { close = true; }
                        });
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    ScrollArea::horizontal().id_salt("settings_tabs").auto_shrink([false, true])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (tab, label) in Tab::ALL {
                                    let selected = self.tab == tab;
                                    let response = ui.add(Button::new(RichText::new(label).color(if selected { palette.text } else { palette.text_dim }))
                                        .fill(Color32::TRANSPARENT).stroke(Stroke::NONE).corner_radius(0).min_size(Vec2::new(0.0, 38.0)));
                                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label));
                                    if self.focus_first && tab == Tab::Appearance { response.request_focus(); }
                                    if selected { ui.painter().hline(response.rect.x_range(), response.rect.bottom() - 1.0, Stroke::new(2.0, palette.accent)); }
                                    if response.clicked() && !selected { self.commit_speed(&mut settings); self.tab = tab; }
                                }
                            });
                        });
                    self.focus_first = false;
                    ui.separator();
                    ui.add_space(8.0);
                    match self.tab {
                        Tab::Appearance => {
                            setting_row(ui, palette, "Theme", "Light, Dark or System (follows your OS)", |ui| {
                                Frame::NONE.fill(palette.surface_2).corner_radius(8).inner_margin(Margin::same(3)).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        for (choice, label) in [(ThemeChoice::Light, "Light"), (ThemeChoice::System, "System"), (ThemeChoice::Dark, "Dark")] {
                                            let selected = settings.theme == choice;
                                            let response = ui.add(Button::new(RichText::new(label).color(if selected { Color32::WHITE } else { palette.text_dim }))
                                                .fill(if selected { palette.accent } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).corner_radius(6));
                                            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, label));
                                            if response.clicked() { settings.theme = choice; }
                                        }
                                    });
                                });
                            });
                            setting_row(ui, palette, "Compact mode", "Tighter rows", |ui| { switch(ui, palette, &mut settings.compact, "Compact mode"); });
                        }
                        Tab::Downloads => {
                            setting_row(ui, palette, "Max concurrent", "Downloads running at once (1–10)", |ui| {
                                let response = ui.add(egui::DragValue::new(&mut settings.max_concurrent).range(1..=10).speed(1.0).update_while_editing(false));
                                response.widget_info(|| { let mut info = egui::WidgetInfo::drag_value(ui.is_enabled(), settings.max_concurrent as f64); info.label = Some("Max concurrent downloads".into()); info });
                            });
                            setting_row(ui, palette, "Download speed limit", "10 KB/s to 1 GB/s, or Unlimited. Shared across running downloads.", |ui| self.speed_control(ui, palette, &mut settings));
                        }
                        Tab::Connections => {
                            setting_row(ui, palette, "Seed until ratio", "Seeding stops when upload ÷ size reaches this value", |ui| {
                                let response = ui.add(egui::DragValue::new(&mut settings.seed_ratio).range(0.1..=f64::MAX).speed(0.1).fixed_decimals(1).update_while_editing(false));
                                response.widget_info(|| { let mut info = egui::WidgetInfo::drag_value(ui.is_enabled(), settings.seed_ratio); info.label = Some("Seeding ratio".into()); info });
                            });
                        }
                        Tab::Network => {
                            setting_row(ui, palette, "Download speed limit", "10 KB/s to 1 GB/s, or Unlimited. Shared across running downloads.", |ui| self.speed_control(ui, palette, &mut settings));
                        }
                        Tab::Notifications => {
                            for label in ["Sound on completion", "Sound on failure"] {
                                setting_row(ui, palette, label, "Sound is unavailable in this prototype", |ui| {
                                    ui.add_enabled_ui(false, |ui| { let mut off = false; switch(ui, palette, &mut off, label); });
                                });
                            }
                            setting_row(ui, palette, "Show alerts in app", "Messages when downloads finish or fail", |ui| { switch(ui, palette, &mut settings.alerts, "Show alerts in app"); });
                        }
                        Tab::Advanced => {
                            setting_row(ui, palette, "Confirm before delete", "Ask before deleting files from disk", |ui| { switch(ui, palette, &mut settings.confirm_delete, "Confirm before delete"); });
                        }
                    }
                    if let Some(error) = &self.error {
                        ui.add_space(8.0);
                        ui.colored_label(palette.red, error);
                    }
                });
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

    fn speed_control(&mut self, ui: &mut egui::Ui, _palette: Theme, settings: &mut Settings) {
        let mut slider = limit_to_slider(settings.speed_limit_kbps);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 130.0).clamp(90.0, 160.0);
            let response = ui.add(egui::Slider::new(&mut slider, 0..=100).show_value(false));
            response.widget_info(|| {
                egui::WidgetInfo::slider(ui.is_enabled(), slider as f64, "Speed limit slider")
            });
            if response.changed() {
                settings.speed_limit_kbps = slider_to_limit(slider);
                self.speed_text = format_speed(settings.speed_limit_kbps);
                self.speed_dirty = false;
                self.error = None;
            }
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut self.speed_text)
                        .desired_width(120.0)
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
            if response.changed() {
                self.speed_dirty = true;
            }
            if response.lost_focus()
                || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            {
                self.commit_speed(settings);
                if response.has_focus() {
                    response.surrender_focus();
                }
            }
        });
    }
}

fn setting_row(
    ui: &mut egui::Ui,
    palette: Theme,
    title: &str,
    description: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(13.0);
    if ui.available_width() >= 430.0 {
        ui.horizontal_top(|ui| {
            let width = (ui.available_width() * 0.45).max(0.0);
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(RichText::new(title).strong().color(palette.text));
                ui.add(
                    egui::Label::new(
                        RichText::new(description)
                            .size(12.0)
                            .color(palette.text_muted),
                    )
                    .wrap(),
                );
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                control(ui);
            });
        });
    } else {
        ui.label(RichText::new(title).strong().color(palette.text));
        ui.add(
            egui::Label::new(
                RichText::new(description)
                    .size(12.0)
                    .color(palette.text_muted),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        control(ui);
    }
    ui.add_space(13.0);
    ui.separator();
}

fn switch(ui: &mut egui::Ui, palette: Theme, value: &mut bool, label: &str) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *value, label)
    });
    ui.painter().rect_filled(
        rect,
        12,
        if *value {
            palette.accent
        } else {
            Color32::from_rgb(0x4b, 0x52, 0x63)
        },
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            14,
            Stroke::new(1.0, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    ui.painter().circle_filled(
        egui::pos2(
            if *value {
                rect.right() - 12.0
            } else {
                rect.left() + 12.0
            },
            rect.center().y,
        ),
        9.0,
        Color32::WHITE,
    );
}

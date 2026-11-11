//! Overlay rail and side panels. Pin and panel visibility are session-only.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Order, Pos2, Rect, ScrollArea, Sense, Stroke, UiBuilder,
    Vec2,
};

use crate::categories::category_of;
use crate::fake_backend::{Scope, State};
use crate::icons::{self, Icon};
use crate::theme::Theme;

const COLLAPSED: f32 = 60.0;
const EXPANDED: f32 = 240.0;
const ITEMS: [(&str, &str); 5] = [
    ("categories", "Categories"),
    ("labels", "Labels"),
    ("rss", "RSS Feeds"),
    ("recent", "Recent"),
    ("favorites", "Favorites"),
];

pub enum RailAction {
    ToggleDaemon,
    OpenPanel(&'static str),
    SelectCategory(String),
}

pub struct Rail {
    pinned: bool,
    open: Option<&'static str>,
    last_width: f32,
    reduced_motion: Arc<AtomicBool>,
    applied_reduced_motion: Option<bool>,
}

impl Rail {
    pub fn new(ctx: &egui::Context) -> Self {
        // Default to no animation until the OS preference has been checked.
        let reduced_motion = Arc::new(AtomicBool::new(true));
        watch_motion_preference(reduced_motion.clone(), ctx.clone());
        Self {
            pinned: false,
            open: None,
            last_width: COLLAPSED,
            reduced_motion,
            applied_reduced_motion: None,
        }
    }

    /// Reserve only the collapsed strip; the visible rail is drawn as an Area.
    pub fn reserve(&self, ui: &mut egui::Ui, palette: Theme) -> Rect {
        egui::Panel::left("rail_reservation")
            .exact_size(COLLAPSED)
            .frame(egui::Frame::NONE.fill(palette.surface))
            .show_separator_line(false)
            .show(ui, |_ui| {})
            .response
            .rect
    }

    pub fn close_panel(&mut self) {
        self.open = None;
    }

    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        rail_rect: Rect,
        palette: Theme,
        state: &State,
    ) -> Option<RailAction> {
        let reduced = self.reduced_motion.load(Ordering::Relaxed);
        if self.applied_reduced_motion != Some(reduced) {
            ctx.all_styles_mut(|style| style.animation_time = if reduced { 0.0 } else { 0.2 });
            self.applied_reduced_motion = Some(reduced);
        }
        let hover_rect = Rect::from_min_size(
            rail_rect.min,
            Vec2::new(self.last_width, rail_rect.height()),
        );
        let hovered = ctx.input(|i| {
            i.pointer
                .hover_pos()
                .is_some_and(|pos| hover_rect.contains(pos))
        });
        let expanded = hovered || self.pinned;
        let expansion = if reduced {
            if expanded {
                1.0
            } else {
                0.0
            }
        } else {
            ctx.animate_bool_with_time(Id::new("rail_width"), expanded, 0.18)
        };
        let alpha = if reduced {
            if expanded {
                1.0
            } else {
                0.0
            }
        } else {
            ctx.animate_bool_with_time(Id::new("rail_labels"), expanded, 0.15)
        };
        let width = COLLAPSED + (EXPANDED - COLLAPSED) * expansion;
        self.last_width = width;
        // Keep the side overlay adjacent to the visible rail so pinned content
        // does not cover its controls. Neither overlay changes the main layout.
        let mut action = self.show_side(ctx, rail_rect, width, palette, state);
        egui::Area::new(Id::new("rail_overlay"))
            .order(Order::Foreground)
            .fixed_pos(rail_rect.min)
            .movable(false)
            .fade_in(false)
            .constrain(false)
            .show(ctx, |ui| {
                let size = Vec2::new(width, rail_rect.height().max(0.0));
                ui.set_min_size(size);
                ui.set_max_size(size);
                let rect = Rect::from_min_size(ui.min_rect().min, size);
                let painter = ui.painter().clone();
                if expansion > 0.0 {
                    let shadow = egui::epaint::Shadow {
                        offset: [5, 0],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_black_alpha((70.0 * expansion) as u8),
                    };
                    painter.add(shadow.as_shape(rect, 0));
                }
                painter.rect_filled(rect, 0, palette.surface);
                painter.line_segment(
                    [rect.right_top(), rect.right_bottom()],
                    Stroke::new(1.0, palette.border),
                );
                let usable = (rect.height() - 58.0).max(0.0);
                let list_rect = Rect::from_min_size(rect.min, Vec2::new(width, usable));
                ui.scope_builder(UiBuilder::new().max_rect(list_rect), |ui| {
                    ui.set_clip_rect(list_rect);
                    ScrollArea::vertical()
                        .id_salt("rail_items")
                        .auto_shrink([false, false])
                        .max_height(usable)
                        .show(ui, |ui| {
                            ui.set_min_width(width);
                            ui.add_space(8.0);
                            let (row, response) =
                                ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
                            let response = response.on_hover_text(if self.pinned {
                                "Unpin rail"
                            } else {
                                "Pin rail"
                            });
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::Button,
                                    ui.is_enabled(),
                                    self.pinned,
                                    "Pin rail",
                                )
                            });
                            if response.hovered() || self.pinned {
                                ui.painter().rect_filled(
                                    row.shrink2(Vec2::new(8.0, 2.0)),
                                    8,
                                    palette.hover,
                                );
                            }
                            crate::theme::focus_outline(ui, &response, palette);
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    Pos2::new(row.left() + 30.0, row.center().y),
                                    Vec2::splat(18.0),
                                ),
                                Icon::Pin,
                                palette.text_dim,
                            );
                            paint_label(ui.painter(), row, "Pin rail", palette.text_dim, alpha);
                            if response.clicked() {
                                self.pinned = !self.pinned;
                            }
                            for (key, label) in ITEMS {
                                let (row, response) =
                                    ui.allocate_exact_size(Vec2::new(width, 46.0), Sense::click());
                                let response = response.on_hover_text(label);
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::Role::Button,
                                        ui.is_enabled(),
                                        self.open == Some(key),
                                        label,
                                    )
                                });
                                if response.hovered() || self.open == Some(key) {
                                    ui.painter().rect_filled(
                                        row.shrink2(Vec2::new(8.0, 2.0)),
                                        8,
                                        palette.hover,
                                    );
                                }
                                crate::theme::focus_outline(ui, &response, palette);
                                let color = if self.open == Some(key) || response.hovered() {
                                    palette.text
                                } else {
                                    palette.text_dim
                                };
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        Pos2::new(row.left() + 30.0, row.center().y),
                                        Vec2::splat(18.0),
                                    ),
                                    match key {
                                        "categories" => Icon::Folder,
                                        "labels" => Icon::Tag,
                                        "rss" => Icon::Rss,
                                        "recent" => Icon::Clock,
                                        "favorites" => Icon::Star,
                                        _ => Icon::Folder,
                                    },
                                    color,
                                );
                                paint_label(ui.painter(), row, label, color, alpha);
                                if response.clicked() {
                                    self.open = if self.open == Some(key) {
                                        None
                                    } else {
                                        Some(key)
                                    };
                                    if self.open.is_some() {
                                        action = Some(RailAction::OpenPanel(key));
                                    }
                                }
                            }
                        });
                });
                let status_rect = Rect::from_min_max(
                    Pos2::new(rect.left(), (rect.bottom() - 58.0).max(rect.top())),
                    rect.right_bottom(),
                );
                painter.line_segment(
                    [status_rect.left_top(), status_rect.right_top()],
                    Stroke::new(1.0, palette.border),
                );
                let response = ui
                    .interact(status_rect, Id::new("rail_daemon"), Sense::click())
                    .on_hover_text("Toggle simulated daemon connection");
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        ui.is_enabled(),
                        if state.daemon_connected {
                            "Daemon connected: toggle connection"
                        } else {
                            "Daemon disconnected: toggle connection"
                        },
                    )
                });
                if response.hovered() {
                    painter.rect_filled(status_rect, 0, palette.hover);
                }
                crate::theme::focus_outline(ui, &response, palette);
                let dot_color = if state.daemon_connected {
                    palette.green
                } else {
                    palette.red
                };
                let pulse = if state.daemon_connected && !reduced {
                    let time = ctx.input(|input| input.time);
                    ctx.request_repaint_after(Duration::from_millis(50));
                    0.8 + 0.2 * (std::f64::consts::PI * time).cos() as f32
                } else {
                    1.0
                };
                painter.circle_filled(
                    Pos2::new(rect.left() + 30.0, status_rect.center().y),
                    4.0,
                    dot_color.gamma_multiply(pulse),
                );
                paint_label(
                    &painter.with_clip_rect(rect),
                    status_rect,
                    if state.daemon_connected {
                        "Daemon connected"
                    } else {
                        "Daemon disconnected"
                    },
                    dot_color,
                    alpha,
                );
                if response.clicked() {
                    action = Some(RailAction::ToggleDaemon);
                }
            });
        action
    }

    fn show_side(
        &mut self,
        ctx: &egui::Context,
        rail_rect: Rect,
        rail_width: f32,
        palette: Theme,
        state: &State,
    ) -> Option<RailAction> {
        let panel = self.open?;
        let title = ITEMS
            .iter()
            .find(|(key, _)| *key == panel)
            .map(|(_, title)| *title)
            .unwrap_or(panel);
        let available = ctx.content_rect().right() - rail_rect.left() - rail_width;
        let width = 320.0_f32.min(available.max(0.0));
        if width <= 0.0 || rail_rect.height() <= 0.0 {
            return None;
        }
        let mut action = None;
        egui::Area::new(Id::new("side_overlay"))
            .order(Order::Middle)
            .fixed_pos(Pos2::new(rail_rect.left() + rail_width, rail_rect.top()))
            .movable(false)
            .fade_in(false)
            .constrain(false)
            .show(ctx, |ui| {
                let size = Vec2::new(width, rail_rect.height());
                ui.set_min_size(size);
                ui.set_max_size(size);
                let rect = Rect::from_min_size(ui.min_rect().min, size);
                ui.painter().rect_filled(rect, 0, palette.surface);
                ui.painter().line_segment(
                    [rect.right_top(), rect.right_bottom()],
                    Stroke::new(1.0, palette.border),
                );
                let content = rect.shrink2(Vec2::new(
                    20.0_f32.min(width / 4.0),
                    16.0_f32.min(rect.height() / 4.0),
                ));
                ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
                    ui.set_clip_rect(content);
                    ui.horizontal(|ui| {
                        ui.heading(title);
                        let close_response = ui
                            .add(
                                egui::Button::new(icons::atom(Icon::Close, 16.0, palette.text_dim))
                                    .frame(false),
                            )
                            .on_hover_text("Close panel");
                        close_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                ui.is_enabled(),
                                "Close panel",
                            )
                        });
                        if close_response.clicked() {
                            self.open = None;
                        }
                    });
                    ui.add_space(12.0);
                    ScrollArea::vertical()
                        .id_salt("side_contents")
                        .auto_shrink([false, false])
                        .show(ui, |ui| match panel {
                            "categories" => {
                                if state.categories.is_empty() {
                                    ui.label("Nothing here");
                                }
                                for category in &state.categories {
                                    let count = state
                                        .library
                                        .iter()
                                        .filter(|item| {
                                            category_of(state, &item.name) == category.id
                                        })
                                        .count();
                                    if ui
                                        .add(egui::Button::new((
                                            icons::atom(
                                                icons::category(&category.icon),
                                                16.0,
                                                palette.text_dim,
                                            ),
                                            format!("{}   {}", category.name, count),
                                        )))
                                        .clicked()
                                    {
                                        action =
                                            Some(RailAction::SelectCategory(category.id.clone()));
                                        self.open = None;
                                    }
                                }
                            }
                            "labels" => show_entries(
                                ui,
                                state.labels.iter().map(|name| (name.as_str(), "")),
                                palette,
                            ),
                            "rss" => show_entries(
                                ui,
                                state.feeds.iter().map(|name| (name.as_str(), "")),
                                palette,
                            ),
                            "recent" => show_entries(
                                ui,
                                state
                                    .library
                                    .iter()
                                    .take(3)
                                    .map(|item| (item.name.as_str(), item.finished.as_str())),
                                palette,
                            ),
                            "favorites" => {
                                let names =
                                    state.favorites.iter().filter_map(|item| match item.scope {
                                        Scope::Task => state
                                            .tasks
                                            .iter()
                                            .find(|x| x.id == item.id)
                                            .map(|x| x.name.as_str()),
                                        Scope::Seed => state
                                            .seeds
                                            .iter()
                                            .find(|x| x.id == item.id)
                                            .map(|x| x.name.as_str()),
                                        Scope::Library => state
                                            .library
                                            .iter()
                                            .find(|x| x.id == item.id)
                                            .map(|x| x.name.as_str()),
                                    });
                                show_entries(ui, names.map(|name| (name, "")), palette);
                            }
                            _ => {
                                ui.label("Nothing here");
                            }
                        });
                });
            });
        action
    }
}

fn show_entries<'a>(
    ui: &mut egui::Ui,
    items: impl Iterator<Item = (&'a str, &'a str)>,
    palette: Theme,
) {
    let mut empty = true;
    for (title, detail) in items {
        empty = false;
        let (row, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::hover());
        let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
        let detail_width = if detail.is_empty() {
            0.0
        } else {
            80.0_f32.min(row.width() * 0.4)
        };
        let title_rect = Rect::from_min_max(
            row.min + Vec2::new(8.0, 0.0),
            Pos2::new(
                (row.right() - detail_width - 8.0).max(row.left() + 8.0),
                row.bottom(),
            ),
        );
        crate::row_ui::filename(&painter, title_rect, row.center().y, title, palette.text);
        if !detail.is_empty() {
            painter.text(
                Pos2::new(row.right() - 8.0, row.center().y),
                Align2::RIGHT_CENTER,
                detail,
                FontId::proportional(11.0),
                palette.text_muted,
            );
        }
        painter.line_segment(
            [row.left_bottom(), row.right_bottom()],
            Stroke::new(1.0, palette.border),
        );
    }
    if empty {
        ui.label("Nothing here");
    }
}

fn paint_label(painter: &egui::Painter, row: Rect, label: &str, color: Color32, alpha: f32) {
    if alpha > 0.0 {
        painter.text(
            Pos2::new(row.left() + 62.0, row.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            color.gamma_multiply(alpha),
        );
    }
}

fn watch_motion_preference(reduced: Arc<AtomicBool>, ctx: egui::Context) {
    std::thread::spawn(move || loop {
        let value = query_reduced_motion().unwrap_or(true);
        if reduced.swap(value, Ordering::Relaxed) != value {
            ctx.request_repaint();
        }
        std::thread::sleep(Duration::from_secs(10));
    });
}

#[cfg(target_os = "linux")]
fn query_reduced_motion() -> Option<bool> {
    if std::env::var("GTK_ANIMATIONS").ok().as_deref() == Some("0") {
        return Some(true);
    }
    if std::env::var_os("KDE_FULL_SESSION").is_some() {
        let path = std::env::var_os("HOME")
            .map(PathBuf::from)?
            .join(".config/kdeglobals");
        if let Ok(config) = std::fs::read_to_string(path) {
            if let Some(value) = config
                .lines()
                .find_map(|line| line.trim().strip_prefix("AnimationDurationFactor="))
            {
                if let Ok(factor) = value.trim().parse::<f64>() {
                    return Some(factor == 0.0);
                }
            }
        }
    }
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
fn query_reduced_motion() -> Option<bool> {
    let output = std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn query_reduced_motion() -> Option<bool> {
    let output = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "[Windows.UI.ViewManagement.UISettings, Windows.UI.ViewManagement, ContentType = WindowsRuntime]::new().AnimationsEnabled"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "True" => Some(false),
        "False" => Some(true),
        _ => None,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn query_reduced_motion() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_backend::{Command, FakeBackend, View};

    #[test]
    fn categories_use_only_the_last_extension_and_ignore_case() {
        let path = std::env::temp_dir()
            .join(format!("rget-rail-category-test-{}", std::process::id()))
            .join("state.json");
        let mut backend = FakeBackend::open(path.clone()).unwrap();
        let state = backend.snapshot();
        assert_eq!(category_of(state, "Photo.JPEG"), "images");
        assert_eq!(category_of(state, "report.PDF"), "pdfs");
        assert_eq!(category_of(state, "report.pdf.backup"), "other");
        assert_eq!(category_of(state, "no_extension"), "other");
        backend
            .dispatch(Command::SelectCategory("pdfs".into()))
            .unwrap();
        assert_eq!(backend.snapshot().view, View::Downloads);
        assert_eq!(backend.snapshot().selected_category, "pdfs");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

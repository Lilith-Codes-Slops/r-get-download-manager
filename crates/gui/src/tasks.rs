//! Tasks view: independent Active and Transfers panes.
use eframe::egui::{
    self, Align2, FontId, Id, Layout, Pos2, Rect, ScrollArea, Sense, Stroke, UiBuilder, Vec2,
};

use crate::fake_backend::{Command, Scope, Selection, SelectionMode, State, TaskStatus};
use crate::icons::{self, Icon};
use crate::row_layout::{self, TASK_COLUMNS};
use crate::task_filter;
use crate::theme::Theme;

const HEADER_HEIGHT: f32 = 52.0;
const ACTIVE_EMPTY: &str = "No active downloads. Use + URL to add one.";
const TRANSFERS_EMPTY: &str = "No completed or stopped transfers";

struct Row {
    selection: Selection,
    name: String,
    detail: String,
    speed: String,
    progress: Option<f64>,
    progress_color: eframe::egui::Color32,
    quick_icon: Icon,
    quick_label: &'static str,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &State,
    palette: Theme,
    menu_action: &mut Option<crate::menus::Action>,
) -> Vec<Command> {
    let mut actions = Vec::new();
    let active = task_filter::active_items(state);
    let transfers = task_filter::transfer_items(state);
    let active_rows: Vec<_> = active
        .iter()
        .map(|task| {
            let status = match task.status {
                TaskStatus::Active if !state.daemon_connected => "Waiting for daemon",
                TaskStatus::Active if task.queued => "Queued",
                TaskStatus::Active => "Downloading",
                TaskStatus::Paused => "Paused",
                TaskStatus::Failed => "Failed",
            };
            let pct = if task.progress.is_finite() {
                task.progress.clamp(0.0, 100.0)
            } else {
                0.0
            };
            let running =
                matches!(task.status, TaskStatus::Active) && state.daemon_connected && !task.queued;
            let speed = if running { task.speed_kbps } else { 0.0 };
            let detail = match status {
                "Downloading" => format!(
                    "{:.0}% · ETA {} · {} connections",
                    pct.floor(),
                    row_layout::format_eta(task.size_mb, pct, speed),
                    task.connections
                ),
                "Queued" => format!(
                    "{:.0}% · Queued (max {} at once)",
                    pct.floor(),
                    state.settings.max_concurrent
                ),
                "Waiting for daemon" => format!("{:.0}% · Waiting for daemon", pct.floor()),
                "Paused" => format!(
                    "{:.0}% · Paused · {} connections",
                    pct.floor(),
                    task.connections
                ),
                _ => format!("{:.0}% · Failed · Retry restarts the download", pct.floor()),
            };
            let (quick_icon, quick_label, progress_color) = match task.status {
                TaskStatus::Active => (Icon::Pause, "Pause", palette.green),
                TaskStatus::Paused => (Icon::Play, "Resume", palette.orange),
                TaskStatus::Failed => (Icon::Retry, "Retry", palette.red),
            };
            Row {
                selection: Selection {
                    scope: Scope::Task,
                    id: task.id.clone(),
                },
                name: task.name.clone(),
                detail,
                speed: format_speed(speed),
                progress: Some(task.progress),
                progress_color,
                quick_icon,
                quick_label,
            }
        })
        .collect();
    let transfer_rows: Vec<_> = transfers
        .iter()
        .map(|seed| Row {
            selection: Selection {
                scope: Scope::Seed,
                id: seed.id.clone(),
            },
            name: seed.name.clone(),
            detail: format!(
                "Ratio {:.2} of {:.1} · {}",
                seed.ratio,
                state.settings.seed_ratio,
                if !seed.running {
                    "Stopped"
                } else if !state.daemon_connected {
                    "Waiting for daemon"
                } else {
                    "Seeding"
                }
            ),
            speed: format_speed(if seed.running && state.daemon_connected {
                seed.speed_kbps
            } else {
                0.0
            }),
            progress: None,
            progress_color: palette.green,
            quick_icon: if seed.running { Icon::Stop } else { Icon::Play },
            quick_label: if seed.running {
                "Stop seeding"
            } else {
                "Start seeding"
            },
        })
        .collect();

    let size = ui.available_size_before_wrap().max(Vec2::ZERO);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    // Breakpoint uses window width, not the narrower content area after the rail.
    let stacked = ui.ctx().content_rect().width() < 820.0;
    let (active_rect, transfer_rect) = if stacked {
        let mid = rect.top() + rect.height() / 2.0;
        (
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), mid)),
            Rect::from_min_max(Pos2::new(rect.left(), mid), rect.max),
        )
    } else {
        let mid = rect.left() + rect.width() / 2.0;
        (
            Rect::from_min_max(rect.min, Pos2::new(mid, rect.bottom())),
            Rect::from_min_max(Pos2::new(mid, rect.top()), rect.max),
        )
    };
    pane(
        ui,
        active_rect,
        palette,
        "Active",
        state.tasks.len(),
        &state.active_filter,
        ACTIVE_EMPTY,
        &active_rows,
        state,
        true,
        &mut actions,
        menu_action,
    );
    pane(
        ui,
        transfer_rect,
        palette,
        "Transfers",
        state.seeds.len(),
        &state.seed_filter,
        TRANSFERS_EMPTY,
        &transfer_rows,
        state,
        false,
        &mut actions,
        menu_action,
    );
    if stacked {
        ui.painter().line_segment(
            [active_rect.left_bottom(), active_rect.right_bottom()],
            Stroke::new(1.0, palette.border),
        );
    } else {
        ui.painter().line_segment(
            [active_rect.right_top(), active_rect.right_bottom()],
            Stroke::new(1.0, palette.border),
        );
    }
    actions
}

#[allow(clippy::too_many_arguments)]
fn pane(
    ui: &mut egui::Ui,
    rect: Rect,
    palette: Theme,
    title: &str,
    count: usize,
    filter: &str,
    empty: &str,
    rows: &[Row],
    state: &State,
    active: bool,
    actions: &mut Vec<Command>,
    menu_action: &mut Option<crate::menus::Action>,
) {
    let header_height = HEADER_HEIGHT.min(rect.height());
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header_height));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(header, 0, palette.surface);
    painter.line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, palette.border),
    );
    let search_width = 150.0_f32.min((rect.width() * 0.42).max(0.0));
    let search_right = (header.right() - 14.0).max(header.left());
    let search_rect = Rect::from_min_max(
        Pos2::new(
            (search_right - search_width).max(header.left()),
            header.center().y - 14.0,
        ),
        Pos2::new(search_right, header.center().y + 14.0),
    )
    .intersect(header);
    let title_clip = Rect::from_min_max(
        header.min,
        Pos2::new(search_rect.left().max(header.left()), header.bottom()),
    );
    painter.with_clip_rect(title_clip).text(
        Pos2::new(header.left() + 18.0, header.center().y),
        Align2::LEFT_CENTER,
        format!("{title} ({count})"),
        FontId::proportional(13.0),
        palette.text_dim,
    );
    let mut filter_text = filter.to_owned();
    if search_rect.is_positive() {
        let response = ui.put(
            search_rect,
            egui::TextEdit::singleline(&mut filter_text)
                .id(Id::unique(if active {
                    "active_filter"
                } else {
                    "transfer_filter"
                }))
                .hint_text("Filter…")
                .desired_width(search_width),
        );
        response.widget_info(|| {
            let mut info =
                egui::WidgetInfo::text_edit(ui.is_enabled(), &filter_text, &filter_text, "Filter…");
            info.label = Some(
                if active {
                    "Filter active downloads"
                } else {
                    "Filter transfers"
                }
                .into(),
            );
            info
        });
        if response.changed() {
            actions.push(if active {
                Command::SetActiveFilter(filter_text)
            } else {
                Command::SetSeedFilter(filter_text)
            });
        }
    }
    let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), rect.max);
    if !body.is_positive() {
        return;
    }
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(if active {
                "active_pane"
            } else {
                "transfer_pane"
            })
            .max_rect(body)
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    ScrollArea::vertical()
        .id_salt("rows")
        .min_scrolled_height(0.0)
        .max_height(body.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if rows.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(empty).color(palette.text_muted))
                            .wrap(),
                    );
                });
                return;
            }
            let visible: Vec<_> = rows.iter().map(|row| row.selection.clone()).collect();
            for row in rows {
                draw_row(ui, row, &visible, state, palette, actions, menu_action);
            }
        });
}

fn draw_row(
    ui: &mut egui::Ui,
    row: &Row,
    visible: &[Selection],
    state: &State,
    palette: Theme,
    actions: &mut Vec<Command>,
    menu_action: &mut Option<crate::menus::Action>,
) {
    let compact = state.settings.compact;
    let padding = if compact { 6.0 } else { 10.0 };
    let horizontal_padding = if compact { 12.0 } else { 14.0 };
    // Match the content's intrinsic height: filename, bars and metadata.
    let content_height = if row.progress.is_some() { 58.0 } else { 33.0 };
    let height = content_height + 2.0 * padding + 1.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let response = response.on_hover_text(&row.name);
    if response.secondary_clicked() && !state.selections.contains(&row.selection) {
        actions.push(Command::Select {
            item: row.selection.clone(),
            mode: SelectionMode::Replace,
            visible: visible.to_vec(),
        });
    }
    let menu_was_empty = menu_action.is_none();
    response.context_menu(|ui| crate::menus::row(ui, state, &row.selection, palette, menu_action));
    if menu_was_empty && menu_action.is_some() {
        response.request_focus();
    }
    let mut keyboard_activation = false;
    if response.has_focus() && !egui::Popup::is_any_open(ui.ctx()) {
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            keyboard_activation = true;
            if !state.selections.is_empty() {
                *menu_action = Some(crate::menus::Action::ToggleSelected);
            }
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            && !state.selections.is_empty()
        {
            *menu_action = Some(crate::menus::Action::RemoveSelected);
        }
    }
    let selected = state.selections.contains(&row.selection);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, &row.name)
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
    crate::theme::focus_outline(ui, &response, palette);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, palette.border),
    );
    let columns = row_layout::columns(rect.width(), horizontal_padding, TASK_COLUMNS);
    let col = |index: usize| {
        Rect::from_min_size(
            Pos2::new(rect.left() + columns[index].0, rect.top()),
            Vec2::new(columns[index].1, rect.height()),
        )
    };
    let first = col(0);
    let content_column = col(1);
    let speed = col(2);
    let selecting = !state.selections.is_empty();
    let mut control_rects = Vec::new();
    if selecting && first.width() > 0.0 {
        let checkbox_rect =
            Rect::from_center_size(first.center(), Vec2::new(22.0_f32.min(first.width()), 18.0))
                .intersect(first);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(("row_checkbox", &row.selection.id))
                .max_rect(checkbox_rect)
                .layout(Layout::left_to_right(egui::Align::Center)),
        );
        child.set_clip_rect(checkbox_rect.intersect(ui.clip_rect()));
        let mut checked = selected;
        let checkbox = child.add(egui::Checkbox::new(&mut checked, ""));
        checkbox.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                ui.is_enabled(),
                checked,
                format!("Select {}", row.name),
            )
        });
        if checkbox.changed() {
            actions.push(Command::Select {
                item: row.selection.clone(),
                mode: SelectionMode::Toggle,
                visible: visible.to_vec(),
            });
        }
        control_rects.push(checkbox_rect);
    }
    // When checkboxes are visible, reserve the end of the flexible track for
    // the quick action, leaving the 90px speed column readable.
    let button_track = if selecting { content_column } else { first };
    let button_size = 32.0_f32.min(button_track.width());
    if button_size >= 16.0 {
        let button_rect = Rect::from_center_size(
            Pos2::new(
                if selecting {
                    button_track.right() - button_size / 2.0
                } else {
                    button_track.left() + button_size / 2.0
                },
                rect.center().y,
            ),
            Vec2::splat(button_size),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(("quick", &row.selection.id))
                .max_rect(button_rect),
        );
        child.set_clip_rect(button_rect.intersect(ui.clip_rect()));
        child.spacing_mut().button_padding = Vec2::ZERO;
        let label = format!("{} {}", row.quick_label, row.name);
        let button = child.add_sized(
            button_rect.size(),
            egui::Button::new(icons::atom(row.quick_icon, 16.0, palette.text))
                .fill(if ui.rect_contains_pointer(button_rect) {
                    palette.surface_3
                } else {
                    palette.surface_2
                })
                .stroke(Stroke::NONE)
                .corner_radius(8),
        );
        let button = button.on_hover_text(&label);
        button
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
        if button.clicked() {
            actions.push(match row.selection.scope {
                Scope::Task => Command::ToggleTask(row.selection.id.clone()),
                Scope::Seed => Command::ToggleSeed(row.selection.id.clone()),
                Scope::Library => unreachable!("library rows are not displayed here"),
            });
        }
        control_rects.push(button_rect);
    }
    let content = if selecting && button_size >= 16.0 {
        Rect::from_min_max(
            content_column.min,
            Pos2::new(
                (content_column.right() - button_size - 10.0).max(content_column.left()),
                content_column.bottom(),
            ),
        )
    } else {
        content_column
    };
    if content.width() > 0.0 {
        let text_painter = painter.with_clip_rect(content.intersect(rect));
        let top = rect.center().y - content_height / 2.0;
        crate::row_ui::filename(
            &text_painter,
            content.intersect(rect),
            top + 8.0,
            &row.name,
            palette.text,
        );
        if let Some(progress) = row.progress {
            let bar = Rect::from_min_size(
                Pos2::new(content.left(), top + 22.0),
                Vec2::new(content.width(), 5.0),
            );
            text_painter.rect_filled(bar, 3, palette.surface_3);
            let fill = bar.width() * row_layout::progress_fraction(progress);
            if fill > 0.0 {
                text_painter.with_clip_rect(bar).rect_filled(
                    Rect::from_min_size(bar.min, Vec2::new(fill, 5.0)),
                    3,
                    row.progress_color,
                );
            }
            let segments = Rect::from_min_size(
                Pos2::new(content.left(), top + 33.0),
                Vec2::new(content.width(), 8.0),
            );
            let gap = 1.0_f32.min(segments.width() / (row_layout::SEGMENTS - 1) as f32);
            let segment_width = ((segments.width() - gap * (row_layout::SEGMENTS - 1) as f32)
                / row_layout::SEGMENTS as f32)
                .max(0.0);
            for i in 0..row_layout::SEGMENTS {
                let segment = Rect::from_min_size(
                    Pos2::new(
                        segments.left() + i as f32 * (segment_width + gap),
                        segments.top(),
                    ),
                    Vec2::new(segment_width, 8.0),
                );
                text_painter.rect_filled(segment, 0, palette.track);
                let fraction = row_layout::segment_fraction(progress, i);
                if fraction > 0.0 {
                    text_painter.rect_filled(
                        Rect::from_min_size(segment.min, Vec2::new(segment_width * fraction, 8.0)),
                        0,
                        if fraction >= 1.0 {
                            palette.seg_done
                        } else {
                            palette.seg_active
                        },
                    );
                }
            }
            text_painter.text(
                Pos2::new(content.left(), top + 51.0),
                Align2::LEFT_CENTER,
                &row.detail,
                FontId::proportional(11.0),
                palette.text_muted,
            );
        } else {
            text_painter.text(
                Pos2::new(content.left(), top + 26.0),
                Align2::LEFT_CENTER,
                &row.detail,
                FontId::proportional(11.0),
                palette.text_muted,
            );
        }
    }
    painter.with_clip_rect(speed.intersect(rect)).text(
        Pos2::new(speed.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &row.speed,
        FontId::proportional(12.0),
        palette.text_dim,
    );
    let on_control = ui.input(|input| {
        input
            .pointer
            .interact_pos()
            .is_some_and(|pos| control_rects.iter().any(|r| r.contains(pos)))
    });
    if response.clicked()
        && !keyboard_activation
        && (!on_control || !response.clicked_by(egui::PointerButton::Primary))
    {
        let modifiers = ui.input(|input| input.modifiers);
        let mode = match (modifiers.shift, modifiers.command) {
            (false, false) => SelectionMode::Replace,
            (false, true) => SelectionMode::Toggle,
            (true, false) => SelectionMode::Range,
            (true, true) => SelectionMode::AddRange,
        };
        actions.push(Command::Select {
            item: row.selection.clone(),
            mode,
            visible: visible.to_vec(),
        });
    }
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

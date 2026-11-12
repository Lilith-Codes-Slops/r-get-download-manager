//! Shared compact framing for prototype dialogs.
use crate::theme::Theme;
use eframe::egui::{Color32, Frame, Id, Margin, Modal, Stroke};

pub fn modal(id: &'static str, palette: Theme) -> Modal {
    Modal::new(Id::unique(id))
        .frame(frame(palette))
        .backdrop_color(Color32::from_black_alpha(153))
}

fn frame(palette: Theme) -> Frame {
    Frame::NONE
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(14)
        .inner_margin(Margin::same(20))
}

pub fn width(ctx: &eframe::egui::Context, max_width: f32) -> f32 {
    (ctx.content_rect().width() - 72.0).clamp(0.0, max_width)
}

pub fn max_height(ctx: &eframe::egui::Context) -> f32 {
    (ctx.content_rect().height() * 0.82 - 40.0).max(0.0)
}

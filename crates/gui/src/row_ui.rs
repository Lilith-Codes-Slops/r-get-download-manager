//! Shared filename rendering for narrow, scrollable row grids.
use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect};

pub fn filename(painter: &Painter, bounds: Rect, center_y: f32, text: &str, color: Color32) {
    if bounds.width() <= 0.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        FontId::proportional(13.0),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(bounds.width());
    let galley = painter.layout_job(job);
    let pos = Pos2::new(bounds.left(), center_y - galley.size().y / 2.0);
    painter.with_clip_rect(bounds).galley(pos, galley, color);
}

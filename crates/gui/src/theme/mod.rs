//! Theme tokens shared by the native prototype's widgets.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

/// The HTML reference's semantic palette, including colors for components
/// that will be introduced in later stages of the prototype.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub green: Color32,
    pub orange: Color32,
    pub red: Color32,
    pub hover: Color32,
    pub track: Color32,
    pub seg_done: Color32,
    pub seg_active: Color32,
}

impl Theme {
    pub const DARK: Self = Self {
        bg: rgb(0x0c0e12),
        surface: rgb(0x16191f),
        surface_2: rgb(0x1e222a),
        surface_3: rgb(0x252a34),
        border: rgb(0x2d3340),
        text: rgb(0xf0f2f5),
        text_dim: rgb(0xa3acbf),
        text_muted: rgb(0x8b93a3),
        accent: rgb(0x3b82f6),
        green: rgb(0x10b981),
        orange: rgb(0xf59e0b),
        red: rgb(0xef4444),
        hover: rgb(0x222733),
        track: rgb(0x3a4050),
        seg_done: rgb(0x059669),
        seg_active: rgb(0x34d399),
    };

    // The reference only overrides these tokens in light mode; semantic colors
    // such as accent and segment colors remain the same in both themes.
    pub const LIGHT: Self = Self {
        bg: rgb(0xf4f5f7),
        surface: rgb(0xffffff),
        surface_2: rgb(0xf0f2f5),
        surface_3: rgb(0xe5e7eb),
        border: rgb(0xe2e5ea),
        text: rgb(0x111827),
        text_dim: rgb(0x4b5563),
        text_muted: rgb(0x6b7280),
        hover: rgb(0xf3f4f6),
        track: rgb(0xd1d5db),
        ..Self::DARK
    };

    fn visuals(self, dark: bool) -> Visuals {
        let mut visuals = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.surface;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(10);
        visuals.menu_corner_radius = CornerRadius::same(10);
        visuals.weak_text_color = Some(self.text_dim);
        visuals.hyperlink_color = self.accent;
        visuals.faint_bg_color = self.surface;
        visuals.extreme_bg_color = self.surface_2;
        visuals.text_edit_bg_color = Some(self.surface_2);
        visuals.code_bg_color = self.surface_2;
        visuals.warn_fg_color = self.orange;
        visuals.error_fg_color = self.red;
        visuals.selection.bg_fill = self.accent;
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

        for (widget, fill, foreground) in [
            (&mut visuals.widgets.noninteractive, self.surface, self.text),
            (&mut visuals.widgets.inactive, self.surface_2, self.text),
            (&mut visuals.widgets.hovered, self.hover, self.text),
            (&mut visuals.widgets.active, self.surface_3, self.text),
            (&mut visuals.widgets.open, self.surface_3, self.text),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, self.border);
            widget.fg_stroke = Stroke::new(1.0, foreground);
            widget.corner_radius = CornerRadius::same(8);
            widget.expansion = 0.0;
        }
        visuals.widgets.active.bg_stroke = Stroke::new(2.0, self.accent);
        visuals
    }
}

/// The reference's visible keyboard focus ring for custom-painted controls.
pub fn focus_outline(ui: &egui::Ui, response: &egui::Response, palette: Theme) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            4,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Install both palettes so System follows eframe's OS theme events, including
/// changes while the app is running. Egui uses its default fallback if the OS
/// does not report a preference.
pub fn apply_theme(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_visuals_of(egui::Theme::Dark, Theme::DARK.visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, Theme::LIGHT.visuals(false));
    ctx.set_theme(preference);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_keep_the_reference_colors() {
        assert_eq!(Theme::DARK.bg, rgb(0x0c0e12));
        assert_eq!(Theme::DARK.border, rgb(0x2d3340));
        assert_eq!(Theme::DARK.seg_done, rgb(0x059669));
        assert_eq!(Theme::DARK.seg_active, rgb(0x34d399));
        assert_eq!(Theme::LIGHT.bg, rgb(0xf4f5f7));
        assert_eq!(Theme::LIGHT.border, rgb(0xe2e5ea));
        assert_eq!(Theme::LIGHT.accent, Theme::DARK.accent);
        assert_eq!(Theme::LIGHT.seg_done, Theme::DARK.seg_done);
    }

    #[test]
    fn applies_both_themes_and_respects_explicit_preference() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, egui::ThemePreference::Light);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(
            ctx.style_of(egui::Theme::Light).visuals.panel_fill,
            Theme::LIGHT.bg
        );
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Theme::DARK.bg
        );
        apply_theme(&ctx, egui::ThemePreference::Dark);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn system_preference_tracks_reported_theme_changes() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, egui::ThemePreference::System);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            ctx.begin_pass(egui::RawInput {
                system_theme: Some(theme),
                ..Default::default()
            });
            assert_eq!(ctx.theme(), theme);
            let _ = ctx.end_pass();
        }
    }
}

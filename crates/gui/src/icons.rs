//! Theme-aware outline icons, drawn natively in logical 16 or 18 point squares.
//! No icon fonts, image decoding, or external assets are required.
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Folder,
    Tag,
    Rss,
    Clock,
    Star,
    Pin,
    Image,
    Video,
    File,
    Archive,
    Music,
    Disc,
    Grid,
    Question,
    Link,
    Magnet,
    FilePlus,
    Pause,
    Play,
    Stop,
    Retry,
    Trash,
    Settings,
    Help,
    Command,
    Plus,
    Theme,
    Close,
    Minimize,
    Maximize,
    Download,
    Upload,
}

/// Select the icon stored on a category, with a stable fallback for custom data.
pub fn category(icon: &str) -> Icon {
    match icon {
        "image" => Icon::Image,
        "video" => Icon::Video,
        "file" => Icon::File,
        "archive" => Icon::Archive,
        "music" => Icon::Music,
        "disc" => Icon::Disc,
        _ => Icon::Folder,
    }
}

/// Make an icon usable inside a normal accessible egui button alongside text.
pub fn atom(icon: Icon, size: f32, color: Color32) -> egui::Atom<'static> {
    egui::Atom::paint(Vec2::splat(size), move |ui, args| {
        paint(ui.painter(), args.rect, icon, color);
    })
}

/// Paint a centered icon into the supplied 16/18 point rectangle.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let scale = rect.width().min(rect.height()) / 16.0;
    if scale <= 0.0 {
        return;
    }
    let center = rect.center();
    let point = |x: f32, y: f32| center + Vec2::new(x * scale, y * scale);
    let stroke = Stroke::new(1.35 * scale, color);
    let path = |points: &[(f32, f32)]| {
        painter.add(Shape::line(
            points.iter().map(|&(x, y)| point(x, y)).collect(),
            stroke,
        ));
    };
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    };
    let circle = |x: f32, y: f32, radius: f32| {
        painter.circle_stroke(point(x, y), radius * scale, stroke);
    };
    let arc = |x: f32, y: f32, radius: f32, start: f32, end: f32| {
        let points: Vec<Pos2> = (0..=16)
            .map(|i| {
                let angle = start + (end - start) * i as f32 / 16.0;
                point(x + radius * angle.cos(), y + radius * angle.sin())
            })
            .collect();
        painter.add(Shape::line(points, stroke));
    };
    use std::f32::consts::{FRAC_PI_2, PI};
    match icon {
        Icon::Folder => path(&[
            (-6.8, -4.8),
            (-2.5, -4.8),
            (-0.8, -3.0),
            (6.8, -3.0),
            (6.8, 5.5),
            (-6.8, 5.5),
            (-6.8, -4.8),
        ]),
        Icon::Tag => {
            path(&[
                (-6.5, -6.0),
                (0.2, -6.0),
                (6.6, 0.4),
                (0.5, 6.5),
                (-6.5, -0.5),
                (-6.5, -6.0),
            ]);
            circle(-3.5, -3.2, 0.7);
        }
        Icon::Rss => {
            circle(-5.3, 5.2, 0.6);
            arc(-5.3, 5.2, 6.1, -FRAC_PI_2, 0.0);
            arc(-5.3, 5.2, 11.8, -FRAC_PI_2, 0.0);
        }
        Icon::Clock => {
            circle(0.0, 0.0, 6.6);
            path(&[(0.0, -4.0), (0.0, 0.2), (3.4, 2.0)]);
        }
        Icon::Star => {
            let points: Vec<_> = (0..=10)
                .map(|i| {
                    let a = -FRAC_PI_2 + i as f32 * PI / 5.0;
                    let r = if i % 2 == 0 { 6.8 } else { 3.0 };
                    (r * a.cos(), r * a.sin())
                })
                .collect();
            path(&points);
        }
        Icon::Pin => {
            path(&[
                (-4.0, -6.3),
                (4.0, -6.3),
                (2.4, -4.8),
                (2.4, -1.8),
                (4.5, 0.2),
                (-4.5, 0.2),
                (-2.4, -1.8),
                (-2.4, -4.8),
                (-4.0, -6.3),
            ]);
            line((0.0, 0.2), (0.0, 6.5));
        }
        Icon::Image => {
            path(&[
                (-6.5, -5.8),
                (6.5, -5.8),
                (6.5, 5.8),
                (-6.5, 5.8),
                (-6.5, -5.8),
            ]);
            circle(-2.7, -2.2, 1.0);
            path(&[
                (-5.2, 4.2),
                (-1.0, 0.5),
                (1.0, 2.2),
                (3.0, -0.1),
                (6.2, 3.5),
            ]);
        }
        Icon::Video => {
            path(&[
                (-6.5, -4.8),
                (2.5, -4.8),
                (2.5, 4.8),
                (-6.5, 4.8),
                (-6.5, -4.8),
            ]);
            path(&[(2.5, -2.5), (6.5, -4.3), (6.5, 4.3), (2.5, 2.5)]);
        }
        Icon::File | Icon::FilePlus | Icon::Question => {
            path(&[
                (-5.0, -6.8),
                (1.5, -6.8),
                (5.5, -2.8),
                (5.5, 6.8),
                (-5.0, 6.8),
                (-5.0, -6.8),
            ]);
            path(&[(1.5, -6.8), (1.5, -2.8), (5.5, -2.8)]);
            if icon == Icon::FilePlus {
                line((-2.8, 2.0), (3.0, 2.0));
                line((0.1, -0.8), (0.1, 4.8));
            }
            if icon == Icon::Question {
                arc(0.0, 0.0, 2.1, -PI, 0.1);
                line((2.1, 0.2), (0.0, 2.1));
                line((0.0, 2.1), (0.0, 2.8));
                circle(0.0, 5.0, 0.45);
            }
        }
        Icon::Archive => {
            path(&[
                (-6.7, -6.0),
                (6.7, -6.0),
                (6.7, -3.0),
                (-6.7, -3.0),
                (-6.7, -6.0),
            ]);
            path(&[(-5.4, -3.0), (-5.4, 6.0), (5.4, 6.0), (5.4, -3.0)]);
            line((-2.0, 0.1), (2.0, 0.1));
        }
        Icon::Music => {
            path(&[(-1.8, 3.8), (-1.8, -5.4), (5.5, -6.4), (5.5, 2.1)]);
            circle(-4.0, 4.4, 2.0);
            circle(3.4, 3.0, 2.0);
        }
        Icon::Disc => {
            circle(0.0, 0.0, 6.7);
            circle(0.0, 0.0, 1.7);
        }
        Icon::Grid => {
            for (x, y) in [(-5.5, -5.5), (1.5, -5.5), (-5.5, 1.5), (1.5, 1.5)] {
                path(&[
                    (x, y),
                    (x + 4.0, y),
                    (x + 4.0, y + 4.0),
                    (x, y + 4.0),
                    (x, y),
                ]);
            }
        }
        Icon::Link => {
            arc(-2.6, 2.1, 3.0, 0.4, PI + 0.6);
            arc(2.6, -2.1, 3.0, PI + 0.4, 2.0 * PI + 0.6);
            line((-2.8, 2.8), (2.8, -2.8));
        }
        Icon::Magnet => {
            path(&[(-5.8, -5.8), (-2.4, -5.8), (-2.4, 1.5)]);
            path(&[(2.4, 1.5), (2.4, -5.8), (5.8, -5.8), (5.8, 1.5)]);
            arc(0.0, 1.5, 5.8, 0.0, PI);
            arc(0.0, 1.5, 2.4, 0.0, PI);
            line((-5.8, -3.1), (-2.4, -3.1));
            line((2.4, -3.1), (5.8, -3.1));
        }
        Icon::Pause => {
            path(&[
                (-4.6, -5.6),
                (-1.8, -5.6),
                (-1.8, 5.6),
                (-4.6, 5.6),
                (-4.6, -5.6),
            ]);
            path(&[
                (1.8, -5.6),
                (4.6, -5.6),
                (4.6, 5.6),
                (1.8, 5.6),
                (1.8, -5.6),
            ]);
        }
        Icon::Play => path(&[(-4.6, -6.0), (5.8, 0.0), (-4.6, 6.0), (-4.6, -6.0)]),
        Icon::Stop => path(&[
            (-5.0, -5.0),
            (5.0, -5.0),
            (5.0, 5.0),
            (-5.0, 5.0),
            (-5.0, -5.0),
        ]),
        Icon::Retry => {
            arc(0.0, 0.0, 6.2, -2.3, 3.7);
            path(&[(-6.3, -5.0), (-6.3, -1.4), (-2.6, -1.4)]);
        }
        Icon::Trash => {
            line((-6.0, -4.4), (6.0, -4.4));
            path(&[(-4.5, -4.4), (-4.0, 6.2), (4.0, 6.2), (4.5, -4.4)]);
            path(&[(-2.6, -4.4), (-2.0, -6.5), (2.0, -6.5), (2.6, -4.4)]);
            line((-1.7, -1.7), (-1.7, 3.5));
            line((1.7, -1.7), (1.7, 3.5));
        }
        Icon::Settings => {
            circle(0.0, 0.0, 2.5);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                line(
                    (4.7 * a.cos(), 4.7 * a.sin()),
                    (6.7 * a.cos(), 6.7 * a.sin()),
                );
            }
            circle(0.0, 0.0, 5.0);
        }
        Icon::Help => {
            circle(0.0, 0.0, 6.7);
            arc(0.0, -1.8, 2.2, -PI, 0.2);
            line((2.1, -1.4), (0.0, 0.4));
            line((0.0, 0.4), (0.0, 1.4));
            circle(0.0, 4.1, 0.45);
        }
        Icon::Command => {
            for (x, y) in [(-3.7, -3.7), (3.7, -3.7), (-3.7, 3.7), (3.7, 3.7)] {
                circle(x, y, 2.0);
            }
            path(&[
                (-3.7, -1.7),
                (-3.7, 1.7),
                (3.7, 1.7),
                (3.7, -1.7),
                (-3.7, -1.7),
            ]);
        }
        Icon::Plus => {
            line((-5.5, 0.0), (5.5, 0.0));
            line((0.0, -5.5), (0.0, 5.5));
        }
        Icon::Theme => {
            circle(0.0, 0.0, 3.4);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                line(
                    (5.0 * a.cos(), 5.0 * a.sin()),
                    (6.7 * a.cos(), 6.7 * a.sin()),
                );
            }
        }
        Icon::Close => {
            line((-5.0, -5.0), (5.0, 5.0));
            line((5.0, -5.0), (-5.0, 5.0));
        }
        Icon::Minimize => line((-5.5, 2.5), (5.5, 2.5)),
        Icon::Maximize => path(&[
            (-5.5, -5.0),
            (5.5, -5.0),
            (5.5, 5.0),
            (-5.5, 5.0),
            (-5.5, -5.0),
        ]),
        Icon::Download => {
            line((0.0, -6.2), (0.0, 3.0));
            path(&[(-3.5, -0.2), (0.0, 3.4), (3.5, -0.2)]);
            path(&[(-5.8, 3.7), (-5.8, 6.0), (5.8, 6.0), (5.8, 3.7)]);
        }
        Icon::Upload => {
            line((0.0, 6.2), (0.0, -3.0));
            path(&[(-3.5, 0.2), (0.0, -3.4), (3.5, 0.2)]);
            path(&[(-5.8, -3.7), (-5.8, -6.0), (5.8, -6.0), (5.8, -3.7)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_icons_have_a_safe_custom_fallback() {
        assert_eq!(category("image"), Icon::Image);
        assert_eq!(category("video"), Icon::Video);
        assert_eq!(category("file"), Icon::File);
        assert_eq!(category("archive"), Icon::Archive);
        assert_eq!(category("music"), Icon::Music);
        assert_eq!(category("disc"), Icon::Disc);
        assert_eq!(category("custom"), Icon::Folder);
    }

    #[test]
    fn every_icon_emits_vector_shapes_in_both_sizes() {
        let icons = [
            Icon::Folder,
            Icon::Tag,
            Icon::Rss,
            Icon::Clock,
            Icon::Star,
            Icon::Pin,
            Icon::Image,
            Icon::Video,
            Icon::File,
            Icon::Archive,
            Icon::Music,
            Icon::Disc,
            Icon::Grid,
            Icon::Question,
            Icon::Link,
            Icon::Magnet,
            Icon::FilePlus,
            Icon::Pause,
            Icon::Play,
            Icon::Stop,
            Icon::Retry,
            Icon::Trash,
            Icon::Settings,
            Icon::Help,
            Icon::Command,
            Icon::Plus,
            Icon::Theme,
            Icon::Close,
            Icon::Minimize,
            Icon::Maximize,
            Icon::Download,
            Icon::Upload,
        ];
        let ctx = egui::Context::default();
        for size in [16.0, 18.0] {
            for icon in icons {
                ctx.begin_pass(egui::RawInput::default());
                let painter = ctx.layer_painter(egui::LayerId::background());
                paint(
                    &painter,
                    Rect::from_min_size(Pos2::ZERO, Vec2::splat(size)),
                    icon,
                    Color32::WHITE,
                );
                let output = ctx.end_pass();
                assert!(!output.shapes.is_empty(), "{icon:?} did not draw");
            }
        }
    }
}

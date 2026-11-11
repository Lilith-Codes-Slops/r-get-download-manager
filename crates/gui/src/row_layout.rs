//! Geometry and progress arithmetic shared by the native rows.

pub const TASK_COLUMNS: &[f32] = &[36.0, -1.0, 90.0];
pub const LIBRARY_COLUMNS: &[f32] = &[28.0, -1.0, 80.0, 100.0, 90.0];
pub const COLUMN_GAP: f32 = 10.0;
pub const SEGMENTS: usize = 8;

/// Return column offsets and widths. A negative width denotes the flexible
/// filename column; fixed columns retain their reference widths when possible.
/// On very narrow windows fixed tracks shrink proportionally to avoid overlap.
pub fn columns(width: f32, padding: f32, tracks: &[f32]) -> Vec<(f32, f32)> {
    let width = width.max(0.0);
    let padding = padding.min(width / 2.0).max(0.0);
    let inner = (width - 2.0 * padding).max(0.0);
    let gap = if tracks.len() > 1 {
        COLUMN_GAP.min(inner / (tracks.len() - 1) as f32)
    } else {
        0.0
    };
    let available = (inner - gap * tracks.len().saturating_sub(1) as f32).max(0.0);
    let fixed: f32 = tracks.iter().copied().filter(|x| *x >= 0.0).sum();
    let fixed_scale = if fixed > available && fixed > 0.0 {
        available / fixed
    } else {
        1.0
    };
    let flexible_count = tracks.iter().filter(|x| **x < 0.0).count().max(1) as f32;
    let flex = (available - fixed * fixed_scale).max(0.0) / flexible_count;
    let mut left = padding;
    tracks
        .iter()
        .map(|track| {
            let w = if *track < 0.0 {
                flex
            } else {
                *track * fixed_scale
            };
            let result = (left, w);
            left += w + gap;
            result
        })
        .collect()
}

pub fn progress_fraction(percent: f64) -> f32 {
    if percent.is_finite() {
        (percent.clamp(0.0, 100.0) / 100.0) as f32
    } else {
        0.0
    }
}

pub fn segment_fraction(percent: f64, segment: usize) -> f32 {
    (progress_fraction(percent) * SEGMENTS as f32 - segment as f32).clamp(0.0, 1.0)
}

pub fn format_eta(size_mb: f64, percent: f64, speed_kbps: f64) -> String {
    if speed_kbps < 1.0 || !speed_kbps.is_finite() || !size_mb.is_finite() {
        return "—".into();
    }
    let seconds = ((100.0 - percent) / 100.0 * size_mb * 1000.0 / speed_kbps).max(0.0);
    if seconds < 60.0 {
        format!("{:.0}s", seconds)
    } else if seconds < 3600.0 {
        format!(
            "{:.0}m {:.0}s",
            (seconds / 60.0).floor(),
            (seconds % 60.0).round()
        )
    } else {
        format!(
            "{:.0}h {:.0}m",
            (seconds / 3600.0).floor(),
            ((seconds % 3600.0) / 60.0).floor()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_grids_keep_fixed_columns_and_gaps() {
        assert_eq!(
            columns(500.0, 14.0, TASK_COLUMNS),
            vec![(14.0, 36.0), (60.0, 326.0), (396.0, 90.0)]
        );
        assert_eq!(
            columns(600.0, 14.0, LIBRARY_COLUMNS),
            vec![
                (14.0, 28.0),
                (52.0, 234.0),
                (296.0, 80.0),
                (386.0, 100.0),
                (496.0, 90.0)
            ]
        );
    }

    #[test]
    fn narrow_columns_never_overlap_or_overflow() {
        for width in [0.0, 20.0, 120.0, 350.0] {
            for tracks in [TASK_COLUMNS, LIBRARY_COLUMNS] {
                let cols = columns(width, 14.0, tracks);
                for &(x, w) in &cols {
                    assert!(x >= 0.0 && w >= 0.0 && x + w <= width + 0.001);
                }
                for pair in cols.windows(2) {
                    assert!(pair[0].0 + pair[0].1 <= pair[1].0 + 0.001);
                }
            }
        }
    }

    #[test]
    fn eta_matches_reference_units_and_unknown_speed() {
        assert_eq!(format_eta(1.0, 0.0, 0.0), "—");
        assert_eq!(format_eta(1.0, 50.0, 10.0), "50s");
        assert_eq!(format_eta(10.0, 0.0, 100.0), "1m 40s");
        assert_eq!(format_eta(100.0, 0.0, 10.0), "2h 46m");
    }

    #[test]
    fn eight_segments_track_full_partial_and_empty_progress() {
        assert_eq!(progress_fraction(-1.0), 0.0);
        assert_eq!(progress_fraction(101.0), 1.0);
        assert_eq!(progress_fraction(f64::NAN), 0.0);
        assert_eq!(segment_fraction(0.0, 0), 0.0);
        assert_eq!(segment_fraction(12.5, 0), 1.0);
        assert_eq!(segment_fraction(12.5, 1), 0.0);
        assert_eq!(segment_fraction(31.25, 2), 0.5);
        assert_eq!(segment_fraction(100.0, 7), 1.0);
    }
}

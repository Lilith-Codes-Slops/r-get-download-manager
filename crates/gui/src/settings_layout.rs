//! Pure layout decisions for the settings rows, in logical points.

pub const CONTROL_MAX_WIDTH: f32 = 236.0;
pub const TWO_COLUMN_MIN_WIDTH: f32 = 460.0;
pub const COLUMN_GAP: f32 = 20.0;
pub const SPEED_STACK_MIN_WIDTH: f32 = 164.0;

pub struct RowLayout {
    pub stacked: bool,
    pub label_width: f32,
    pub control_width: f32,
    pub control_height: f32,
}

impl RowLayout {
    pub fn new(width: f32, speed: bool) -> Self {
        let width = width.max(0.0);
        let stacked = width < TWO_COLUMN_MIN_WIDTH;
        let control_width = width.min(CONTROL_MAX_WIDTH);
        let label_width = if stacked {
            width
        } else {
            (width - control_width - COLUMN_GAP).max(0.0)
        };
        let control_height = if speed && control_width < SPEED_STACK_MIN_WIDTH {
            64.0
        } else {
            32.0
        };
        Self {
            stacked,
            label_width,
            control_width,
            control_height,
        }
    }

    pub fn inner_height(&self, text_height: f32) -> f32 {
        if self.stacked {
            text_height + 8.0 + self.control_height
        } else {
            text_height.max(self.control_height)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_rows_have_separate_columns_and_shared_right_edge() {
        for width in [460.0, 500.0, 600.0] {
            let row = RowLayout::new(width, true);
            assert!(!row.stacked);
            assert_eq!(row.control_width, CONTROL_MAX_WIDTH);
            assert_eq!(row.label_width + COLUMN_GAP + row.control_width, width);
            assert_eq!(row.inner_height(40.0), 40.0);
        }
    }

    #[test]
    fn narrow_rows_stack_without_overlapping_and_speed_wraps_only_when_needed() {
        for width in [0.0, 120.0, 163.0, 164.0, 240.0, 459.0] {
            let row = RowLayout::new(width, true);
            assert!(row.stacked);
            assert_eq!(row.label_width, width);
            assert!(row.control_width <= width);
            assert_eq!(
                row.control_height,
                if width < SPEED_STACK_MIN_WIDTH {
                    64.0
                } else {
                    32.0
                }
            );
            assert_eq!(row.inner_height(30.0), 30.0 + 8.0 + row.control_height);
        }
        assert_eq!(RowLayout::new(120.0, false).control_height, 32.0);
    }
}

//! Responsive toolbar regions; coordinates are relative to the content area.
const GAP: f32 = 10.0;
const ROW_HEIGHT: f32 = 36.0;
const ROW_STEP: f32 = 40.0;
const REGULAR_RIGHT: f32 = 280.0;
const COMPACT_RIGHT: f32 = 120.0;

#[derive(Debug)]
pub struct ToolbarLayout {
    pub compact_right: bool,
    pub right_x: f32,
    pub right_width: f32,
    pub search_x: f32,
    pub search_width: f32,
    pub search_row: usize,
    pub actions_width: f32,
    pub actions_row: usize,
    pub rows: usize,
}

impl ToolbarLayout {
    pub fn new(width: f32, tasks: bool) -> Self {
        let width = width.max(0.0);
        let compact_right = width < 390.0;
        let right_width = width.min(if compact_right {
            COMPACT_RIGHT
        } else {
            REGULAR_RIGHT
        });
        let right_x = width - right_width;
        let search_first = right_x >= 110.0;
        let search_row = usize::from(!search_first);
        let search_available = if search_first {
            (right_x - GAP).max(0.0)
        } else {
            width
        };
        let search_width = search_available.min(220.0);
        // Below this width the long action group gets its own scrollable row,
        // leaving the search and pinned controls accessible without overlap.
        let single_row_tasks = tasks && search_first && width >= 1350.0;
        let search_x = if single_row_tasks {
            (right_x - GAP - search_width).max(0.0)
        } else {
            ((search_available - search_width) / 2.0).max(0.0)
        };
        let actions_row = if single_row_tasks { 0 } else { search_row + 1 };
        let actions_width = if single_row_tasks {
            (search_x - GAP).max(0.0)
        } else {
            width
        };
        let rows = if tasks {
            actions_row + 1
        } else {
            search_row + 1
        };
        Self {
            compact_right,
            right_x,
            right_width,
            search_x,
            search_width,
            search_row,
            actions_width,
            actions_row,
            rows,
        }
    }

    pub fn height(&self) -> f32 {
        ROW_HEIGHT + (self.rows - 1) as f32 * ROW_STEP + 12.0
    }
    pub fn row_y(row: usize) -> f32 {
        row as f32 * ROW_STEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn right_controls_remain_at_the_right_at_all_widths() {
        for width in [180.0, 320.0, 600.0, 720.0, 1100.0, 1500.0] {
            for tasks in [false, true] {
                let layout = ToolbarLayout::new(width, tasks);
                assert_eq!(layout.right_x + layout.right_width, width);
                assert!(layout.search_x + layout.search_width <= width);
                if layout.search_row == 0 {
                    assert!(layout.search_x + layout.search_width <= layout.right_x);
                }
                if tasks && layout.actions_row == layout.search_row {
                    assert!(layout.actions_width <= layout.search_x);
                }
                assert!(
                    layout.rows > layout.search_row && (!tasks || layout.rows > layout.actions_row)
                );
            }
        }
    }
}

//! Responsive top-bar regions, relative to its horizontal content area.
const GAP: f32 = 10.0;
const REGULAR_RIGHT: f32 = 280.0;
const COMPACT_RIGHT: f32 = 120.0;
const LEFT_IDEAL: f32 = 320.0;

#[derive(Debug)]
pub struct ToolbarLayout {
    pub compact_right: bool,
    pub right_x: f32,
    pub right_width: f32,
    pub left_width: f32,
    pub search_x: f32,
    pub search_width: f32,
    pub search_row: usize,
}

impl ToolbarLayout {
    pub fn new(width: f32) -> Self {
        let width = width.max(0.0);
        let compact_right = width < 760.0;
        let right_width = width.min(if compact_right {
            COMPACT_RIGHT
        } else {
            REGULAR_RIGHT
        });
        let right_x = width - right_width;
        let search_width = width.min(220.0);
        let search_x = (width - search_width) / 2.0;
        // At smaller sizes search gets a second top-bar row; the navigation and
        // pinned utilities remain reachable independently rather than overlapping.
        let first_row = search_x >= LEFT_IDEAL + GAP && search_x + search_width + GAP <= right_x;
        let search_row = usize::from(!first_row);
        let left_width = if first_row {
            (search_x - GAP).max(0.0)
        } else {
            (right_x - GAP).max(0.0)
        };
        Self {
            compact_right,
            right_x,
            right_width,
            left_width,
            search_x,
            search_width,
            search_row,
        }
    }

    pub fn height(&self) -> f32 {
        52.0 + self.search_row as f32 * 40.0
    }
    pub fn search_y(&self) -> f32 {
        if self.search_row == 0 {
            11.0
        } else {
            56.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centered_search_and_pinned_utilities_never_overlap() {
        for width in [
            0.0, 100.0, 180.0, 320.0, 600.0, 720.0, 860.0, 1100.0, 1500.0,
        ] {
            let l = ToolbarLayout::new(width);
            assert_eq!(l.right_x + l.right_width, width);
            assert_eq!(l.search_x + l.search_width / 2.0, width / 2.0);
            assert!(l.left_width <= l.right_x);
            if l.search_row == 0 {
                assert!(l.left_width <= l.search_x);
                assert!(l.search_x + l.search_width <= l.right_x);
            }
            assert!(l.search_y() + 30.0 <= l.height());
        }
    }
}

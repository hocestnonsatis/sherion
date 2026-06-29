use winit::dpi::PhysicalSize;

/// Pixel-perfect terminal layout derived from the window size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalLayout {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    /// Horizontal offset where terminal content begins (right of tab strip).
    pub content_offset_x: f32,
    /// Vertical offset where terminal content begins (below title bar).
    pub content_offset_y: f32,
}

/// Axis-aligned content rectangle for a grid pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl ContentRect {
    pub fn width(self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(self) -> f32 {
        self.y1 - self.y0
    }
}

pub const GRID_GUTTER: f32 = 1.0;
pub const SPLIT_DIVIDER: f32 = 4.0;

/// Maximum number of tabs shown simultaneously in grid view.
pub const MAX_GRID_PANES: usize = 9;

impl TerminalLayout {
    pub fn grid_dims_changed(self, other: Self) -> bool {
        self.cols != other.cols || self.rows != other.rows
    }

    #[allow(dead_code)]
    pub fn display_metrics_changed(self, other: Self) -> bool {
        (self.cell_width - other.cell_width).abs() > f32::EPSILON
            || (self.cell_height - other.cell_height).abs() > f32::EPSILON
            || (self.content_offset_x - other.content_offset_x).abs() > f32::EPSILON
            || (self.content_offset_y - other.content_offset_y).abs() > f32::EPSILON
    }

    pub fn snapshot_from_window(
        size: PhysicalSize<u32>,
        scale_factor: f64,
        font_size: f32,
        content_offset_x: f32,
        content_offset_y: f32,
    ) -> Self {
        Self::from_pixels(
            size,
            scale_factor,
            font_size,
            content_offset_x,
            content_offset_y,
        )
    }

    pub fn from_pixels(
        size: PhysicalSize<u32>,
        scale_factor: f64,
        font_size: f32,
        content_offset_x: f32,
        content_offset_y: f32,
    ) -> Self {
        let content_width = (size.width as f32 - content_offset_x).max(1.0);
        let content_height = (size.height as f32 - content_offset_y).max(1.0);
        Self::from_rect(
            content_offset_x,
            content_offset_y,
            content_width,
            content_height,
            scale_factor,
            font_size,
        )
    }

    /// Build a layout for an arbitrary content rectangle (used for grid panes).
    pub fn from_rect(
        origin_x: f32,
        origin_y: f32,
        width: f32,
        height: f32,
        scale_factor: f64,
        font_size: f32,
    ) -> Self {
        let scale = scale_factor as f32;
        let cell_height = (font_size * 1.2 * scale).round().max(10.0);
        let cell_width = (font_size * 0.6 * scale).round().max(6.0);

        let content_width = width.max(cell_width);
        let content_height = height.max(cell_height);

        let cols = (content_width / cell_width).floor().max(1.0) as u16;
        let rows = (content_height / cell_height).floor().max(1.0) as u16;

        let cell_width = content_width / f32::from(cols);
        let cell_height = content_height / f32::from(rows);
        let font_size = font_size * scale;

        Self {
            cols,
            rows,
            cell_width,
            cell_height,
            font_size,
            content_offset_x: origin_x,
            content_offset_y: origin_y,
        }
    }

    pub fn pixel_bounds(self) -> ContentRect {
        ContentRect {
            x0: self.content_offset_x,
            y0: self.content_offset_y,
            x1: self.content_offset_x + self.cell_width * f32::from(self.cols),
            y1: self.content_offset_y + self.cell_height * f32::from(self.rows),
        }
    }
}

/// Column/row count for a dynamic grid holding `n` panes.
pub fn grid_dims(n: usize) -> (usize, usize) {
    if n == 0 {
        return (1, 1);
    }
    let cols = (n as f64).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);
    (cols, rows)
}

/// Pane rectangles over the content area. The last (partial) row stretches panes
/// across the full width.
pub fn pane_rects(
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
    n: usize,
) -> Vec<ContentRect> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![ContentRect {
            x0: content_x,
            y0: content_y,
            x1: content_x + content_w,
            y1: content_y + content_h,
        }];
    }

    let (cols, rows) = grid_dims(n);
    let gutter = GRID_GUTTER;
    let row_h = (content_h - gutter * (rows as f32 - 1.0)) / rows as f32;

    let mut rects = Vec::with_capacity(n);
    let mut pane_idx = 0;

    for row in 0..rows {
        let panes_in_row = if row == rows - 1 { n - pane_idx } else { cols };

        let row_y0 = content_y + row as f32 * (row_h + gutter);
        let row_y1 = row_y0 + row_h;
        let col_w = (content_w - gutter * (panes_in_row as f32 - 1.0)) / panes_in_row as f32;

        for col in 0..panes_in_row {
            let x0 = content_x + col as f32 * (col_w + gutter);
            rects.push(ContentRect {
                x0,
                y0: row_y0,
                x1: x0 + col_w,
                y1: row_y1,
            });
            pane_idx += 1;
            if pane_idx >= n {
                return rects;
            }
        }
    }

    rects
}

#[derive(Clone, Copy, Debug)]
pub struct SplitLayoutEntry {
    pub leaf_id: usize,
    pub rect: ContentRect,
}

#[derive(Clone, Copy, Debug)]
pub struct SplitDividerHit {
    pub index: usize,
    pub rect: ContentRect,
    pub direction: crate::split::SplitDirection,
    /// Inner dimension along the split axis (for ratio drag deltas).
    pub axis_span: f32,
}

/// Assign leaf rectangles for a split tree inside `rect`.
pub fn split_tree_rects(
    rect: ContentRect,
    node: &crate::split::SplitNode,
    leaf_id: &mut usize,
) -> Vec<SplitLayoutEntry> {
    use crate::split::{SplitDirection, SplitNode};

    match node {
        SplitNode::Pending => Vec::new(),
        SplitNode::Leaf { .. } => {
            let id = *leaf_id;
            *leaf_id += 1;
            vec![SplitLayoutEntry { leaf_id: id, rect }]
        }
        SplitNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let gutter = SPLIT_DIVIDER;
            match direction {
                SplitDirection::Horizontal => {
                    let inner_w = rect.width() - gutter;
                    let first_w = inner_w * ratio.clamp(0.1, 0.9);
                    let first_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0,
                        x1: rect.x0 + first_w,
                        y1: rect.y1,
                    };
                    let second_rect = ContentRect {
                        x0: rect.x0 + first_w + gutter,
                        y0: rect.y0,
                        x1: rect.x1,
                        y1: rect.y1,
                    };
                    let mut out = split_tree_rects(first_rect, first, leaf_id);
                    out.extend(split_tree_rects(second_rect, second, leaf_id));
                    out
                }
                SplitDirection::Vertical => {
                    let inner_h = rect.height() - gutter;
                    let first_h = inner_h * ratio.clamp(0.1, 0.9);
                    let first_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0,
                        x1: rect.x1,
                        y1: rect.y0 + first_h,
                    };
                    let second_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0 + first_h + gutter,
                        x1: rect.x1,
                        y1: rect.y1,
                    };
                    let mut out = split_tree_rects(first_rect, first, leaf_id);
                    out.extend(split_tree_rects(second_rect, second, leaf_id));
                    out
                }
            }
        }
    }
}

pub fn split_dividers(
    rect: ContentRect,
    node: &crate::split::SplitNode,
    divider_index: &mut usize,
) -> Vec<SplitDividerHit> {
    use crate::split::{SplitDirection, SplitNode};

    match node {
        SplitNode::Leaf { .. } | SplitNode::Pending => Vec::new(),
        SplitNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let gutter = SPLIT_DIVIDER;
            let mut out = Vec::new();
            let idx = *divider_index;
            *divider_index += 1;
            match direction {
                SplitDirection::Horizontal => {
                    let inner_w = rect.width() - gutter;
                    let first_w = inner_w * ratio.clamp(0.1, 0.9);
                    let divider_rect = ContentRect {
                        x0: rect.x0 + first_w,
                        y0: rect.y0,
                        x1: rect.x0 + first_w + gutter,
                        y1: rect.y1,
                    };
                    out.push(SplitDividerHit {
                        index: idx,
                        rect: divider_rect,
                        direction: SplitDirection::Horizontal,
                        axis_span: inner_w,
                    });
                    let first_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0,
                        x1: rect.x0 + first_w,
                        y1: rect.y1,
                    };
                    let second_rect = ContentRect {
                        x0: rect.x0 + first_w + gutter,
                        y0: rect.y0,
                        x1: rect.x1,
                        y1: rect.y1,
                    };
                    out.extend(split_dividers(first_rect, first, divider_index));
                    out.extend(split_dividers(second_rect, second, divider_index));
                }
                SplitDirection::Vertical => {
                    let inner_h = rect.height() - gutter;
                    let first_h = inner_h * ratio.clamp(0.1, 0.9);
                    let divider_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0 + first_h,
                        x1: rect.x1,
                        y1: rect.y0 + first_h + gutter,
                    };
                    out.push(SplitDividerHit {
                        index: idx,
                        rect: divider_rect,
                        direction: SplitDirection::Vertical,
                        axis_span: inner_h,
                    });
                    let first_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0,
                        x1: rect.x1,
                        y1: rect.y0 + first_h,
                    };
                    let second_rect = ContentRect {
                        x0: rect.x0,
                        y0: rect.y0 + first_h + gutter,
                        x1: rect.x1,
                        y1: rect.y1,
                    };
                    out.extend(split_dividers(first_rect, first, divider_index));
                    out.extend(split_dividers(second_rect, second, divider_index));
                }
            }
            out
        }
    }
}

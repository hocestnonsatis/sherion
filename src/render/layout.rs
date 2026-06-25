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
        let scale = scale_factor as f32;
        let cell_height = (font_size * 1.2 * scale).round().max(10.0);
        let cell_width = (font_size * 0.6 * scale).round().max(6.0);

        let content_width = (size.width as f32 - content_offset_x).max(cell_width);
        let content_height = (size.height as f32 - content_offset_y).max(cell_height);

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
            content_offset_x,
            content_offset_y,
        }
    }
}

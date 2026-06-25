use std::ops::Range;

use vello::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii};
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::tab_bar::TabBarEntry;
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;
use crate::tabs::TabId;

const TAB_ROW_HEIGHT: f64 = 30.0;
const NEW_TAB_HEIGHT: f64 = 36.0;
const TAB_PADDING: f64 = 10.0;
const CLOSE_SIZE: f64 = 18.0;
const CLOSE_MARGIN: f64 = 6.0;
const TOGGLE_HEIGHT: f64 = 30.0;

pub const SIDEBAR_MIN_WIDTH: f32 = 96.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 420.0;
/// Width of the sidebar when collapsed to icon-only mode.
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 48.0;

#[derive(Clone, Copy, Debug)]
pub struct TabStripMetrics {
    pub width: f32,
}

impl TabStripMetrics {
    pub fn from_font_size(font_size: f32, scale: f32) -> Self {
        let width = (font_size * 9.0 * scale).round().clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        Self { width }
    }

    pub fn row_height(&self) -> f64 {
        TAB_ROW_HEIGHT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStripHit {
    Tab(TabId),
    Close(TabId),
    NewTab,
    ToggleCollapse,
    None,
}

pub struct TabStripLayout {
    pub metrics: TabStripMetrics,
    /// Top edge (y) where the sidebar begins, i.e. below the full-width title bar.
    pub top: f64,
    pub width: f64,
    pub height: f64,
    /// Whether the sidebar is rendered in icon-only (collapsed) mode.
    pub collapsed: bool,
    /// "New tab" button, pinned to the top of the sidebar.
    pub new_tab_region: Rect,
    /// Top edge (y) of the scrollable tab list (below the new-tab button).
    pub list_top: f64,
    pub list_height: f64,
    /// Collapse/expand toggle button, pinned to the bottom of the sidebar.
    pub collapse_region: Rect,
    /// Tab rows in content space (0-based from the list top, before scroll).
    pub tab_regions: Vec<(TabId, Range<f64>)>,
    pub content_height: f64,
    /// Left edge (x) of the per-tab close button hit zone.
    pub close_x_start: f64,
}

impl TabStripLayout {
    pub fn compute(
        metrics: TabStripMetrics,
        top: f64,
        height: f64,
        entries: &[TabBarEntry],
        collapsed: bool,
    ) -> Self {
        let width = f64::from(metrics.width);
        let height = height.max(0.0);
        let new_tab_region = Rect::new(0.0, top, width, top + NEW_TAB_HEIGHT);
        let list_top = top + NEW_TAB_HEIGHT;
        let bottom = top + height;
        let collapse_top = (bottom - TOGGLE_HEIGHT).max(list_top);
        let collapse_region = Rect::new(0.0, collapse_top, width, bottom);
        let list_height = (collapse_top - list_top).max(0.0);

        let mut y = 0.0;
        let mut tab_regions = Vec::with_capacity(entries.len());
        for entry in entries {
            tab_regions.push((entry.id, y..y + TAB_ROW_HEIGHT));
            y += TAB_ROW_HEIGHT;
        }
        let content_height = y;
        let close_x_start = width - CLOSE_SIZE - CLOSE_MARGIN;

        Self {
            metrics,
            top,
            width,
            height,
            collapsed,
            new_tab_region,
            list_top,
            list_height,
            collapse_region,
            tab_regions,
            content_height,
            close_x_start,
        }
    }

    pub fn list_bottom(&self) -> f64 {
        self.list_top + self.list_height
    }

    pub fn max_scroll_offset(&self) -> f64 {
        (self.content_height - self.list_height).max(0.0)
    }

    pub fn hit_test(&self, x: f64, y: f64, scroll_offset: f64) -> TabStripHit {
        if x < 0.0 || x >= self.width || y < self.top || y >= self.top + self.height {
            return TabStripHit::None;
        }
        if self.new_tab_region.contains((x, y)) {
            return TabStripHit::NewTab;
        }
        if self.collapse_region.contains((x, y)) {
            return TabStripHit::ToggleCollapse;
        }
        if y < self.list_top || y >= self.list_bottom() {
            return TabStripHit::None;
        }

        let content_y = (y - self.list_top) + scroll_offset;
        for (id, range) in &self.tab_regions {
            if range.contains(&content_y) {
                if !self.collapsed && x >= self.close_x_start && x <= self.width - CLOSE_MARGIN {
                    return TabStripHit::Close(*id);
                }
                return TabStripHit::Tab(*id);
            }
        }
        TabStripHit::None
    }

    pub fn active_tab_range(&self, active_id: TabId) -> Option<Range<f64>> {
        self.tab_regions
            .iter()
            .find(|(id, _)| *id == active_id)
            .map(|(_, range)| range.clone())
    }
}

pub struct TabStripRenderer;

impl TabStripRenderer {
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        scene: &mut Scene,
        layout: &TabStripLayout,
        entries: &[TabBarEntry],
        scroll_offset: f64,
        config: &Config,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        font_size: f32,
    ) {
        let theme = UiTheme::from_config(config);
        let bg = theme.chrome_brush();
        let active_bg = theme.surface_brush();
        let inactive_bg = theme.tab_inactive_brush();
        let fg = theme.foreground_brush();
        let accent = theme.accent_brush();
        let family = font_family_from_config(config);
        let label_font = (font_size * 0.82).max(10.0);
        let collapsed = layout.collapsed;
        let label_max_width = (layout.close_x_start - TAB_PADDING - 2.0).max(1.0) as f32;
        let list_bottom = layout.list_bottom();

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &bg,
            None,
            &Rect::new(0.0, layout.top, layout.width, layout.top + layout.height),
        );

        // "New tab" button pinned at the top of the sidebar.
        let new_rect = RoundedRect::new(
            4.0,
            layout.new_tab_region.y0 + 4.0,
            layout.width - 4.0,
            layout.new_tab_region.y1 - 4.0,
            RoundedRectRadii::from_single_radius(4.0),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &inactive_bg, None, &new_rect);
        let plus_y = layout.new_tab_region.y0
            + (layout.new_tab_region.height() - f64::from(label_font)) * 0.5;
        if collapsed {
            draw_centered_text(
                layout_cx,
                scene,
                font_cx,
                "+",
                layout.width,
                plus_y,
                label_font,
                family.clone(),
                &fg,
            );
        } else {
            draw_text(
                layout_cx,
                scene,
                font_cx,
                "+ New Tab",
                TAB_PADDING,
                plus_y,
                label_font,
                family.clone(),
                &fg,
                Some((layout.width - TAB_PADDING * 2.0).max(1.0) as f32),
            );
        }

        for (index, (id, range)) in layout.tab_regions.iter().enumerate() {
            let Some(entry) = entries.iter().find(|e| e.id == *id) else {
                continue;
            };

            let y0 = layout.list_top + range.start - scroll_offset;
            let y1 = layout.list_top + range.end - scroll_offset;
            if y1 <= layout.list_top || y0 >= list_bottom {
                continue;
            }

            let draw_top = y0.max(layout.list_top);
            let draw_bottom = y1.min(list_bottom);
            let tab_bg = if entry.active { &active_bg } else { &inactive_bg };

            let radii = if entry.active {
                RoundedRectRadii::new(0.0, 4.0, 4.0, 0.0)
            } else {
                RoundedRectRadii::from_single_radius(4.0)
            };
            let rect = RoundedRect::new(
                2.0,
                draw_top + 2.0,
                layout.width - 2.0,
                draw_bottom - 2.0,
                radii,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, tab_bg, None, &rect);

            if entry.active {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &accent,
                    None,
                    &Rect::new(layout.width - 2.0, draw_top + 2.0, layout.width, draw_bottom - 2.0),
                );
            }

            if collapsed {
                // Icon-only mode: show the 1-based tab index centered in the row.
                let icon = (index + 1).to_string();
                let icon_y = draw_top + ((draw_bottom - draw_top) - f64::from(label_font)) * 0.5;
                draw_centered_text(
                    layout_cx,
                    scene,
                    font_cx,
                    &icon,
                    layout.width,
                    icon_y.max(draw_top + 3.0),
                    label_font,
                    family.clone(),
                    &fg,
                );
                continue;
            }

            let max_chars = (label_max_width / 7.0).max(3.0) as usize;
            let label = truncate_title(&entry.title, max_chars);
            let label_y = draw_top + ((draw_bottom - draw_top) - f64::from(label_font)) * 0.5;

            draw_text(
                layout_cx,
                scene,
                font_cx,
                &label,
                TAB_PADDING,
                label_y.max(draw_top + 3.0),
                label_font,
                family.clone(),
                &fg,
                Some(label_max_width),
            );

            // Close button (×) on the right side of the row.
            let close_center_y = (draw_top + draw_bottom) * 0.5;
            if close_center_y > layout.list_top && close_center_y < list_bottom {
                let close_y = close_center_y - f64::from(label_font) * 0.5;
                draw_text(
                    layout_cx,
                    scene,
                    font_cx,
                    "×",
                    layout.close_x_start + 2.0,
                    close_y.max(draw_top + 3.0),
                    label_font,
                    family.clone(),
                    &fg,
                    Some(CLOSE_SIZE as f32),
                );
            }
        }

        // Collapse/expand toggle pinned to the bottom of the sidebar.
        let toggle = &layout.collapse_region;
        let toggle_rect = RoundedRect::new(
            4.0,
            toggle.y0 + 4.0,
            layout.width - 4.0,
            toggle.y1 - 4.0,
            RoundedRectRadii::from_single_radius(4.0),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &inactive_bg, None, &toggle_rect);
        let toggle_icon = if collapsed { "»" } else { "« Collapse" };
        let toggle_y = toggle.y0 + (toggle.height() - f64::from(label_font)) * 0.5;
        if collapsed {
            draw_centered_text(
                layout_cx,
                scene,
                font_cx,
                toggle_icon,
                layout.width,
                toggle_y.max(toggle.y0 + 3.0),
                label_font,
                family.clone(),
                &fg,
            );
        } else {
            draw_text(
                layout_cx,
                scene,
                font_cx,
                toggle_icon,
                TAB_PADDING,
                toggle_y.max(toggle.y0 + 3.0),
                label_font,
                family.clone(),
                &fg,
                Some((layout.width - TAB_PADDING * 2.0).max(1.0) as f32),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_centered_text(
    layout_cx: &mut parley::LayoutContext<Brush>,
    scene: &mut Scene,
    font_cx: &mut parley::FontContext,
    text: &str,
    container_width: f64,
    y: f64,
    font_size: f32,
    family: parley::FontFamily<'static>,
    brush: &Brush,
) {
    // Approximate glyph advance for monospace-ish centering of a short icon string.
    let approx = f64::from(font_size) * 0.6 * text.chars().count() as f64;
    let x = ((container_width - approx) * 0.5).max(2.0);
    draw_text(
        layout_cx,
        scene,
        font_cx,
        text,
        x,
        y,
        font_size,
        family,
        brush,
        Some(container_width.max(1.0) as f32),
    );
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(3);
    let char_count = title.chars().count();
    if char_count <= max_chars {
        return title.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", title.chars().take(keep).collect::<String>())
}

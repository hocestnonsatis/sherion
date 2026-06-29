use std::ops::Range;
use std::time::Duration;

use vello::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::tab_bar::TabBarEntry;
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;
use crate::tabs::TabId;

// All sidebar dimensions below are expressed in *logical* pixels and scaled by
// the window's DPI factor at use sites, so the sidebar stays proportional and
// readable on HiDPI displays instead of collapsing into tiny rows.

/// Minimum readable UI font for sidebar labels (logical px).
const UI_FONT_MIN: f32 = 12.0;
/// Cap so an extreme terminal zoom doesn't blow the sidebar font out.
const UI_FONT_MAX: f32 = 17.0;
/// Sidebar label size relative to the terminal font.
const UI_FONT_RATIO: f32 = 0.95;

/// Comfortable tab row height as a multiple of the label font, with a
/// touch-friendly minimum (logical px).
const ROW_HEIGHT_RATIO: f64 = 2.3;
const ROW_HEIGHT_MIN: f64 = 38.0;
const NEW_TAB_HEIGHT_RATIO: f64 = 2.7;
const NEW_TAB_HEIGHT_MIN: f64 = 44.0;
const TOGGLE_HEIGHT_RATIO: f64 = 2.3;
const TOGGLE_HEIGHT_MIN: f64 = 38.0;

const PADDING_RATIO: f64 = 0.9;
const PADDING_MIN: f64 = 12.0;
const ICON_RATIO: f64 = 1.25;
const ICON_MIN: f64 = 18.0;
const ICON_MARGIN: f64 = 6.0;
const CORNER_RADIUS: f64 = 5.0;
const ROW_INSET: f64 = 3.0;

/// Sidebar width sizing relative to the UI font (logical px).
const WIDTH_FONT_RATIO: f32 = 11.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 150.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 480.0;
/// Width of the sidebar when collapsed to icon-only mode (logical px).
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 52.0;

/// Logical UI font for the sidebar derived from the terminal font.
fn ui_font_logical(font_size: f32) -> f32 {
    (font_size * UI_FONT_RATIO).clamp(UI_FONT_MIN, UI_FONT_MAX)
}

/// Scale-aware clamp bounds for the resizable sidebar width (physical px).
pub fn sidebar_width_bounds(scale: f32) -> (f32, f32) {
    let scale = scale.max(1.0);
    (SIDEBAR_MIN_WIDTH * scale, SIDEBAR_MAX_WIDTH * scale)
}

/// Collapsed sidebar width in physical px for the given scale.
pub fn collapsed_sidebar_width(scale: f32) -> f32 {
    (SIDEBAR_COLLAPSED_WIDTH * scale.max(1.0)).round()
}

#[derive(Clone, Copy, Debug)]
pub struct TabStripMetrics {
    pub width: f32,
    scale: f32,
    /// Logical (unscaled) UI font size for sidebar labels.
    ui_font: f32,
}

impl TabStripMetrics {
    pub fn from_font_size(font_size: f32, scale: f32) -> Self {
        let scale = scale.max(1.0);
        let ui_font = ui_font_logical(font_size);
        let (min_w, max_w) = sidebar_width_bounds(scale);
        let width = (ui_font * WIDTH_FONT_RATIO * scale)
            .round()
            .clamp(min_w, max_w);
        Self {
            width,
            scale,
            ui_font,
        }
    }

    fn scaled(&self, value: f64) -> f64 {
        value * f64::from(self.scale)
    }

    /// Physical-pixel label font for sidebar text.
    pub fn label_font(&self) -> f32 {
        (self.ui_font * self.scale).round().max(11.0)
    }

    pub fn row_height(&self) -> f64 {
        self.scaled((f64::from(self.ui_font) * ROW_HEIGHT_RATIO).max(ROW_HEIGHT_MIN))
    }

    pub fn new_tab_height(&self) -> f64 {
        self.scaled((f64::from(self.ui_font) * NEW_TAB_HEIGHT_RATIO).max(NEW_TAB_HEIGHT_MIN))
    }

    pub fn toggle_height(&self) -> f64 {
        self.scaled((f64::from(self.ui_font) * TOGGLE_HEIGHT_RATIO).max(TOGGLE_HEIGHT_MIN))
    }

    pub fn padding(&self) -> f64 {
        self.scaled((f64::from(self.ui_font) * PADDING_RATIO).max(PADDING_MIN))
    }

    pub fn icon_size(&self) -> f64 {
        self.scaled((f64::from(self.ui_font) * ICON_RATIO).max(ICON_MIN))
    }

    pub fn icon_margin(&self) -> f64 {
        self.scaled(ICON_MARGIN)
    }

    pub fn corner_radius(&self) -> f64 {
        self.scaled(CORNER_RADIUS)
    }

    pub fn row_inset(&self) -> f64 {
        self.scaled(ROW_INSET)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStripHit {
    Tab(TabId),
    Close(TabId),
    Detach(TabId),
    NewTab,
    ToggleCollapse,
    None,
}

pub struct TabStripLayout {
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
    /// Left edge (x) of the per-tab detach button hit zone.
    pub detach_x_start: f64,
    /// Left edge (x) of the per-tab close button hit zone.
    pub close_x_start: f64,
    /// Derived, scale-aware sizing shared by hit-testing and rendering.
    pub label_font: f32,
    pub padding: f64,
    pub icon_size: f64,
    pub icon_margin: f64,
    pub corner_radius: f64,
    pub row_inset: f64,
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
        let row_height = metrics.row_height();
        let new_tab_height = metrics.new_tab_height();
        let toggle_height = metrics.toggle_height();
        let icon_size = metrics.icon_size();
        let icon_margin = metrics.icon_margin();

        let new_tab_region = Rect::new(0.0, top, width, top + new_tab_height);
        let list_top = top + new_tab_height;
        let bottom = top + height;
        let collapse_top = (bottom - toggle_height).max(list_top);
        let collapse_region = Rect::new(0.0, collapse_top, width, bottom);
        let list_height = (collapse_top - list_top).max(0.0);

        let mut y = 0.0;
        let mut tab_regions = Vec::with_capacity(entries.len());
        for entry in entries {
            tab_regions.push((entry.id, y..y + row_height));
            y += row_height;
        }
        let content_height = y;
        let close_x_start = width - icon_size - icon_margin;
        let detach_x_start = close_x_start - icon_size - icon_margin;

        Self {
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
            detach_x_start,
            close_x_start,
            label_font: metrics.label_font(),
            padding: metrics.padding(),
            icon_size,
            icon_margin,
            corner_radius: metrics.corner_radius(),
            row_inset: metrics.row_inset(),
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
                if !self.collapsed {
                    if x >= self.close_x_start && x <= self.width - self.icon_margin {
                        return TabStripHit::Close(*id);
                    }
                    if x >= self.detach_x_start && x < self.close_x_start {
                        return TabStripHit::Detach(*id);
                    }
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
    ) {
        let theme = UiTheme::from_config(config);
        let bg = theme.chrome_brush();
        let active_bg = theme.surface_brush();
        let inactive_bg = theme.tab_inactive_brush();
        let fg = theme.foreground_brush();
        let muted = theme.muted_brush();
        let accent = theme.accent_brush();
        let family = font_family_from_config(config);
        let label_font = layout.label_font;
        let padding = layout.padding;
        let inset = layout.row_inset;
        let radius = layout.corner_radius;
        let collapsed = layout.collapsed;
        let label_max_width = (layout.detach_x_start - padding - inset).max(1.0) as f32;
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
            inset,
            layout.new_tab_region.y0 + inset,
            layout.width - inset,
            layout.new_tab_region.y1 - inset,
            RoundedRectRadii::from_single_radius(radius),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &inactive_bg,
            None,
            &new_rect,
        );
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
                padding,
                plus_y,
                label_font,
                family.clone(),
                &fg,
                Some((layout.width - padding * 2.0).max(1.0) as f32),
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
            let tab_bg = if entry.active {
                &active_bg
            } else {
                &inactive_bg
            };

            let radii = if entry.active {
                RoundedRectRadii::new(0.0, radius, radius, 0.0)
            } else {
                RoundedRectRadii::from_single_radius(radius)
            };
            let rect = RoundedRect::new(
                inset,
                draw_top + inset,
                layout.width - inset,
                draw_bottom - inset,
                radii,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, tab_bg, None, &rect);

            if let Some([r, g, b]) = entry.accent {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &Brush::Solid(AlphaColor::from_rgb8(r, g, b)),
                    None,
                    &Rect::new(inset, draw_top + inset, inset + 3.0, draw_bottom - inset),
                );
            }

            if entry.active {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &accent,
                    None,
                    &Rect::new(
                        layout.width - inset,
                        draw_top + inset,
                        layout.width,
                        draw_bottom - inset,
                    ),
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
                    icon_y.max(draw_top + inset),
                    label_font,
                    family.clone(),
                    &fg,
                );
                continue;
            }

            let row_h = draw_bottom - draw_top;
            let subtitle_font = (label_font * 0.78).max(9.0);
            let has_subtitle = entry.cwd.as_ref().is_some_and(|cwd| !cwd.is_empty())
                && row_h >= f64::from(label_font + subtitle_font) + inset * 3.0;
            let label_y = if has_subtitle {
                draw_top + inset + 4.0
            } else {
                draw_top + (row_h - f64::from(label_font)) * 0.5
            };
            let mut text_max_width = label_max_width;
            let glyph_advance = f64::from(label_font) * 0.55;

            // Show the elapsed running time between the detach and close buttons.
            if entry.busy {
                if let Some(elapsed) = entry.elapsed {
                    let time_text = format_elapsed(elapsed);
                    let char_count = time_text.chars().count() as f64;
                    let time_w = glyph_advance * char_count + inset * 2.0;
                    let time_x = (layout.detach_x_start - inset - time_w).max(padding);
                    text_max_width = ((time_x - padding) as f32).clamp(20.0, label_max_width);
                    draw_text(
                        layout_cx,
                        scene,
                        font_cx,
                        &time_text,
                        time_x,
                        label_y.max(draw_top + inset),
                        label_font,
                        family.clone(),
                        &accent,
                        Some(time_w as f32 + 2.0),
                    );
                }
            }

            let max_chars = (f64::from(text_max_width) / glyph_advance.max(1.0)).max(3.0) as usize;
            let label = if entry.pinned {
                format!(
                    "📌 {}",
                    truncate_title(&entry.title, max_chars.saturating_sub(2))
                )
            } else {
                truncate_title(&entry.title, max_chars)
            };

            draw_text(
                layout_cx,
                scene,
                font_cx,
                &label,
                padding,
                label_y.max(draw_top + inset),
                label_font,
                family.clone(),
                &fg,
                Some(text_max_width),
            );

            if has_subtitle {
                if let Some(cwd) = entry.cwd.as_ref() {
                    let subtitle_y = (label_y + f64::from(label_font) * 0.9)
                        .min(draw_bottom - f64::from(subtitle_font) - inset);
                    let subtitle_chars = (f64::from(text_max_width)
                        / (f64::from(subtitle_font) * 0.52).max(1.0))
                    .max(3.0) as usize;
                    let subtitle = truncate_title(cwd, subtitle_chars);
                    draw_text(
                        layout_cx,
                        scene,
                        font_cx,
                        &subtitle,
                        padding,
                        subtitle_y.max(draw_top + inset),
                        subtitle_font,
                        family.clone(),
                        &muted,
                        Some(text_max_width),
                    );
                }
            }

            let close_center_y = (draw_top + draw_bottom) * 0.5;
            if close_center_y > layout.list_top && close_center_y < list_bottom {
                let icon_y = close_center_y - f64::from(label_font) * 0.5;
                draw_text(
                    layout_cx,
                    scene,
                    font_cx,
                    "↗",
                    layout.detach_x_start + 1.0,
                    icon_y.max(draw_top + inset),
                    label_font,
                    family.clone(),
                    &fg,
                    Some(layout.icon_size as f32),
                );
                draw_text(
                    layout_cx,
                    scene,
                    font_cx,
                    "×",
                    layout.close_x_start + 2.0,
                    icon_y.max(draw_top + inset),
                    label_font,
                    family.clone(),
                    &fg,
                    Some(layout.icon_size as f32),
                );
            }
        }

        // Collapse/expand toggle pinned to the bottom of the sidebar.
        let toggle = &layout.collapse_region;
        let toggle_rect = RoundedRect::new(
            inset,
            toggle.y0 + inset,
            layout.width - inset,
            toggle.y1 - inset,
            RoundedRectRadii::from_single_radius(radius),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &inactive_bg,
            None,
            &toggle_rect,
        );
        let toggle_icon = if collapsed { "»" } else { "« Collapse" };
        let toggle_y = toggle.y0 + (toggle.height() - f64::from(label_font)) * 0.5;
        if collapsed {
            draw_centered_text(
                layout_cx,
                scene,
                font_cx,
                toggle_icon,
                layout.width,
                toggle_y.max(toggle.y0 + inset),
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
                padding,
                toggle_y.max(toggle.y0 + inset),
                label_font,
                family.clone(),
                &fg,
                Some((layout.width - padding * 2.0).max(1.0) as f32),
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

/// Format a running duration compactly: `12s`, `1:05`, or `1:02:03`.
fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    if total < 60 {
        format!("{total}s")
    } else if total < 3600 {
        format!("{}:{:02}", total / 60, total % 60)
    } else {
        format!(
            "{}:{:02}:{:02}",
            total / 3600,
            (total % 3600) / 60,
            total % 60
        )
    }
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

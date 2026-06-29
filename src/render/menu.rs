use std::ops::Range;

use vello::kurbo::{
    Affine, Circle, CircleSegment, Point, Rect, RoundedRect, RoundedRectRadii, Stroke,
};
use vello::peniko::color::{AlphaColor, Srgb};
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::{Config, ThemeMode};
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;
use crate::window_state::{ViewMode, OPACITY_MAX, OPACITY_MIN};

const MENU_WIDTH: f64 = 220.0;
const MENU_ITEM_HEIGHT: f64 = 32.0;
const MENU_SEPARATOR_HEIGHT: f64 = 9.0;
const MENU_PADDING: f64 = 4.0;
const MENU_SIDE_PADDING: f64 = 12.0;

const THEME_ROW_HEIGHT: f64 = 42.0;
const THEME_CIRCLE_RADIUS: f64 = 11.0;
const THEME_CIRCLE_GAP: f64 = 12.0;

const VIEW_ROW_HEIGHT: f64 = 42.0;
const VIEW_ICON_SIZE: f64 = 18.0;
const VIEW_ICON_GAP: f64 = 14.0;

const OPACITY_ROW_HEIGHT: f64 = 50.0;
const SLIDER_TRACK_HEIGHT: f64 = 5.0;
const SLIDER_KNOB_RADIUS: f64 = 7.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewTab,
    DuplicateTab,
    CloseTab,
    DetachTab,
    RenameTab,
    Copy,
    Paste,
    ClearScrollback,
    ThemeLight,
    ThemeDark,
    ThemeAuto,
    ViewSingle,
    ViewGrid,
    TogglePerfOverlay,
    ToggleFullscreen,
    ToggleFollowOutput,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTabPin,
    CycleTabColor,
    ToggleAlwaysOnTop,
    Quit,
}

/// Result of clicking inside the open menu.
#[derive(Clone, Copy, Debug)]
pub enum MenuHit {
    /// A discrete menu action was triggered.
    Action(MenuAction),
    /// The opacity slider was grabbed/moved; carries the resulting opacity value.
    Opacity(f32),
    /// The click landed on an interactive row but produced no action; keep the
    /// menu open and swallow the click.
    Keep,
}

#[derive(Clone, Debug)]
pub enum MenuEntry {
    Action { action: MenuAction, label: String },
    ViewSelector { current: ViewMode },
    ThemeSelector { current: ThemeMode },
    OpacitySlider { value: f32 },
    Separator,
}

#[derive(Clone, Copy)]
struct ViewIconGeom {
    action: MenuAction,
    center: Point,
}

#[derive(Clone)]
struct ViewRowGeom {
    y: Range<f64>,
    icons: Vec<ViewIconGeom>,
}

#[derive(Clone, Copy)]
struct ThemeCircleGeom {
    action: MenuAction,
    center: Point,
}

#[derive(Clone)]
struct ThemeRowGeom {
    y: Range<f64>,
    circles: Vec<ThemeCircleGeom>,
}

#[derive(Clone)]
struct SliderRowGeom {
    y: Range<f64>,
    track_x0: f64,
    track_x1: f64,
}

pub struct MenuLayout {
    pub bounds: Rect,
    item_regions: Vec<(MenuAction, Range<f64>)>,
    theme_row: Option<ThemeRowGeom>,
    view_row: Option<ViewRowGeom>,
    slider_row: Option<SliderRowGeom>,
}

impl MenuLayout {
    pub fn compute(anchor_x: f64, anchor_y: f64, width: f64, entries: &[MenuEntry]) -> Self {
        let menu_height = MENU_PADDING * 2.0 + entries.iter().map(entry_height).sum::<f64>();
        let mut x = anchor_x;
        if x + MENU_WIDTH > width {
            x = (width - MENU_WIDTH).max(0.0);
        }

        let bounds = Rect::new(x, anchor_y, x + MENU_WIDTH, anchor_y + menu_height);
        let mut item_regions = Vec::new();
        let mut theme_row = None;
        let mut view_row = None;
        let mut slider_row = None;
        let mut y = anchor_y + MENU_PADDING;
        for entry in entries {
            match entry {
                MenuEntry::Action { action, .. } => {
                    item_regions.push((*action, y..y + MENU_ITEM_HEIGHT));
                    y += MENU_ITEM_HEIGHT;
                }
                MenuEntry::ViewSelector { .. } => {
                    let centers = view_icon_centers(&bounds, y);
                    let icons = vec![
                        ViewIconGeom {
                            action: MenuAction::ViewSingle,
                            center: centers[0],
                        },
                        ViewIconGeom {
                            action: MenuAction::ViewGrid,
                            center: centers[1],
                        },
                    ];
                    view_row = Some(ViewRowGeom {
                        y: y..y + VIEW_ROW_HEIGHT,
                        icons,
                    });
                    y += VIEW_ROW_HEIGHT;
                }
                MenuEntry::ThemeSelector { .. } => {
                    let centers = theme_circle_centers(&bounds, y);
                    let circles = vec![
                        ThemeCircleGeom {
                            action: MenuAction::ThemeLight,
                            center: centers[0],
                        },
                        ThemeCircleGeom {
                            action: MenuAction::ThemeDark,
                            center: centers[1],
                        },
                        ThemeCircleGeom {
                            action: MenuAction::ThemeAuto,
                            center: centers[2],
                        },
                    ];
                    theme_row = Some(ThemeRowGeom {
                        y: y..y + THEME_ROW_HEIGHT,
                        circles,
                    });
                    y += THEME_ROW_HEIGHT;
                }
                MenuEntry::OpacitySlider { .. } => {
                    let (track_x0, track_x1, _track_cy) = slider_track(&bounds, y);
                    slider_row = Some(SliderRowGeom {
                        y: y..y + OPACITY_ROW_HEIGHT,
                        track_x0,
                        track_x1,
                    });
                    y += OPACITY_ROW_HEIGHT;
                }
                MenuEntry::Separator => {
                    y += MENU_SEPARATOR_HEIGHT;
                }
            }
        }

        Self {
            bounds,
            item_regions,
            theme_row,
            view_row,
            slider_row,
        }
    }

    pub fn hit_test(&self, x: f64, y: f64) -> Option<MenuHit> {
        if !self.bounds.contains((x, y)) {
            return None;
        }
        for (action, range) in &self.item_regions {
            if range.contains(&y) {
                return Some(MenuHit::Action(*action));
            }
        }
        if let Some(view) = &self.view_row {
            if view.y.contains(&y) {
                let half = VIEW_ICON_SIZE * 0.5 + 5.0;
                for icon in &view.icons {
                    let dx = x - icon.center.x;
                    let dy = y - icon.center.y;
                    if dx.abs() <= half && dy.abs() <= half {
                        return Some(MenuHit::Action(icon.action));
                    }
                }
                return Some(MenuHit::Keep);
            }
        }
        if let Some(theme) = &self.theme_row {
            if theme.y.contains(&y) {
                let hit_radius = THEME_CIRCLE_RADIUS + 5.0;
                for circle in &theme.circles {
                    let dx = x - circle.center.x;
                    let dy = y - circle.center.y;
                    if dx * dx + dy * dy <= hit_radius * hit_radius {
                        return Some(MenuHit::Action(circle.action));
                    }
                }
                return Some(MenuHit::Keep);
            }
        }
        if let Some(slider) = &self.slider_row {
            if slider.y.contains(&y) {
                return Some(MenuHit::Opacity(opacity_from_fraction(fraction_at(
                    slider, x,
                ))));
            }
        }
        None
    }

    /// Map an x coordinate onto the opacity track, used while dragging the knob.
    pub fn opacity_from_x(&self, x: f64) -> Option<f32> {
        let slider = self.slider_row.as_ref()?;
        Some(opacity_from_fraction(fraction_at(slider, x)))
    }
}

fn entry_height(entry: &MenuEntry) -> f64 {
    match entry {
        MenuEntry::Action { .. } => MENU_ITEM_HEIGHT,
        MenuEntry::ViewSelector { .. } => VIEW_ROW_HEIGHT,
        MenuEntry::ThemeSelector { .. } => THEME_ROW_HEIGHT,
        MenuEntry::OpacitySlider { .. } => OPACITY_ROW_HEIGHT,
        MenuEntry::Separator => MENU_SEPARATOR_HEIGHT,
    }
}

fn view_icon_centers(bounds: &Rect, row_top: f64) -> [Point; 2] {
    let cy = row_top + VIEW_ROW_HEIGHT * 0.5;
    let step = VIEW_ICON_SIZE + VIEW_ICON_GAP;
    let last = bounds.x1 - MENU_SIDE_PADDING - VIEW_ICON_SIZE * 0.5;
    let first = last - step;
    [Point::new(first, cy), Point::new(last, cy)]
}

fn theme_circle_centers(bounds: &Rect, row_top: f64) -> [Point; 3] {
    let cy = row_top + THEME_ROW_HEIGHT * 0.5;
    let step = THEME_CIRCLE_RADIUS * 2.0 + THEME_CIRCLE_GAP;
    let last = bounds.x1 - MENU_SIDE_PADDING - THEME_CIRCLE_RADIUS;
    let first = last - step * 2.0;
    [
        Point::new(first, cy),
        Point::new(first + step, cy),
        Point::new(first + step * 2.0, cy),
    ]
}

fn slider_track(bounds: &Rect, row_top: f64) -> (f64, f64, f64) {
    let x0 = bounds.x0 + MENU_SIDE_PADDING;
    let x1 = bounds.x1 - MENU_SIDE_PADDING;
    let cy = row_top + OPACITY_ROW_HEIGHT * 0.64;
    (x0, x1, cy)
}

fn fraction_at(slider: &SliderRowGeom, x: f64) -> f64 {
    let width = slider.track_x1 - slider.track_x0;
    if width <= 0.0 {
        return 0.0;
    }
    ((x - slider.track_x0) / width).clamp(0.0, 1.0)
}

fn opacity_from_fraction(fraction: f64) -> f32 {
    let range = f64::from(OPACITY_MAX - OPACITY_MIN);
    (f64::from(OPACITY_MIN) + fraction * range) as f32
}

fn fraction_from_opacity(value: f32) -> f64 {
    let range = f64::from(OPACITY_MAX - OPACITY_MIN);
    if range <= 0.0 {
        return 0.0;
    }
    (f64::from(value - OPACITY_MIN) / range).clamp(0.0, 1.0)
}

pub struct MenuRenderer;

impl MenuRenderer {
    pub fn render(
        &self,
        scene: &mut Scene,
        layout: &MenuLayout,
        entries: &[MenuEntry],
        config: &Config,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        font_size: f32,
    ) {
        let panel = RoundedRect::new(
            layout.bounds.x0,
            layout.bounds.y0,
            layout.bounds.x1,
            layout.bounds.y1,
            RoundedRectRadii::from_single_radius(6.0),
        );
        let theme = UiTheme::from_config(config);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &theme.chrome_brush(),
            None,
            &panel,
        );

        let fg = theme.foreground_brush();
        let muted = theme.muted_brush();
        let accent = theme.accent_brush();
        let family = font_family_from_config(config);
        let label_font = (font_size * 0.85).max(11.0);

        let mut y = layout.bounds.y0 + MENU_PADDING;
        for entry in entries {
            match entry {
                MenuEntry::Action { label, .. } => {
                    let hover_rect = Rect::new(
                        layout.bounds.x0 + 2.0,
                        y,
                        layout.bounds.x1 - 2.0,
                        y + MENU_ITEM_HEIGHT,
                    );
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &theme.elevated_brush(),
                        None,
                        &hover_rect,
                    );
                    draw_text(
                        layout_cx,
                        scene,
                        font_cx,
                        label,
                        layout.bounds.x0 + 12.0,
                        y + 6.0,
                        label_font,
                        family.clone(),
                        &fg,
                        Some((MENU_WIDTH - 24.0) as f32),
                    );
                    y += MENU_ITEM_HEIGHT;
                }
                MenuEntry::ViewSelector { current } => {
                    self.render_view_selector(
                        scene, layout, y, *current, font_cx, layout_cx, &family, label_font, &fg,
                        &muted, &accent,
                    );
                    y += VIEW_ROW_HEIGHT;
                }
                MenuEntry::ThemeSelector { current } => {
                    self.render_theme_selector(
                        scene, layout, y, *current, font_cx, layout_cx, &family, label_font, &fg,
                        &accent,
                    );
                    y += THEME_ROW_HEIGHT;
                }
                MenuEntry::OpacitySlider { value } => {
                    self.render_opacity_slider(
                        scene, layout, y, *value, font_cx, layout_cx, &family, label_font, &fg,
                        &muted, &accent,
                    );
                    y += OPACITY_ROW_HEIGHT;
                }
                MenuEntry::Separator => {
                    let line_y = y + MENU_SEPARATOR_HEIGHT * 0.5;
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &muted,
                        None,
                        &Rect::new(
                            layout.bounds.x0 + 8.0,
                            line_y,
                            layout.bounds.x1 - 8.0,
                            line_y + 1.0,
                        ),
                    );
                    y += MENU_SEPARATOR_HEIGHT;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_view_selector(
        &self,
        scene: &mut Scene,
        layout: &MenuLayout,
        row_top: f64,
        current: ViewMode,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        family: &parley::FontFamily<'static>,
        label_font: f32,
        fg: &Brush,
        muted: &Brush,
        accent: &Brush,
    ) {
        draw_text(
            layout_cx,
            scene,
            font_cx,
            "View",
            layout.bounds.x0 + 12.0,
            row_top + VIEW_ROW_HEIGHT * 0.5 - f64::from(label_font) * 0.6,
            label_font,
            family.clone(),
            fg,
            None,
        );

        let centers = view_icon_centers(&layout.bounds, row_top);
        let modes = [ViewMode::Single, ViewMode::Grid];
        let stroke = Stroke::new(1.2);
        let tile = VIEW_ICON_SIZE * 0.42;
        let gap = VIEW_ICON_SIZE * 0.12;

        for (center, mode) in centers.iter().zip(modes.iter()) {
            let half = VIEW_ICON_SIZE * 0.5;
            let x0 = center.x - half;
            let y0 = center.y - half;
            let x1 = center.x + half;
            let y1 = center.y + half;

            match mode {
                ViewMode::Single => {
                    let panel = RoundedRect::new(
                        x0 + 2.0,
                        y0 + 2.0,
                        x1 - 2.0,
                        y1 - 2.0,
                        RoundedRectRadii::from_single_radius(2.0),
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fg, None, &panel);
                }
                ViewMode::Grid => {
                    for row in 0..2 {
                        for col in 0..2 {
                            let cell_x0 = x0 + gap + col as f64 * (tile + gap);
                            let cell_y0 = y0 + gap + row as f64 * (tile + gap);
                            let cell = RoundedRect::new(
                                cell_x0,
                                cell_y0,
                                cell_x0 + tile,
                                cell_y0 + tile,
                                RoundedRectRadii::from_single_radius(1.5),
                            );
                            scene.fill(Fill::NonZero, Affine::IDENTITY, fg, None, &cell);
                        }
                    }
                }
            }

            let frame = RoundedRect::new(x0, y0, x1, y1, RoundedRectRadii::from_single_radius(3.0));
            scene.stroke(&stroke, Affine::IDENTITY, muted, None, &frame);

            if *mode == current {
                let ring = RoundedRect::new(
                    x0 - 3.0,
                    y0 - 3.0,
                    x1 + 3.0,
                    y1 + 3.0,
                    RoundedRectRadii::from_single_radius(5.0),
                );
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, accent, None, &ring);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_theme_selector(
        &self,
        scene: &mut Scene,
        layout: &MenuLayout,
        row_top: f64,
        current: ThemeMode,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        family: &parley::FontFamily<'static>,
        label_font: f32,
        fg: &Brush,
        accent: &Brush,
    ) {
        draw_text(
            layout_cx,
            scene,
            font_cx,
            "Theme",
            layout.bounds.x0 + 12.0,
            row_top + THEME_ROW_HEIGHT * 0.5 - f64::from(label_font) * 0.6,
            label_font,
            family.clone(),
            fg,
            None,
        );

        let white = Brush::Solid(AlphaColor::<Srgb>::from_rgb8(238, 238, 238));
        let black = Brush::Solid(AlphaColor::<Srgb>::from_rgb8(24, 24, 24));
        // Outline with the foreground color so the swatch stays visible even when
        // its fill matches the menu background (e.g. the black dark-theme swatch on
        // dark chrome).
        let border = Stroke::new(1.3);

        let centers = theme_circle_centers(&layout.bounds, row_top);
        let modes = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Auto];
        for (center, mode) in centers.iter().zip(modes.iter()) {
            let disc = Circle::new(*center, THEME_CIRCLE_RADIUS);
            match mode {
                ThemeMode::Light => {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &white, None, &disc);
                }
                ThemeMode::Dark => {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &black, None, &disc);
                }
                ThemeMode::Auto => {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &white, None, &disc);
                    let half = CircleSegment::new(
                        *center,
                        THEME_CIRCLE_RADIUS,
                        0.0,
                        -std::f64::consts::FRAC_PI_2,
                        std::f64::consts::PI,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &black, None, &half);
                }
            }

            scene.stroke(&border, Affine::IDENTITY, fg, None, &disc);

            if *mode == current {
                let ring = Circle::new(*center, THEME_CIRCLE_RADIUS + 3.5);
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, accent, None, &ring);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_opacity_slider(
        &self,
        scene: &mut Scene,
        layout: &MenuLayout,
        row_top: f64,
        value: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        family: &parley::FontFamily<'static>,
        label_font: f32,
        fg: &Brush,
        muted: &Brush,
        accent: &Brush,
    ) {
        let label = format!("Opacity  {:.0}%", value * 100.0);
        draw_text(
            layout_cx,
            scene,
            font_cx,
            &label,
            layout.bounds.x0 + 12.0,
            row_top + 7.0,
            label_font,
            family.clone(),
            fg,
            Some((MENU_WIDTH - 24.0) as f32),
        );

        let (track_x0, track_x1, cy) = slider_track(&layout.bounds, row_top);
        let half_h = SLIDER_TRACK_HEIGHT * 0.5;
        let track = RoundedRect::new(
            track_x0,
            cy - half_h,
            track_x1,
            cy + half_h,
            RoundedRectRadii::from_single_radius(half_h),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, muted, None, &track);

        let fraction = fraction_from_opacity(value);
        let knob_x = track_x0 + fraction * (track_x1 - track_x0);

        if knob_x > track_x0 {
            let filled = RoundedRect::new(
                track_x0,
                cy - half_h,
                knob_x,
                cy + half_h,
                RoundedRectRadii::from_single_radius(half_h),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &filled);
        }

        let knob = Circle::new(Point::new(knob_x, cy), SLIDER_KNOB_RADIUS);
        scene.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &knob);
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, fg, None, &knob);
    }
}

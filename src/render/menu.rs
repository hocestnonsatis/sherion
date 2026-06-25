use std::ops::Range;

use vello::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii};
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;

const MENU_WIDTH: f64 = 220.0;
const MENU_ITEM_HEIGHT: f64 = 32.0;
const MENU_SEPARATOR_HEIGHT: f64 = 9.0;
const MENU_PADDING: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewTab,
    CloseTab,
    Copy,
    Paste,
    ClearScrollback,
    ThemeLight,
    ThemeDark,
    ThemeAuto,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    OpacityIncrease,
    OpacityDecrease,
    Quit,
}

#[derive(Clone, Debug)]
pub enum MenuEntry {
    Action {
        action: MenuAction,
        label: String,
    },
    Separator,
}

pub struct MenuLayout {
    pub bounds: Rect,
    item_regions: Vec<(MenuAction, Range<f64>)>,
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
        let mut y = anchor_y + MENU_PADDING;
        for entry in entries {
            match entry {
                MenuEntry::Action { action, .. } => {
                    item_regions.push((*action, y..y + MENU_ITEM_HEIGHT));
                    y += MENU_ITEM_HEIGHT;
                }
                MenuEntry::Separator => {
                    y += MENU_SEPARATOR_HEIGHT;
                }
            }
        }

        Self {
            bounds,
            item_regions,
        }
    }

    pub fn hit_test(&self, x: f64, y: f64) -> Option<MenuAction> {
        if !self.bounds.contains((x, y)) {
            return None;
        }
        for (action, range) in &self.item_regions {
            if range.contains(&y) {
                return Some(*action);
            }
        }
        None
    }
}

fn entry_height(entry: &MenuEntry) -> f64 {
    match entry {
        MenuEntry::Action { .. } => MENU_ITEM_HEIGHT,
        MenuEntry::Separator => MENU_SEPARATOR_HEIGHT,
    }
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
}

use std::ops::Range;

use vello::kurbo::{Affine, Line, Rect, RoundedRect, RoundedRectRadii, Stroke};
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;

const WINDOW_BUTTON_WIDTH: f64 = 46.0;

#[derive(Clone, Copy, Debug)]
pub struct TitleBarMetrics {
    pub height: f32,
}

impl TitleBarMetrics {
    pub fn from_font_size(font_size: f32, scale: f32) -> Self {
        let height = (font_size * 1.35 * scale).round().max(32.0);
        Self { height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleBarHit {
    Drag,
    Hamburger,
    Minimize,
    Maximize,
    Close,
    None,
}

pub struct TitleBarLayout {
    pub metrics: TitleBarMetrics,
    pub x_offset: f64,
    pub drag_region: Range<f64>,
    pub hamburger_region: Range<f64>,
    pub minimize_region: Range<f64>,
    pub maximize_region: Range<f64>,
    pub close_region: Range<f64>,
    pub width: f64,
}

impl TitleBarLayout {
    pub fn compute(metrics: TitleBarMetrics, x_offset: f64, width: f64) -> Self {
        let close_start = (width - WINDOW_BUTTON_WIDTH).max(0.0);
        let maximize_start = (close_start - WINDOW_BUTTON_WIDTH).max(0.0);
        let minimize_start = (maximize_start - WINDOW_BUTTON_WIDTH).max(0.0);
        let hamburger_start = (minimize_start - WINDOW_BUTTON_WIDTH).max(0.0);

        Self {
            metrics,
            x_offset,
            drag_region: 0.0..hamburger_start,
            hamburger_region: hamburger_start..minimize_start,
            minimize_region: minimize_start..maximize_start,
            maximize_region: maximize_start..close_start,
            close_region: close_start..width,
            width,
        }
    }

    pub fn hit_test(&self, x: f64, y: f64) -> TitleBarHit {
        let local_x = x - self.x_offset;
        if y < 0.0 || y >= f64::from(self.metrics.height) {
            return TitleBarHit::None;
        }
        if local_x < 0.0 || local_x >= self.width {
            return TitleBarHit::None;
        }
        if self.close_region.contains(&local_x) {
            return TitleBarHit::Close;
        }
        if self.maximize_region.contains(&local_x) {
            return TitleBarHit::Maximize;
        }
        if self.minimize_region.contains(&local_x) {
            return TitleBarHit::Minimize;
        }
        if self.hamburger_region.contains(&local_x) {
            return TitleBarHit::Hamburger;
        }
        if self.drag_region.contains(&local_x) {
            return TitleBarHit::Drag;
        }
        TitleBarHit::None
    }

    pub fn hamburger_anchor(&self) -> (f64, f64) {
        (
            self.x_offset + self.hamburger_region.start,
            f64::from(self.metrics.height),
        )
    }
}

pub struct TitleBarRenderer;

impl TitleBarRenderer {
    pub fn render(
        &self,
        scene: &mut Scene,
        layout: &TitleBarLayout,
        title: &str,
        config: &Config,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
        font_size: f32,
        menu_open: bool,
    ) {
        let height = f64::from(layout.metrics.height);
        let transform = Affine::translate((layout.x_offset, 0.0));
        let theme = UiTheme::from_config(config);
        let bg = theme.chrome_brush();
        let fg = theme.foreground_brush();
        let hover_bg = theme.elevated_brush();
        let family = font_family_from_config(config);
        let label_font = (font_size * 0.85).max(11.0);
        let label_y = ((height - f64::from(label_font)) * 0.5).max(4.0);

        scene.fill(
            Fill::NonZero,
            transform,
            &bg,
            None,
            &Rect::new(0.0, 0.0, layout.width, height),
        );

        let title_max_width =
            (layout.drag_region.end - layout.drag_region.start - 16.0).max(0.0) as f32;
        if title_max_width > 0.0 {
            draw_text(
                layout_cx,
                scene,
                font_cx,
                title,
                layout.x_offset + layout.drag_region.start + 12.0,
                label_y,
                label_font,
                family.clone(),
                &fg,
                Some(title_max_width),
            );
        }

        self.draw_window_button(
            scene,
            layout.x_offset + layout.hamburger_region.start,
            layout.x_offset + layout.hamburger_region.end,
            height,
            if menu_open { &hover_bg } else { &bg },
            ButtonKind::Hamburger,
            &fg,
        );

        self.draw_window_button(
            scene,
            layout.x_offset + layout.minimize_region.start,
            layout.x_offset + layout.minimize_region.end,
            height,
            &hover_bg,
            ButtonKind::Minimize,
            &fg,
        );

        self.draw_window_button(
            scene,
            layout.x_offset + layout.maximize_region.start,
            layout.x_offset + layout.maximize_region.end,
            height,
            &hover_bg,
            ButtonKind::Maximize,
            &fg,
        );

        self.draw_window_button(
            scene,
            layout.x_offset + layout.close_region.start,
            layout.x_offset + layout.close_region.end,
            height,
            &hover_bg,
            ButtonKind::Close,
            &fg,
        );
    }

    fn draw_window_button(
        &self,
        scene: &mut Scene,
        x0: f64,
        x1: f64,
        height: f64,
        bg: &Brush,
        kind: ButtonKind,
        icon: &Brush,
    ) {
        let rect = Rect::new(x0, 0.0, x1, height);
        scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &rect);

        let cx = (x0 + x1) * 0.5;
        let cy = height * 0.5;
        match kind {
            ButtonKind::Hamburger => self.draw_hamburger(scene, cx, cy, icon),
            ButtonKind::Minimize => self.draw_minimize(scene, cx, cy, icon),
            ButtonKind::Maximize => self.draw_maximize(scene, cx, cy, icon),
            ButtonKind::Close => self.draw_close(scene, cx, cy, icon),
        }
    }

    fn draw_hamburger(&self, scene: &mut Scene, cx: f64, cy: f64, brush: &Brush) {
        let line_w = 14.0;
        let line_h = 1.8;
        let gap = 4.5;
        for offset in [-gap, 0.0, gap] {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                brush,
                None,
                &RoundedRect::new(
                    cx - line_w * 0.5,
                    cy + offset - line_h * 0.5,
                    cx + line_w * 0.5,
                    cy + offset + line_h * 0.5,
                    RoundedRectRadii::from_single_radius(1.0),
                ),
            );
        }
    }

    fn draw_minimize(&self, scene: &mut Scene, cx: f64, cy: f64, brush: &Brush) {
        let line_w = 10.0;
        let line_h = 1.8;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            brush,
            None,
            &RoundedRect::new(
                cx - line_w * 0.5,
                cy + 4.0 - line_h * 0.5,
                cx + line_w * 0.5,
                cy + 4.0 + line_h * 0.5,
                RoundedRectRadii::from_single_radius(1.0),
            ),
        );
    }

    fn draw_maximize(&self, scene: &mut Scene, cx: f64, cy: f64, brush: &Brush) {
        let size = 9.0;
        let stroke = Stroke::new(1.6);
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            brush,
            None,
            &Rect::new(
                cx - size * 0.5,
                cy - size * 0.5,
                cx + size * 0.5,
                cy + size * 0.5,
            ),
        );
    }

    fn draw_close(&self, scene: &mut Scene, cx: f64, cy: f64, brush: &Brush) {
        let size = 5.5;
        let stroke = Stroke::new(1.8);
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            brush,
            None,
            &Line::new((cx - size, cy - size), (cx + size, cy + size)),
        );
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            brush,
            None,
            &Line::new((cx + size, cy - size), (cx - size, cy + size)),
        );
    }
}

enum ButtonKind {
    Hamburger,
    Minimize,
    Maximize,
    Close,
}

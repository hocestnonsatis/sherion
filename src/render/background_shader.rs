use vello::kurbo::{Affine, Rect};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill, Mix};
use vello::Scene;

use crate::config::{BackgroundShader, Config};

pub fn draw_background_shader(scene: &mut Scene, bounds: Rect, config: &Config) {
    let shader = config.appearance.background_shader;
    if shader == BackgroundShader::None {
        return;
    }

    let intensity = config.appearance.background_shader_intensity.clamp(0.0, 1.0);
    if intensity <= f32::EPSILON {
        return;
    }

    match shader {
        BackgroundShader::None => {}
        BackgroundShader::Vignette => draw_vignette(scene, bounds, intensity),
        BackgroundShader::Scanlines => draw_scanlines(scene, bounds, intensity),
        BackgroundShader::Noise => draw_noise(scene, bounds, intensity),
    }
}

fn draw_vignette(scene: &mut Scene, bounds: Rect, intensity: f32) {
    let width = bounds.width();
    let height = bounds.height();
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let edge = (width.min(height) * 0.22).max(24.0);
    let alpha = (180.0 * intensity) as u8;
    let brush = Brush::Solid(AlphaColor::from_rgba8(0, 0, 0, alpha));

    scene.push_layer(
        Fill::NonZero,
        Mix::Normal,
        1.0,
        Affine::IDENTITY,
        &bounds,
    );

    let top = Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + edge);
    let bottom = Rect::new(bounds.x0, bounds.y1 - edge, bounds.x1, bounds.y1);
    let left = Rect::new(bounds.x0, bounds.y0, bounds.x0 + edge, bounds.y1);
    let right = Rect::new(bounds.x1 - edge, bounds.y0, bounds.x1, bounds.y1);

    for rect in [top, bottom, left, right] {
        scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &rect);
    }

    scene.pop_layer();
}

fn draw_scanlines(scene: &mut Scene, bounds: Rect, intensity: f32) {
    let line_h = 2.0;
    let gap = 4.0;
    let alpha = (120.0 * intensity) as u8;
    let brush = Brush::Solid(AlphaColor::from_rgba8(0, 0, 0, alpha));

    scene.push_layer(
        Fill::NonZero,
        Mix::Normal,
        1.0,
        Affine::IDENTITY,
        &bounds,
    );

    let mut y = bounds.y0;
    while y < bounds.y1 {
        let rect = Rect::new(bounds.x0, y, bounds.x1, (y + line_h).min(bounds.y1));
        scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &rect);
        y += line_h + gap;
    }

    scene.pop_layer();
}

fn draw_noise(scene: &mut Scene, bounds: Rect, intensity: f32) {
    let alpha = (90.0 * intensity) as u8;
    let brush = Brush::Solid(AlphaColor::from_rgba8(255, 255, 255, alpha));
    let step = 6.0;

    scene.push_layer(
        Fill::NonZero,
        Mix::Normal,
        0.35 * intensity,
        Affine::IDENTITY,
        &bounds,
    );

    let mut y = bounds.y0;
    let mut row = 0usize;
    while y < bounds.y1 {
        let mut x = bounds.x0 + if row % 2 == 0 { 0.0 } else { step * 0.5 };
        let mut col = 0usize;
        while x < bounds.x1 {
            if (row + col) % 3 == 0 {
                let size = step * 0.45;
                let rect = Rect::new(x, y, (x + size).min(bounds.x1), (y + size).min(bounds.y1));
                scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &rect);
            }
            x += step;
            col += 1;
        }
        y += step;
        row += 1;
    }

    scene.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_shader_is_noop() {
        let mut scene = Scene::new();
        let config = Config::default();
        draw_background_shader(&mut scene, Rect::new(0.0, 0.0, 100.0, 100.0), &config);
    }
}

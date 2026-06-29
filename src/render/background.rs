use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Blob, Extend, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
use vello::Scene;

use crate::config::{BackgroundMode, Config};

pub struct BackgroundImage {
    brush: ImageBrush,
    width: u32,
    height: u32,
    source: Option<PathBuf>,
    mode: BackgroundMode,
    opacity: f32,
}

impl BackgroundImage {
    pub fn from_config(config: &Config) -> Result<Option<Self>> {
        let Some(path) = config.appearance.background_image.as_ref() else {
            return Ok(None);
        };
        Self::load(
            path,
            config.appearance.background_mode,
            config.appearance.background_opacity,
        )
        .map(Some)
    }

    pub fn load(path: &Path, mode: BackgroundMode, opacity: f32) -> Result<Self> {
        let image = image::open(path)
            .with_context(|| format!("failed to open background image {}", path.display()))?;
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let image_data = ImageData {
            data: Blob::new(Arc::new(rgba.into_raw())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width,
            height,
        };
        let brush = ImageBrush::new(image_data)
            .with_extend(Extend::Pad)
            .with_quality(ImageQuality::Medium);
        Ok(Self {
            brush,
            width,
            height,
            source: Some(path.to_path_buf()),
            mode,
            opacity,
        })
    }

    pub fn matches_config(&self, config: &Config) -> bool {
        self.source.as_deref() == config.appearance.background_image.as_deref()
            && self.mode == config.appearance.background_mode
            && (self.opacity - config.appearance.background_opacity).abs() < f32::EPSILON
    }

    pub fn draw(&self, scene: &mut Scene, bounds: Rect) {
        if self.width == 0 || self.height == 0 {
            return;
        }

        let transform = match self.mode {
            BackgroundMode::Tile => {
                scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::Mix::Normal,
                    self.opacity,
                    Affine::IDENTITY,
                    &bounds,
                );
                let tile_w = self.width as f64;
                let tile_h = self.height as f64;
                let mut y = bounds.y0;
                while y < bounds.y1 {
                    let mut x = bounds.x0;
                    while x < bounds.x1 {
                        scene.draw_image(
                            self.brush.as_ref(),
                            Affine::translate((x, y)),
                        );
                        x += tile_w;
                    }
                    y += tile_h;
                }
                scene.pop_layer();
                return;
            }
            BackgroundMode::Center => {
                let x = bounds.x0 + (bounds.width() - self.width as f64) * 0.5;
                let y = bounds.y0 + (bounds.height() - self.height as f64) * 0.5;
                Affine::translate((x, y))
            }
            BackgroundMode::Contain | BackgroundMode::Cover => {
                let scale_x = bounds.width() / self.width as f64;
                let scale_y = bounds.height() / self.height as f64;
                let scale = if self.mode == BackgroundMode::Cover {
                    scale_x.max(scale_y)
                } else {
                    scale_x.min(scale_y)
                };
                let draw_w = self.width as f64 * scale;
                let draw_h = self.height as f64 * scale;
                let x = bounds.x0 + (bounds.width() - draw_w) * 0.5;
                let y = bounds.y0 + (bounds.height() - draw_h) * 0.5;
                Affine::scale_non_uniform(scale, scale)
                    * Affine::translate((x / scale, y / scale))
            }
        };

        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::Mix::Normal,
            self.opacity,
            Affine::IDENTITY,
            &bounds,
        );
        scene.draw_image(self.brush.as_ref(), transform);
        scene.pop_layer();
    }
}

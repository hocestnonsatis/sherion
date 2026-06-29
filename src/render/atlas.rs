use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use fontdb::{Database, Query};
use swash::scale::image::{Content, Image};
use swash::scale::{Render, ScaleContext, Source};
use swash::FontRef;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{
    Blob, Brush, Extend, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality, Mix,
};
use vello::peniko::Fill;
use vello::Scene;

use crate::config::Config;
use crate::render::TerminalLayout;

#[derive(Clone, Eq)]
struct AtlasKey {
    text: String,
    font_size_bits: u32,
    bold: bool,
    italic: bool,
    family: String,
    fg_rgba: u32,
}

impl PartialEq for AtlasKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.font_size_bits == other.font_size_bits
            && self.bold == other.bold
            && self.italic == other.italic
            && self.family == other.family
            && self.fg_rgba == other.fg_rgba
    }
}

impl Hash for AtlasKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.font_size_bits.hash(state);
        self.bold.hash(state);
        self.italic.hash(state);
        self.family.hash(state);
        self.fg_rgba.hash(state);
    }
}

#[derive(Clone)]
pub struct AtlasGlyph {
    pub image: ImageData,
    pub bearing_x: f32,
    pub bearing_y: f32,
}

pub struct GlyphAtlas {
    entries: HashMap<AtlasKey, AtlasGlyph>,
    scale_cx: ScaleContext,
    db: Database,
}

impl GlyphAtlas {
    pub fn new(config: &Config) -> Option<Self> {
        let mut db = Database::new();
        db.load_system_fonts();
        atlas_font_families(config).first()?;
        Some(Self {
            entries: HashMap::new(),
            scale_cx: ScaleContext::new(),
            db,
        })
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get_or_rasterize_text(
        &mut self,
        text: &str,
        font_size: f32,
        bold: bool,
        italic: bool,
        fg: &Brush,
        config: &Config,
    ) -> Option<&AtlasGlyph> {
        let fg_rgba = brush_to_rgba(fg);
        for family in atlas_font_families(config) {
            let key = AtlasKey {
                text: text.to_owned(),
                font_size_bits: font_size.to_bits(),
                bold,
                italic,
                family: family.clone(),
                fg_rgba,
            };
            if self.entries.contains_key(&key) {
                return self.entries.get(&key);
            }
            if let Some(glyph) =
                self.rasterize_text(text, font_size, bold, italic, fg_rgba, &family)
            {
                self.entries.insert(key.clone(), glyph);
                return self.entries.get(&key);
            }
        }
        None
    }

    fn rasterize_text(
        &mut self,
        text: &str,
        font_size: f32,
        bold: bool,
        italic: bool,
        fg_rgba: u32,
        family: &str,
    ) -> Option<AtlasGlyph> {
        let base = text.chars().next()?;
        let font_id = self.font_id_for_family(family)?;
        let mut result = None;
        self.db.with_face_data(font_id, |data, index| {
            result = Self::rasterize_font_data(
                &mut self.scale_cx,
                data,
                index as usize,
                base,
                font_size,
                bold,
                italic,
                fg_rgba,
            );
        });
        result
    }

    fn font_id_for_family(&self, family: &str) -> Option<fontdb::ID> {
        let query = Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(400),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        self.db.query(&query)
    }

    fn rasterize_font_data(
        scale_cx: &mut ScaleContext,
        data: &[u8],
        index: usize,
        ch: char,
        font_size: f32,
        bold: bool,
        italic: bool,
        fg_rgba: u32,
    ) -> Option<AtlasGlyph> {
        let font = FontRef::from_index(data, index)?;
        let mut scaler = scale_cx.builder(font).size(font_size).hint(true).build();
        let glyph_id = font.charmap().map(ch);
        let mut image = Image::new();
        let mut render = Render::new(&[Source::Outline]);
        if bold {
            render.embolden(0.02);
        }
        let _ = italic;
        if !render.render_into(&mut scaler, glyph_id, &mut image) {
            return None;
        }

        let placement = image.placement;
        let width = placement.width;
        let height = placement.height;
        if width == 0 || height == 0 {
            return None;
        }

        let (fr, fg, fb) = (
            ((fg_rgba >> 24) & 0xff) as u8,
            ((fg_rgba >> 16) & 0xff) as u8,
            ((fg_rgba >> 8) & 0xff) as u8,
        );

        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        match image.content {
            Content::Mask | Content::SubpixelMask => {
                for (i, alpha) in image.data.iter().enumerate() {
                    let offset = i * 4;
                    rgba[offset] = fr;
                    rgba[offset + 1] = fg;
                    rgba[offset + 2] = fb;
                    rgba[offset + 3] = *alpha;
                }
            }
            Content::Color => {
                for (i, chunk) in image.data.chunks(4).enumerate() {
                    let offset = i * 4;
                    let alpha = chunk.get(3).copied().unwrap_or(255);
                    rgba[offset] = fr;
                    rgba[offset + 1] = fg;
                    rgba[offset + 2] = fb;
                    rgba[offset + 3] = alpha;
                }
            }
        }

        let image_data = ImageData {
            data: Blob::new(Arc::new(rgba)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: width as u32,
            height: height as u32,
        };

        Some(AtlasGlyph {
            image: image_data,
            bearing_x: placement.left as f32,
            bearing_y: (placement.top as f32) - height as f32,
        })
    }
}

fn brush_to_rgba(brush: &Brush) -> u32 {
    let Brush::Solid(color) = brush else {
        return 0xff_cc_cc_cc;
    };
    let rgba = color.to_rgba8();
    u32::from(rgba.r) << 24
        | u32::from(rgba.g) << 16
        | u32::from(rgba.b) << 8
        | u32::from(rgba.a)
}

pub fn draw_atlas_glyph(
    scene: &mut Scene,
    glyph: &AtlasGlyph,
    x: f64,
    y: f64,
    cell_height: f32,
) {
    let baseline_y = y + f64::from(cell_height) * 0.8;
    let transform = Affine::translate((
        x + f64::from(glyph.bearing_x),
        baseline_y + f64::from(glyph.bearing_y),
    ));
    let image_brush = ImageBrush::new(glyph.image.clone())
        .with_extend(Extend::Pad)
        .with_quality(ImageQuality::Low);
    scene.draw_image(image_brush.as_ref(), transform);
}

pub fn row_band_clip_rect(
    row: usize,
    layout: TerminalLayout,
    x_off: f64,
    y_off: f64,
) -> Rect {
    let y = y_off + row as f64 * f64::from(layout.cell_height);
    let width = f64::from(layout.cell_width) * f64::from(layout.cols);
    let height = f64::from(layout.cell_height);
    Rect::new(x_off, y, x_off + width, y + height)
}

pub fn push_row_clip(scene: &mut Scene, rect: &Rect) {
    scene.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, rect);
}

pub fn pop_row_clip(scene: &mut Scene) {
    scene.pop_layer();
}

fn atlas_font_families(config: &Config) -> Vec<String> {
    let mut families = vec![config.font.family.clone()];
    families.extend(config.font.fallback.clone());
    families
}

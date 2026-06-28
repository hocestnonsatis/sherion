use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use parley::layout::{GlyphRun, PositionedLayoutItem};
use parley::FontData;
use vello::kurbo::Affine;
use vello::peniko::{Brush, Fill};
use vello::{Glyph, Scene};

#[derive(Clone, Eq)]
pub struct GlyphCacheKey {
    text: TextKey,
    bold: bool,
    italic: bool,
    underline: bool,
    font_size_bits: u32,
}

#[derive(Clone, Eq)]
enum TextKey {
    Single(char),
    Multi(String),
}

impl PartialEq for GlyphCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.font_size_bits == other.font_size_bits
    }
}

impl PartialEq for TextKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Single(a), Self::Single(b)) => a == b,
            (Self::Multi(a), Self::Multi(b)) => a == b,
            (Self::Single(a), Self::Multi(b)) => text_is_single_char(b, *a),
            (Self::Multi(a), Self::Single(b)) => text_is_single_char(a, *b),
        }
    }
}

fn text_is_single_char(text: &str, ch: char) -> bool {
    let mut chars = text.chars();
    chars.next() == Some(ch) && chars.next().is_none()
}

impl Hash for GlyphCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.bold.hash(state);
        self.italic.hash(state);
        self.underline.hash(state);
        self.font_size_bits.hash(state);
    }
}

impl Hash for TextKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Single(ch) => {
                0u8.hash(state);
                ch.hash(state);
            }
            Self::Multi(text) => {
                1u8.hash(state);
                text.hash(state);
            }
        }
    }
}

impl GlyphCacheKey {
    pub fn new(text: &str, bold: bool, italic: bool, underline: bool, font_size: f32) -> Self {
        let text = if text.len() == 1 {
            TextKey::Single(text.chars().next().unwrap_or(' '))
        } else {
            TextKey::Multi(text.to_owned())
        };
        Self {
            text,
            bold,
            italic,
            underline,
            font_size_bits: font_size.to_bits(),
        }
    }
}

#[derive(Clone)]
pub struct CachedGlyphRun {
    pub font: FontData,
    pub font_size: f32,
    pub normalized_coords: Vec<i16>,
    pub glyph_xform: Option<Affine>,
    pub glyphs: Vec<Glyph>,
}

pub struct GlyphCache {
    entries: HashMap<GlyphCacheKey, CachedGlyphRun>,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get(&self, key: &GlyphCacheKey) -> Option<&CachedGlyphRun> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: GlyphCacheKey, run: CachedGlyphRun) {
        self.entries.insert(key, run);
    }
}

pub fn emit_cached_glyphs(
    scene: &mut Scene,
    cached: &CachedGlyphRun,
    brush: &Brush,
    x: f64,
    y: f64,
) {
    if cached.glyphs.is_empty() {
        return;
    }

    scene
        .draw_glyphs(&cached.font)
        .brush(brush)
        .hint(true)
        .transform(Affine::translate((x, y)))
        .glyph_transform(cached.glyph_xform)
        .font_size(cached.font_size)
        .normalized_coords(&cached.normalized_coords)
        .draw(Fill::NonZero, cached.glyphs.iter().copied());
}

pub fn cache_glyph_run(glyph_run: GlyphRun<'_, Brush>, transform: Affine) -> CachedGlyphRun {
    let run = glyph_run.run();
    let font = run.font().clone();
    let font_size = run.font_size();
    let synthesis = run.synthesis();
    let glyph_xform = synthesis
        .skew()
        .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
    let normalized_coords = run.normalized_coords().to_vec();

    let mut glyph_x = glyph_run.offset();
    let glyph_y = glyph_run.baseline();
    let glyphs: Vec<Glyph> = glyph_run
        .glyphs()
        .map(|glyph| {
            let gx = glyph_x + glyph.x;
            let gy = glyph_y + glyph.y;
            glyph_x += glyph.advance;
            Glyph {
                id: glyph.id,
                x: gx,
                y: gy,
            }
        })
        .collect();

    let _ = transform;
    CachedGlyphRun {
        font,
        font_size,
        normalized_coords,
        glyph_xform,
        glyphs,
    }
}

pub fn emit_glyph_run(scene: &mut Scene, glyph_run: GlyphRun<'_, Brush>, transform: Affine) {
    let brush = glyph_run.style().brush.clone();
    let mut glyph_x = glyph_run.offset();
    let glyph_y = glyph_run.baseline();
    let run = glyph_run.run();
    let font = run.font();
    let font_size = run.font_size();
    let synthesis = run.synthesis();
    let glyph_xform = synthesis
        .skew()
        .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

    scene
        .draw_glyphs(font)
        .brush(&brush)
        .hint(true)
        .transform(transform)
        .glyph_transform(glyph_xform)
        .font_size(font_size)
        .normalized_coords(run.normalized_coords())
        .draw(
            Fill::NonZero,
            glyph_run.glyphs().map(|glyph| {
                let gx = glyph_x + glyph.x;
                let gy = glyph_y + glyph.y;
                glyph_x += glyph.advance;
                Glyph {
                    id: glyph.id,
                    x: gx,
                    y: gy,
                }
            }),
        );
}

pub fn cache_layout_glyphs(layout: &parley::Layout<Brush>) -> Option<CachedGlyphRun> {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            return Some(cache_glyph_run(glyph_run, Affine::IDENTITY));
        }
    }
    None
}

use std::borrow::Cow;

use parley::layout::PositionedLayoutItem;
use parley::{
    Alignment, AlignmentOptions, FontFamily, FontFamilyName, LayoutContext, LineHeight,
    StyleProperty,
};
use vello::kurbo::Affine;
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;

pub fn font_family_from_config(config: &Config) -> FontFamily<'static> {
    let mut names = Vec::new();

    // Chrome text is UI, not terminal grid text. Prefer a proportional system
    // face for sharper small labels, then fall back to the configured terminal
    // font so symbols/icons still resolve consistently.
    names.push(FontFamilyName::Generic(parley::GenericFamily::SansSerif));

    if let Some(parsed) = FontFamilyName::parse(&config.font.family) {
        names.push(parsed.into_owned());
    }
    for fallback in &config.font.fallback {
        if let Some(parsed) = FontFamilyName::parse(fallback) {
            names.push(parsed.into_owned());
        }
    }
    names.push(FontFamilyName::Generic(parley::GenericFamily::Monospace));

    FontFamily::List(Cow::Owned(names))
}

pub fn draw_text(
    layout_cx: &mut LayoutContext<Brush>,
    scene: &mut Scene,
    font_cx: &mut parley::FontContext,
    text: &str,
    x: f64,
    y_top: f64,
    font_size: f32,
    font_family: FontFamily<'_>,
    brush: &Brush,
    max_width: Option<f32>,
) {
    if text.is_empty() {
        return;
    }

    let x = x.round();
    let y_top = y_top.round();
    let font_size = font_size.round().max(1.0);
    let max_width = max_width.map(|width| width.round().max(1.0));

    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontFamily(font_family));
    builder.push_default(StyleProperty::Brush(brush.clone()));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.0)));

    let mut text_layout = builder.build(text);
    text_layout.break_all_lines(max_width);
    text_layout.align(Alignment::Start, AlignmentOptions::default());

    let transform = Affine::translate((x, y_top));

    for line in text_layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };

            let style = glyph_run.style();
            let mut glyph_x = glyph_run.offset();
            let glyph_y = glyph_run.baseline();
            let run = glyph_run.run();
            let font = run.font();
            let run_font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

            scene
                .draw_glyphs(font)
                .brush(&style.brush)
                .hint(true)
                .transform(transform)
                .glyph_transform(glyph_xform)
                .font_size(run_font_size)
                .normalized_coords(run.normalized_coords())
                .draw(
                    Fill::NonZero,
                    glyph_run.glyphs().map(|glyph| {
                        let gx = glyph_x + glyph.x;
                        let gy = glyph_y + glyph.y;
                        glyph_x += glyph.advance;
                        vello::Glyph {
                            id: glyph.id,
                            x: gx,
                            y: gy,
                        }
                    }),
                );
        }
    }
}

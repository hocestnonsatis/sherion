use alacritty_terminal::index::Point;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{point_to_viewport, RenderableContent, RenderableCursor};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use parley::layout::PositionedLayoutItem;
use parley::{
    FontFamily, FontFamilyName, FontWeight, LayoutContext, LineHeight, StyleProperty,
};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::TerminalLayout;

pub struct SceneBuilder {
    pub layout_cx: LayoutContext<Brush>,
}

#[derive(Clone)]
struct GlyphStyle {
    fg: Brush,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl SceneBuilder {
    pub fn new() -> Self {
        Self {
            layout_cx: LayoutContext::new(),
        }
    }

    pub fn build(
        &mut self,
        scene: &mut Scene,
        content: RenderableContent<'_>,
        layout: TerminalLayout,
        config: &Config,
        font_cx: &mut parley::FontContext,
        terminal_opacity: f32,
    ) {
        let default_fg: Brush = config.foreground_brush().into();
        let default_bg: Brush = config.background_brush().into();
        let cursor_brush: Brush = config.cursor_brush().into();
        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);

        let width = f64::from(layout.cell_width) * f64::from(layout.cols);
        let height = f64::from(layout.cell_height) * f64::from(layout.rows);

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &with_opacity(default_bg.clone(), terminal_opacity),
            None,
            &Rect::new(x_off, y_off, x_off + width, y_off + height),
        );

        let row_count = layout.rows as usize;
        let col_count = layout.cols as usize;
        let display_offset = content.display_offset;
        let selection = content.selection;
        let mut rows: Vec<Vec<(Point, Cell)>> = vec![Vec::new(); row_count];
        let mut cursor_cell: Option<Cell> = None;

        for indexed in content.display_iter {
            if indexed.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }

            let point = indexed.point;
            let Some(viewport) = point_to_viewport(display_offset, point) else {
                continue;
            };
            let row = viewport.line;
            if row >= row_count {
                continue;
            }

            let col = point.column.0;
            if col >= col_count {
                continue;
            }

            let cell = indexed.cell.clone();

            if point == content.cursor.point {
                cursor_cell = Some(cell.clone());
            }

            let (mut fg, mut bg) =
                resolved_colors(&cell, content.colors, &default_fg, &default_bg);
            apply_cell_colors(&mut fg, &mut bg, &cell, point, selection);

            // Window transparency follows the kitty/alacritty model: the opacity
            // only thins the default background, which is already painted once by
            // the full-grid fill above. Cells that keep the default background add
            // no per-cell fill (so two translucent layers can't stack and wash out
            // the effect), while explicitly colored cells stay fully opaque so
            // color blocks and selections remain solid.
            if !brush_solid_eq(&bg, &default_bg) {
                let col = point.column.0;
                // Snap background rectangles to integer pixel boundaries so adjacent
                // cells share an identical edge. Without this the fractional cell
                // width leaves anti-aliased seams between solid color blocks
                // (e.g. fastfetch's palette row).
                let x0 = (x_off + col as f64 * f64::from(layout.cell_width)).round();
                let x1 = (x_off + (col + 1) as f64 * f64::from(layout.cell_width)).round();
                let y0 = (y_off + row as f64 * f64::from(layout.cell_height)).round();
                let y1 = (y_off + (row + 1) as f64 * f64::from(layout.cell_height)).round();

                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &bg,
                    None,
                    &Rect::new(x0, y0, x1, y1),
                );
            }

            rows[row].push((point, cell));
        }

        for (row, mut cells) in rows.into_iter().enumerate() {
            if cells.is_empty() {
                continue;
            }
            cells.sort_by_key(|(point, _)| point.column.0);
            self.render_row(
                scene,
                font_cx,
                row,
                &cells,
                layout,
                config,
                &default_fg,
                &default_bg,
                content.colors,
                selection,
            );
        }

        self.render_cursor(
            scene,
            content.cursor,
            display_offset,
            layout,
            &cursor_brush,
        );

        if content.cursor.shape == CursorShape::Block {
            if let Some(cell) = cursor_cell {
                self.render_cursor_glyph(
                    scene,
                    font_cx,
                    content.cursor,
                    display_offset,
                    &cell,
                    layout,
                    config,
                    &default_fg,
                    &default_bg,
                    content.colors,
                    selection,
                );
            }
        }
    }

    fn render_row(
        &mut self,
        scene: &mut Scene,
        font_cx: &mut parley::FontContext,
        row: usize,
        cells: &[(Point, Cell)],
        layout: TerminalLayout,
        config: &Config,
        default_fg: &Brush,
        default_bg: &Brush,
        colors: &Colors,
        selection: Option<SelectionRange>,
    ) {
        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let y = y_off + row as f64 * f64::from(layout.cell_height);
        let cell_w = f64::from(layout.cell_width);
        let max_cols = layout.cols as usize;

        for (point, cell) in cells {
            let col = point.column.0;
            if col >= max_cols {
                continue;
            }

            if cell.c == ' ' && cell.zerowidth().is_none() {
                continue;
            }

            let fg = cell_foreground(cell, *point, colors, default_fg, default_bg, selection);
            let mut text = String::new();
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }

            let x = x_off + col as f64 * cell_w;
            self.draw_glyph_text(
                scene,
                font_cx,
                &text,
                x,
                y,
                layout,
                config,
                GlyphStyle {
                    fg,
                    bold: cell.flags.intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
                    italic: cell
                        .flags
                        .intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
                    underline: cell.flags.contains(Flags::UNDERLINE),
                },
            );
        }
    }

    fn draw_glyph_text(
        &mut self,
        scene: &mut Scene,
        font_cx: &mut parley::FontContext,
        text: &str,
        x: f64,
        y: f64,
        layout: TerminalLayout,
        config: &Config,
        style: GlyphStyle,
    ) {
        if text.is_empty() {
            return;
        }

        let transform = Affine::translate((x, y));
        let mut builder = self.layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(layout.font_size));
        builder.push_default(StyleProperty::FontFamily(config_font_family(config)));
        builder.push_default(StyleProperty::Brush(style.fg));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.0)));

        let range = 0..text.len();
        if style.bold {
            builder.push(StyleProperty::FontWeight(FontWeight::new(700.0)), range.clone());
        }
        if style.italic {
            builder.push(
                StyleProperty::FontStyle(parley::FontStyle::Italic),
                range.clone(),
            );
        }
        if style.underline {
            builder.push(StyleProperty::Underline(true), range);
        }

        let mut text_layout = builder.build(text);
        text_layout.break_all_lines(None);
        for line in text_layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };

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

    fn render_cursor_glyph(
        &mut self,
        scene: &mut Scene,
        font_cx: &mut parley::FontContext,
        cursor: RenderableCursor,
        display_offset: usize,
        cell: &Cell,
        layout: TerminalLayout,
        config: &Config,
        default_fg: &Brush,
        default_bg: &Brush,
        colors: &Colors,
        selection: Option<SelectionRange>,
    ) {
        if cell.c == ' ' {
            return;
        }

        let Some(viewport) = point_to_viewport(display_offset, cursor.point) else {
            return;
        };
        let row = viewport.line;
        let col = viewport.column.0;
        if row >= layout.rows as usize || col >= layout.cols as usize {
            return;
        }

        let (mut fg, mut bg) = resolved_colors(cell, colors, default_fg, default_bg);
        apply_cell_colors(&mut fg, &mut bg, cell, cursor.point, selection);
        std::mem::swap(&mut fg, &mut bg);

        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let y = y_off + row as f64 * f64::from(layout.cell_height);
        let x = x_off + col as f64 * f64::from(layout.cell_width);
        let text: String = cell.c.to_string();
        self.draw_glyph_text(
            scene,
            font_cx,
            &text,
            x,
            y,
            layout,
            config,
            GlyphStyle {
                fg,
                bold: cell.flags.intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
                italic: cell
                    .flags
                    .intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
                underline: cell.flags.contains(Flags::UNDERLINE),
            },
        );
    }

    fn render_cursor(
        &self,
        scene: &mut Scene,
        cursor: RenderableCursor,
        display_offset: usize,
        layout: TerminalLayout,
        cursor_brush: &Brush,
    ) {
        if cursor.shape == CursorShape::Hidden {
            return;
        }

        let Some(viewport) = point_to_viewport(display_offset, cursor.point) else {
            return;
        };
        let col = viewport.column.0;
        let row = viewport.line;
        if row >= layout.rows as usize || col >= layout.cols as usize {
            return;
        }

        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let x = x_off + col as f64 * f64::from(layout.cell_width);
        let y = y_off + row as f64 * f64::from(layout.cell_height);
        let cell_w = f64::from(layout.cell_width);
        let cell_h = f64::from(layout.cell_height);

        match cursor.shape {
            CursorShape::Block => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    cursor_brush,
                    None,
                    &Rect::new(x, y, x + cell_w, y + cell_h),
                );
            }
            CursorShape::Underline => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    cursor_brush,
                    None,
                    &Rect::new(x, y + cell_h * 0.85, x + cell_w, y + cell_h),
                );
            }
            CursorShape::Beam => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    cursor_brush,
                    None,
                    &Rect::new(x, y, x + cell_w * 0.15, y + cell_h),
                );
            }
            CursorShape::HollowBlock => {
                let rect = Rect::new(x, y, x + cell_w, y + cell_h);
                scene.stroke(
                    &vello::kurbo::Stroke::new(2.0),
                    Affine::IDENTITY,
                    cursor_brush,
                    None,
                    &rect,
                );
            }
            CursorShape::Hidden => {}
        }
    }
}

fn apply_cell_colors(
    fg: &mut Brush,
    bg: &mut Brush,
    cell: &Cell,
    point: Point,
    selection: Option<SelectionRange>,
) {
    if cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(fg, bg);
    }
    if selection.is_some_and(|range| range.contains(point)) {
        std::mem::swap(fg, bg);
    }
}

fn cell_foreground(
    cell: &Cell,
    point: Point,
    colors: &Colors,
    default_fg: &Brush,
    default_bg: &Brush,
    selection: Option<SelectionRange>,
) -> Brush {
    let (mut fg, mut bg) = resolved_colors(cell, colors, default_fg, default_bg);
    apply_cell_colors(&mut fg, &mut bg, cell, point, selection);
    fg
}

fn config_font_family(config: &Config) -> FontFamily<'static> {
    let mut names: Vec<FontFamilyName<'static>> = Vec::new();

    if let Some(parsed) = FontFamilyName::parse(&config.font.family) {
        names.push(parsed.into_owned());
    }
    for fallback in &config.font.fallback {
        if let Some(parsed) = FontFamilyName::parse(fallback) {
            names.push(parsed.into_owned());
        }
    }
    names.push(FontFamilyName::Generic(parley::GenericFamily::Monospace));

    FontFamily::List(std::borrow::Cow::Owned(names))
}

fn resolved_colors(
    cell: &Cell,
    colors: &Colors,
    default_fg: &Brush,
    default_bg: &Brush,
) -> (Brush, Brush) {
    (
        color_to_brush(cell.fg, colors, default_fg.clone(), true),
        color_to_brush(cell.bg, colors, default_bg.clone(), false),
    )
}

fn color_to_brush(
    color: Color,
    colors: &Colors,
    fallback: Brush,
    is_foreground: bool,
) -> Brush {
    match color {
        Color::Spec(rgb) => brush_from_rgb(rgb),
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground if is_foreground => fallback,
            NamedColor::Background if !is_foreground => fallback,
            _ => colors[named as usize]
                .map(brush_from_rgb)
                .unwrap_or_else(|| brush_from_rgb(named_fallback(named))),
        },
        Color::Indexed(index) => colors[index as usize]
            .map(brush_from_rgb)
            .unwrap_or_else(|| brush_from_rgb(indexed_color(index))),
    }
}

fn brush_from_rgb(rgb: Rgb) -> Brush {
    Brush::Solid(vello::peniko::color::AlphaColor::from_rgb8(rgb.r, rgb.g, rgb.b))
}

fn brush_solid_eq(a: &Brush, b: &Brush) -> bool {
    match (a, b) {
        (Brush::Solid(a), Brush::Solid(b)) => a.to_rgba8() == b.to_rgba8(),
        _ => false,
    }
}

fn with_opacity(brush: Brush, opacity: f32) -> Brush {
    if opacity >= 1.0 - f32::EPSILON {
        return brush;
    }

    let Brush::Solid(color) = brush else {
        return brush;
    };
    let rgba = color.to_rgba8();
    let alpha = ((f32::from(rgba.a) / 255.0) * opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Brush::Solid(vello::peniko::color::AlphaColor::from_rgba8(
        rgba.r, rgba.g, rgba.b, alpha,
    ))
}

fn named_fallback(named: NamedColor) -> Rgb {
    match named {
        NamedColor::Black => Rgb { r: 0, g: 0, b: 0 },
        NamedColor::Red => Rgb { r: 205, g: 49, b: 49 },
        NamedColor::Green => Rgb { r: 13, g: 188, b: 121 },
        NamedColor::Yellow => Rgb { r: 229, g: 229, b: 16 },
        NamedColor::Blue => Rgb { r: 36, g: 114, b: 200 },
        NamedColor::Magenta => Rgb { r: 188, g: 63, b: 188 },
        NamedColor::Cyan => Rgb { r: 17, g: 168, b: 205 },
        NamedColor::White => Rgb { r: 229, g: 229, b: 229 },
        NamedColor::BrightBlack => Rgb { r: 102, g: 102, b: 102 },
        NamedColor::BrightRed => Rgb { r: 241, g: 76, b: 76 },
        NamedColor::BrightGreen => Rgb { r: 35, g: 209, b: 139 },
        NamedColor::BrightYellow => Rgb { r: 245, g: 245, b: 67 },
        NamedColor::BrightBlue => Rgb { r: 59, g: 142, b: 234 },
        NamedColor::BrightMagenta => Rgb { r: 214, g: 112, b: 214 },
        NamedColor::BrightCyan => Rgb { r: 41, g: 184, b: 219 },
        NamedColor::BrightWhite => Rgb { r: 255, g: 255, b: 255 },
        NamedColor::Foreground => Rgb { r: 204, g: 204, b: 204 },
        NamedColor::Background => Rgb { r: 30, g: 30, b: 30 },
        NamedColor::Cursor => Rgb { r: 255, g: 255, b: 255 },
        _ => Rgb { r: 204, g: 204, b: 204 },
    }
}

fn indexed_color(index: u8) -> Rgb {
    if index < 16 {
        named_fallback(match index {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        })
    } else if index < 232 {
        let index = index - 16;
        let r = (index / 36) * 51;
        let g = ((index / 6) % 6) * 51;
        let b = (index % 6) * 51;
        Rgb { r, g, b }
    } else {
        let gray = 8 + (index - 232) * 10;
        Rgb { r: gray, g: gray, b: gray }
    }
}

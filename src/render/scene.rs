use alacritty_terminal::index::Point;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{point_to_viewport, RenderableCursor};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use parley::layout::PositionedLayoutItem;
use parley::{FontFamily, FontFamilyName, FontWeight, LayoutContext, LineHeight, StyleProperty};
use vello::kurbo::{Affine, Rect};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill};
use vello::Scene;

use crate::config::Config;
use crate::render::frame::{FrameDamage, SearchMatch, TerminalFrame};
use crate::render::glyph_cache::{
    cache_layout_glyphs, emit_cached_glyphs, emit_glyph_run, GlyphCache, GlyphCacheKey,
};
use crate::render::TerminalLayout;

pub struct SceneBuilder {
    pub layout_cx: LayoutContext<Brush>,
    text_buf: String,
    font_family: Option<FontFamily<'static>>,
    glyph_cache: GlyphCache,
}

#[derive(Clone)]
struct GlyphStyle {
    fg: Brush,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl GlyphStyle {
    fn from_cell(
        cell: &Cell,
        point: Point,
        colors: &Colors,
        default_fg: &Brush,
        default_bg: &Brush,
        selection: Option<SelectionRange>,
        search_matches: &[SearchMatch],
        search_active: Option<usize>,
        link_hovers: &[SearchMatch],
        row: usize,
    ) -> Self {
        let col = point.column.0;
        let mut fg = cell_foreground(cell, point, colors, default_fg, default_bg, selection);
        let mut underline = cell.flags.contains(Flags::UNDERLINE);

        if link_match_contains(link_hovers, row, col) {
            fg = link_foreground_brush();
            underline = true;
        } else if search_active_match_contains(search_matches, search_active, row, col) {
            fg = search_active_foreground_brush();
        } else if search_match_contains(search_matches, row, col) {
            fg = search_foreground_brush();
        }

        Self {
            fg,
            bold: cell
                .flags
                .intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
            italic: cell.flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
            underline,
        }
    }
}

impl SceneBuilder {
    pub fn new() -> Self {
        Self {
            layout_cx: LayoutContext::new(),
            text_buf: String::with_capacity(256),
            font_family: None,
            glyph_cache: GlyphCache::new(),
        }
    }

    pub fn invalidate_font_cache(&mut self) {
        self.font_family = None;
        self.glyph_cache.clear();
    }

    pub fn update_terminal(
        &mut self,
        scene: &mut Scene,
        frame: &TerminalFrame,
        layout: TerminalLayout,
        config: &Config,
        font_cx: &mut parley::FontContext,
        terminal_opacity: f32,
        damage: &FrameDamage,
    ) {
        let default_fg: Brush = config.foreground_brush().into();
        let default_bg: Brush = config.background_brush().into();
        let cursor_brush: Brush = config.cursor_brush().into();
        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let col_count = layout.cols as usize;
        let selection = frame.selection;
        let search_matches = frame.search_matches.as_slice();
        let search_active = frame.search_active_match;
        let link_hovers = frame.link_hovers.as_slice();
        let colors = &frame.colors;
        let row_count = layout.rows as usize;
        let bg_opaque = with_opacity(default_bg.clone(), terminal_opacity);

        if damage.is_unchanged() {
            return;
        }

        if damage.is_full() {
            scene.reset();

            let width = f64::from(layout.cell_width) * f64::from(layout.cols);
            let height = f64::from(layout.cell_height) * f64::from(layout.rows);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &bg_opaque,
                None,
                &Rect::new(x_off, y_off, x_off + width, y_off + height),
            );

            for (row, cells) in frame.rows.iter().enumerate() {
                if !cells.is_empty() {
                    fill_row_backgrounds(
                        scene,
                        row,
                        cells,
                        layout,
                        x_off,
                        y_off,
                        col_count,
                        colors,
                        &default_fg,
                        &default_bg,
                        selection,
                        search_matches,
                        search_active,
                        link_hovers,
                    );
                }
                self.render_row(
                    scene,
                    font_cx,
                    row,
                    cells,
                    layout,
                    config,
                    &default_fg,
                    &default_bg,
                    colors,
                    selection,
                    search_matches,
                    search_active,
                    link_hovers,
                );
            }
        } else if let Some(lines) = damage.damaged_lines() {
            for bounds in lines {
                if bounds.line >= row_count {
                    continue;
                }
                clear_row_band(scene, bounds.line, layout, x_off, y_off, &bg_opaque);
                let cells = &frame.rows[bounds.line];
                if !cells.is_empty() {
                    fill_row_backgrounds(
                        scene,
                        bounds.line,
                        cells,
                        layout,
                        x_off,
                        y_off,
                        col_count,
                        colors,
                        &default_fg,
                        &default_bg,
                        selection,
                        search_matches,
                        search_active,
                        link_hovers,
                    );
                }
                self.render_row(
                    scene,
                    font_cx,
                    bounds.line,
                    cells,
                    layout,
                    config,
                    &default_fg,
                    &default_bg,
                    colors,
                    selection,
                    search_matches,
                    search_active,
                    link_hovers,
                );
            }
        }

        self.render_cursor(
            scene,
            frame.cursor,
            frame.display_offset,
            layout,
            &cursor_brush,
            frame
                .cursor_cell
                .as_ref()
                .map(cell_column_span)
                .unwrap_or(1),
        );

        if frame.cursor.shape == CursorShape::Block {
            if let Some(cell) = frame.cursor_cell.as_ref() {
                self.render_cursor_glyph(
                    scene,
                    font_cx,
                    frame.cursor,
                    frame.display_offset,
                    cell,
                    layout,
                    config,
                    &default_fg,
                    &default_bg,
                    colors,
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
        search_matches: &[SearchMatch],
        search_active: Option<usize>,
        link_hovers: &[SearchMatch],
    ) {
        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let y = y_off + row as f64 * f64::from(layout.cell_height);
        let cell_w = f64::from(layout.cell_width);
        let max_cols = layout.cols as usize;

        // Each cell is shaped and drawn at its own grid position. Coalescing a
        // run into a single Parley layout makes glyphs advance by the font's
        // natural width instead of the fixed `cell_width`, which accumulates a
        // horizontal drift across the row. Per-cell placement keeps the
        // monospace grid exact.
        for (point, cell) in cells {
            let col = point.column.0;
            if col >= max_cols {
                continue;
            }

            if cell.c == ' ' && cell.zerowidth().is_none() {
                continue;
            }

            let style = GlyphStyle::from_cell(
                cell,
                *point,
                colors,
                default_fg,
                default_bg,
                selection,
                search_matches,
                search_active,
                link_hovers,
                row,
            );

            self.text_buf.clear();
            push_cell_text(&mut self.text_buf, cell);
            let text = std::mem::take(&mut self.text_buf);

            let x = x_off + col as f64 * cell_w;
            self.draw_glyph_text(scene, font_cx, &text, x, y, layout, config, style);

            self.text_buf = text;
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

        let cache_key = GlyphCacheKey::new(
            text,
            style.bold,
            style.italic,
            style.underline,
            layout.font_size,
        );

        if let Some(cached) = self.glyph_cache.get(&cache_key) {
            emit_cached_glyphs(scene, cached, &style.fg, x, y);
            return;
        }

        let font_family = self.font_family(config).clone();
        let transform = Affine::translate((x, y));
        let mut builder = self.layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(layout.font_size));
        builder.push_default(StyleProperty::FontFamily(font_family));
        builder.push_default(StyleProperty::Brush(style.fg.clone()));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.0)));

        let range = 0..text.len();
        if style.bold {
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(700.0)),
                range.clone(),
            );
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

        if let Some(cached) = cache_layout_glyphs(&text_layout) {
            self.glyph_cache.insert(cache_key, cached.clone());
            emit_cached_glyphs(scene, &cached, &style.fg, x, y);
            return;
        }

        for line in text_layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                emit_glyph_run(scene, glyph_run, transform);
            }
        }
    }

    fn font_family(&mut self, config: &Config) -> &FontFamily<'static> {
        if self.font_family.is_none() {
            self.font_family = Some(config_font_family(config));
        }
        self.font_family.as_ref().unwrap()
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

        self.text_buf.clear();
        self.text_buf.push(cell.c);
        let text = std::mem::take(&mut self.text_buf);

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
                bold: cell
                    .flags
                    .intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
                italic: cell.flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
                underline: cell.flags.contains(Flags::UNDERLINE),
            },
        );

        self.text_buf = text;
    }

    fn render_cursor(
        &self,
        scene: &mut Scene,
        cursor: RenderableCursor,
        display_offset: usize,
        layout: TerminalLayout,
        cursor_brush: &Brush,
        cell_span: usize,
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
        let cell_w = f64::from(layout.cell_width) * cell_span as f64;
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

fn clear_row_band(
    scene: &mut Scene,
    row: usize,
    layout: TerminalLayout,
    x_off: f64,
    y_off: f64,
    bg: &Brush,
) {
    let y0 = (y_off + row as f64 * f64::from(layout.cell_height)).round();
    let y1 = (y_off + (row + 1) as f64 * f64::from(layout.cell_height)).round();
    let x0 = x_off.round();
    let x1 = (x_off + f64::from(layout.cell_width) * f64::from(layout.cols)).round();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        bg,
        None,
        &Rect::new(x0, y0, x1, y1),
    );
}

fn fill_row_backgrounds(
    scene: &mut Scene,
    row: usize,
    cells: &[(Point, Cell)],
    layout: TerminalLayout,
    x_off: f64,
    y_off: f64,
    col_count: usize,
    colors: &Colors,
    default_fg: &Brush,
    default_bg: &Brush,
    selection: Option<SelectionRange>,
    search_matches: &[SearchMatch],
    search_active: Option<usize>,
    _link_hovers: &[SearchMatch],
) {
    let cell_w = f64::from(layout.cell_width);
    let y0 = (y_off + row as f64 * f64::from(layout.cell_height)).round();
    let y1 = (y_off + (row + 1) as f64 * f64::from(layout.cell_height)).round();

    let mut segments: Vec<(usize, usize, Brush)> = Vec::new();
    for (point, cell) in cells {
        let col = point.column.0;
        if col >= col_count {
            continue;
        }

        let (mut fg, mut bg) = resolved_colors(cell, colors, default_fg, default_bg);
        apply_cell_colors(&mut fg, &mut bg, cell, *point, selection);
        if search_active_match_contains(search_matches, search_active, row, col) {
            bg = search_active_background_brush();
        } else if search_match_contains(search_matches, row, col) {
            bg = search_background_brush();
        }

        if brush_solid_eq(&bg, default_bg) {
            continue;
        }

        let span = cell_column_span(cell);
        segments.push((col, span, bg));
    }

    if segments.is_empty() {
        return;
    }

    segments.sort_by_key(|(col, _, _)| *col);

    let mut run_start = segments[0].0;
    let mut run_end = segments[0].0 + segments[0].1;
    let mut run_bg = segments[0].2.clone();

    for (col, span, bg) in segments.into_iter().skip(1) {
        if brush_solid_eq(&bg, &run_bg) && col == run_end {
            run_end = col + span;
        } else {
            let x0 = (x_off + run_start as f64 * cell_w).round();
            let x1 = (x_off + run_end as f64 * cell_w).round();
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &run_bg,
                None,
                &Rect::new(x0, y0, x1, y1),
            );
            run_start = col;
            run_end = col + span;
            run_bg = bg;
        }
    }

    let x0 = (x_off + run_start as f64 * cell_w).round();
    let x1 = (x_off + run_end as f64 * cell_w).round();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &run_bg,
        None,
        &Rect::new(x0, y0, x1, y1),
    );
}

fn search_match_contains(matches: &[SearchMatch], row: usize, col: usize) -> bool {
    matches
        .iter()
        .any(|m| m.row == row && col >= m.start_col && col < m.end_col)
}

fn search_active_match_contains(
    matches: &[SearchMatch],
    active: Option<usize>,
    row: usize,
    col: usize,
) -> bool {
    active.is_some_and(|index| {
        matches
            .get(index)
            .is_some_and(|m| m.row == row && col >= m.start_col && col < m.end_col)
    })
}

fn link_match_contains(matches: &[SearchMatch], row: usize, col: usize) -> bool {
    matches
        .iter()
        .any(|m| m.row == row && col >= m.start_col && col < m.end_col)
}

fn search_background_brush() -> Brush {
    Brush::Solid(AlphaColor::from_rgba8(255, 214, 102, 210))
}

fn search_active_background_brush() -> Brush {
    Brush::Solid(AlphaColor::from_rgba8(255, 168, 40, 230))
}

fn search_foreground_brush() -> Brush {
    Brush::Solid(AlphaColor::from_rgba8(20, 20, 20, 255))
}

fn search_active_foreground_brush() -> Brush {
    Brush::Solid(AlphaColor::from_rgba8(255, 255, 255, 255))
}

fn link_foreground_brush() -> Brush {
    Brush::Solid(AlphaColor::from_rgba8(120, 180, 255, 255))
}

fn cell_column_span(cell: &Cell) -> usize {
    if cell.flags.contains(Flags::WIDE_CHAR) {
        2
    } else {
        1
    }
}

fn push_cell_text(buf: &mut String, cell: &Cell) {
    buf.push(cell.c);
    if let Some(zerowidth) = cell.zerowidth() {
        buf.extend(zerowidth);
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

fn color_to_brush(color: Color, colors: &Colors, fallback: Brush, is_foreground: bool) -> Brush {
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
    Brush::Solid(vello::peniko::color::AlphaColor::from_rgb8(
        rgb.r, rgb.g, rgb.b,
    ))
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
        NamedColor::Red => Rgb {
            r: 205,
            g: 49,
            b: 49,
        },
        NamedColor::Green => Rgb {
            r: 13,
            g: 188,
            b: 121,
        },
        NamedColor::Yellow => Rgb {
            r: 229,
            g: 229,
            b: 16,
        },
        NamedColor::Blue => Rgb {
            r: 36,
            g: 114,
            b: 200,
        },
        NamedColor::Magenta => Rgb {
            r: 188,
            g: 63,
            b: 188,
        },
        NamedColor::Cyan => Rgb {
            r: 17,
            g: 168,
            b: 205,
        },
        NamedColor::White => Rgb {
            r: 229,
            g: 229,
            b: 229,
        },
        NamedColor::BrightBlack => Rgb {
            r: 102,
            g: 102,
            b: 102,
        },
        NamedColor::BrightRed => Rgb {
            r: 241,
            g: 76,
            b: 76,
        },
        NamedColor::BrightGreen => Rgb {
            r: 35,
            g: 209,
            b: 139,
        },
        NamedColor::BrightYellow => Rgb {
            r: 245,
            g: 245,
            b: 67,
        },
        NamedColor::BrightBlue => Rgb {
            r: 59,
            g: 142,
            b: 234,
        },
        NamedColor::BrightMagenta => Rgb {
            r: 214,
            g: 112,
            b: 214,
        },
        NamedColor::BrightCyan => Rgb {
            r: 41,
            g: 184,
            b: 219,
        },
        NamedColor::BrightWhite => Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        NamedColor::Foreground => Rgb {
            r: 204,
            g: 204,
            b: 204,
        },
        NamedColor::Background => Rgb {
            r: 30,
            g: 30,
            b: 30,
        },
        NamedColor::Cursor => Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        _ => Rgb {
            r: 204,
            g: 204,
            b: 204,
        },
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
        Rgb {
            r: (index / 36) * 51,
            g: ((index / 6) % 6) * 51,
            b: (index % 6) * 51,
        }
    } else {
        let gray = 8 + (index - 232) * 10;
        Rgb {
            r: gray,
            g: gray,
            b: gray,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_char_span_is_two_columns() {
        let cell = Cell {
            c: '你',
            flags: Flags::WIDE_CHAR,
            ..Cell::default()
        };
        assert_eq!(cell_column_span(&cell), 2);
    }
}

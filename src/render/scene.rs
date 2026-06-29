use alacritty_terminal::index::Point;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{point_to_viewport, RenderableCursor};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use parley::layout::PositionedLayoutItem;
use parley::{FontFamily, FontFamilyName, FontFeatures, FontWeight, LayoutContext, LineHeight, StyleProperty};
use vello::kurbo::{Affine, BezPath, Line, Rect};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill, Mix};
use vello::Scene;

use crate::config::Config;
use crate::render::atlas::{draw_atlas_glyph, pop_row_clip, push_row_clip, row_band_clip_rect, GlyphAtlas};
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
    glyph_atlas: Option<GlyphAtlas>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnderlineKind {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone)]
struct GlyphStyle {
    fg: Brush,
    bold: bool,
    italic: bool,
    underline: UnderlineKind,
    strikethrough: bool,
    blink: bool,
}

fn underline_kind_from_flags(flags: Flags) -> UnderlineKind {
    if flags.contains(Flags::UNDERCURL) {
        UnderlineKind::Curly
    } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
        UnderlineKind::Double
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        UnderlineKind::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        UnderlineKind::Dashed
    } else if flags.contains(Flags::UNDERLINE) {
        UnderlineKind::Single
    } else {
        UnderlineKind::None
    }
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
        let mut underline = underline_kind_from_flags(cell.flags);

        if link_match_contains(link_hovers, row, col) {
            fg = link_foreground_brush();
            if underline == UnderlineKind::None {
                underline = UnderlineKind::Single;
            }
        } else if search_active_match_contains(search_matches, search_active, row, col) {
            fg = search_active_foreground_brush();
        } else if search_match_contains(search_matches, row, col) {
            fg = search_foreground_brush();
        }

        Self {
            fg: {
                let mut brush = fg;
                if cell.flags.intersects(Flags::DIM | Flags::DIM_BOLD) {
                    brush = with_opacity(brush, 0.55);
                }
                brush
            },
            bold: cell
                .flags
                .intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
            italic: cell.flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
            underline,
            strikethrough: cell.flags.contains(Flags::STRIKEOUT),
            blink: cell.flags.contains(Flags::BLINK),
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
            glyph_atlas: None,
        }
    }

    fn ensure_glyph_atlas(&mut self, config: &Config) {
        if config.font.glyph_atlas && self.glyph_atlas.is_none() {
            self.glyph_atlas = GlyphAtlas::new(config);
        }
        if !config.font.glyph_atlas {
            self.glyph_atlas = None;
        }
    }

    pub fn invalidate_font_cache(&mut self) {
        self.font_family = None;
        self.glyph_cache.clear();
        if let Some(atlas) = self.glyph_atlas.as_mut() {
            atlas.clear();
        }
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
        ime_preedit: Option<&str>,
    ) {
        let text_blink_visible = frame.text_blink_visible;
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

        self.ensure_glyph_atlas(config);

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
                    text_blink_visible,
                );
            }
        } else if let Some(lines) = damage.damaged_lines() {
            for bounds in lines {
                if bounds.line >= row_count {
                    continue;
                }
                let clip = row_band_clip_rect(bounds.line, layout, x_off, y_off);
                push_row_clip(scene, &clip);
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
                    text_blink_visible,
                );
                pop_row_clip(scene);
            }
        }

        if frame.cursor_visible {
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

        self.render_scrollbar(scene, frame, layout, x_off, y_off);

        if let Some(preedit) = ime_preedit.filter(|text| !text.is_empty()) {
            self.render_ime_preedit(
                scene,
                font_cx,
                frame.cursor,
                frame.display_offset,
                preedit,
                layout,
                config,
                &default_fg,
            );
        }
    }

    fn render_ime_preedit(
        &mut self,
        scene: &mut Scene,
        font_cx: &mut parley::FontContext,
        cursor: RenderableCursor,
        display_offset: usize,
        preedit: &str,
        layout: TerminalLayout,
        config: &Config,
        default_fg: &Brush,
    ) {
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

        self.text_buf.clear();
        self.text_buf.push_str(preedit);
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
                fg: default_fg.clone(),
                bold: false,
                italic: false,
                underline: UnderlineKind::Single,
                strikethrough: false,
                blink: false,
            },
            false,
        );
        self.text_buf = text;
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
        text_blink_visible: bool,
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
        if config.font.ligatures {
            self.render_row_ligature_runs(
                scene,
                font_cx,
                row,
                cells,
                layout,
                config,
                default_fg,
                default_bg,
                colors,
                selection,
                search_matches,
                search_active,
                link_hovers,
                x_off,
                y,
                cell_w,
                max_cols,
                text_blink_visible,
            );
            return;
        }

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

            if !should_draw_blink_glyph(&style, text_blink_visible) {
                continue;
            }

            self.text_buf.clear();
            push_cell_text(&mut self.text_buf, cell);
            let text = std::mem::take(&mut self.text_buf);

            let x = x_off + col as f64 * cell_w;
            self.draw_glyph_text(scene, font_cx, &text, x, y, layout, config, style.clone(), false);

            if style.strikethrough {
                let cell_h = f64::from(layout.cell_height);
                let strike_y = y + cell_h * 0.55;
                scene.stroke(
                    &vello::kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    &style.fg,
                    None,
                    &Line::new((x, strike_y), (x + cell_w, strike_y)),
                );
            }

            if style.underline != UnderlineKind::None {
                render_cell_underline(
                    scene,
                    x,
                    y,
                    cell_w,
                    f64::from(layout.cell_height),
                    &style.fg,
                    style.underline,
                );
            }

            self.text_buf = text;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row_ligature_runs(
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
        x_off: f64,
        y: f64,
        cell_w: f64,
        max_cols: usize,
        text_blink_visible: bool,
    ) {
        let cell_h = f64::from(layout.cell_height);
        let mut index = 0;
        while index < cells.len() {
            let (point, cell) = &cells[index];
            let col = point.column.0;
            if col >= max_cols || (cell.c == ' ' && cell.zerowidth().is_none()) {
                index += 1;
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
            let start_col = col;
            let mut run_cols = cell_column_span(cell);
            index += 1;

            while index < cells.len() {
                let (next_point, next_cell) = &cells[index];
                let next_col = next_point.column.0;
                if next_col >= max_cols {
                    break;
                }
                if next_cell.c == ' ' && next_cell.zerowidth().is_none() {
                    break;
                }
                let next_style = GlyphStyle::from_cell(
                    next_cell,
                    *next_point,
                    colors,
                    default_fg,
                    default_bg,
                    selection,
                    search_matches,
                    search_active,
                    link_hovers,
                    row,
                );
                if !glyph_styles_match(&style, &next_style) {
                    break;
                }
                if next_col != start_col + run_cols {
                    break;
                }
                push_cell_text(&mut self.text_buf, next_cell);
                run_cols += cell_column_span(next_cell);
                index += 1;
            }

            let text = std::mem::take(&mut self.text_buf);
            let x = x_off + start_col as f64 * cell_w;
            let run_width = run_cols as f64 * cell_w;
            if should_draw_blink_glyph(&style, text_blink_visible) {
                scene.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    1.0,
                    Affine::IDENTITY,
                    &Rect::new(x, y, x + run_width, y + cell_h),
                );
                self.draw_glyph_text(
                    scene,
                    font_cx,
                    &text,
                    x,
                    y,
                    layout,
                    config,
                    style.clone(),
                    true,
                );
                scene.pop_layer();

                for col_offset in 0..run_cols {
                    let cx = x + col_offset as f64 * cell_w;
                    if style.strikethrough {
                        let strike_y = y + cell_h * 0.55;
                        scene.stroke(
                            &vello::kurbo::Stroke::new(1.0),
                            Affine::IDENTITY,
                            &style.fg,
                            None,
                            &Line::new((cx, strike_y), (cx + cell_w, strike_y)),
                        );
                    }
                    if style.underline != UnderlineKind::None {
                        render_cell_underline(
                            scene,
                            cx,
                            y,
                            cell_w,
                            cell_h,
                            &style.fg,
                            style.underline,
                        );
                    }
                }
            }

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
        ligatures: bool,
    ) {
        if text.is_empty() {
            return;
        }

        if config.font.glyph_atlas && !ligatures && atlas_text_candidate(text) {
            if let Some(atlas) = self.glyph_atlas.as_mut() {
                if let Some(entry) = atlas.get_or_rasterize_text(
                    text,
                    layout.font_size,
                    style.bold,
                    style.italic,
                    &style.fg,
                    config,
                ) {
                    draw_atlas_glyph(scene, entry, x, y, layout.cell_height);
                    return;
                }
            }
        }

        let cache_key = GlyphCacheKey::new(
            text,
            style.bold,
            style.italic,
            style.underline == UnderlineKind::Single,
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
        if style.underline == UnderlineKind::Single {
            builder.push(StyleProperty::Underline(true), range.clone());
        }
        if ligatures {
            builder.push(
                StyleProperty::FontFeatures(FontFeatures::Source(std::borrow::Cow::Borrowed(
                    "liga,calt",
                ))),
                range.clone(),
            );
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
                underline: underline_kind_from_flags(cell.flags),
                strikethrough: cell.flags.contains(Flags::STRIKEOUT),
                blink: cell.flags.contains(Flags::BLINK),
            },
            false,
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

    fn render_scrollbar(
        &self,
        scene: &mut Scene,
        frame: &TerminalFrame,
        layout: TerminalLayout,
        x_off: f64,
        y_off: f64,
    ) {
        if frame.scroll_history == 0 {
            return;
        }
        let width = f64::from(layout.cell_width) * f64::from(layout.cols);
        let height = f64::from(layout.cell_height) * f64::from(layout.rows);
        let track_w = 4.0;
        let track_x = x_off + width - track_w - 1.0;
        let track_bg = Brush::Solid(AlphaColor::from_rgba8(255, 255, 255, 40));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &track_bg,
            None,
            &Rect::new(track_x, y_off, track_x + track_w, y_off + height),
        );
        let visible = layout.rows as f64;
        let total = visible + frame.scroll_history as f64;
        let thumb_h = (height * (visible / total)).max(12.0);
        let max_offset = frame.scroll_history as f64;
        let frac = frame.display_offset as f64 / max_offset.max(1.0);
        let thumb_y = y_off + (height - thumb_h) * frac;
        let thumb = Brush::Solid(AlphaColor::from_rgba8(180, 180, 180, 180));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &thumb,
            None,
            &Rect::new(track_x, thumb_y, track_x + track_w, thumb_y + thumb_h),
        );
    }
}

fn render_cell_underline(
    scene: &mut Scene,
    x: f64,
    y: f64,
    cell_w: f64,
    cell_h: f64,
    brush: &Brush,
    kind: UnderlineKind,
) {
    let base_y = y + cell_h * 0.88;
    match kind {
        UnderlineKind::None => {}
        UnderlineKind::Single => {
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                brush,
                None,
                &Line::new((x, base_y), (x + cell_w, base_y)),
            );
        }
        UnderlineKind::Double => {
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                brush,
                None,
                &Line::new((x, base_y), (x + cell_w, base_y)),
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                brush,
                None,
                &Line::new((x, base_y - 3.0), (x + cell_w, base_y - 3.0)),
            );
        }
        UnderlineKind::Dashed => {
            let stroke = vello::kurbo::Stroke::new(1.0).with_dashes(0.0, &[4.0, 3.0]);
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                brush,
                None,
                &Line::new((x, base_y), (x + cell_w, base_y)),
            );
        }
        UnderlineKind::Dotted => {
            let stroke = vello::kurbo::Stroke::new(1.0).with_dashes(0.0, &[1.0, 2.5]);
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                brush,
                None,
                &Line::new((x, base_y), (x + cell_w, base_y)),
            );
        }
        UnderlineKind::Curly => {
            let amplitude = 2.0;
            let wavelength = 6.0;
            let steps = (cell_w / 2.0).ceil().max(1.0) as i32;
            let mut path = BezPath::new();
            for step in 0..=steps {
                let px = x + (step as f64 / steps as f64) * cell_w;
                let py = base_y + amplitude * ((px - x) / wavelength * std::f64::consts::TAU).sin();
                if step == 0 {
                    path.move_to((px, py));
                } else {
                    path.line_to((px, py));
                }
            }
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                brush,
                None,
                &path,
            );
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

fn glyph_styles_match(a: &GlyphStyle, b: &GlyphStyle) -> bool {
    brush_solid_eq(&a.fg, &b.fg)
        && a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
        && a.blink == b.blink
}

fn should_draw_blink_glyph(style: &GlyphStyle, text_blink_visible: bool) -> bool {
    !style.blink || text_blink_visible
}

fn push_cell_text(buf: &mut String, cell: &Cell) {
    buf.push(cell.c);
    if let Some(zerowidth) = cell.zerowidth() {
        buf.extend(zerowidth);
    }
}

fn atlas_text_candidate(text: &str) -> bool {
    if text.is_empty() || text.chars().count() > 8 {
        return false;
    }
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii() {
        return chars.all(|ch| ch.is_ascii() || is_combining_mark(ch));
    }
    text.chars().count() == 1
}

fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
    )
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
    use alacritty_terminal::index::{Column, Line};

    #[test]
    fn wide_char_span_is_two_columns() {
        let cell = Cell {
            c: '你',
            flags: Flags::WIDE_CHAR,
            ..Cell::default()
        };
        assert_eq!(cell_column_span(&cell), 2);
    }

    #[test]
    fn blink_style_is_detected_from_cell_flag() {
        let cell = Cell {
            c: 'X',
            flags: Flags::BLINK,
            ..Cell::default()
        };
        let style = GlyphStyle::from_cell(
            &cell,
            Point::new(Line(0), Column(0)),
            &Colors::default(),
            &Brush::Solid(AlphaColor::from_rgba8(200, 200, 200, 255)),
            &Brush::Solid(AlphaColor::from_rgba8(20, 20, 20, 255)),
            None,
            &[],
            None,
            &[],
            0,
        );
        assert!(style.blink);
    }

    #[test]
    fn combining_marks_are_pushed_into_cell_text() {
        let mut cell = Cell {
            c: 'e',
            ..Cell::default()
        };
        cell.push_zerowidth('\u{0301}');
        let mut buf = String::new();
        push_cell_text(&mut buf, &cell);
        assert_eq!(buf, "e\u{0301}");
        assert!(atlas_text_candidate(&buf));
    }

    #[test]
    fn combining_marks_do_not_split_ligature_runs_by_style() {
        let base = GlyphStyle {
            fg: Brush::Solid(AlphaColor::from_rgba8(200, 200, 200, 255)),
            bold: false,
            italic: false,
            underline: UnderlineKind::None,
            strikethrough: false,
            blink: false,
        };
        let with_blink = GlyphStyle {
            blink: true,
            ..base.clone()
        };
        assert!(!glyph_styles_match(&base, &with_blink));
    }

    #[test]
    fn multiple_combining_marks_are_appended_in_order() {
        let mut cell = Cell {
            c: 'a',
            ..Cell::default()
        };
        cell.push_zerowidth('\u{0301}'); // acute
        cell.push_zerowidth('\u{0308}'); // diaeresis
        let mut buf = String::new();
        push_cell_text(&mut buf, &cell);
        assert_eq!(buf, "a\u{0301}\u{0308}");
        assert!(atlas_text_candidate(&buf));
    }

    #[test]
    fn combining_marks_from_extended_blocks_are_recognized() {
        assert!(is_combining_mark('\u{20D0}')); // combining left harpoon above
        assert!(is_combining_mark('\u{1AB0}')); // combining doubly circumflex accent
        assert!(!is_combining_mark('e'));
    }

    #[test]
    fn wide_char_with_combining_mark_builds_single_text_run() {
        let mut cell = Cell {
            c: '你',
            flags: Flags::WIDE_CHAR,
            ..Cell::default()
        };
        cell.push_zerowidth('\u{0301}');
        let mut buf = String::new();
        push_cell_text(&mut buf, &cell);
        assert_eq!(buf, "你\u{0301}");
        // Non-ASCII base with combining marks exceeds single-scalar atlas path.
        assert!(!atlas_text_candidate(&buf));
    }

    #[test]
    fn wide_char_span_unchanged_with_zerowidth_marks() {
        let mut cell = Cell {
            c: '你',
            flags: Flags::WIDE_CHAR,
            ..Cell::default()
        };
        cell.push_zerowidth('\u{0301}');
        assert_eq!(cell_column_span(&cell), 2);
    }

    #[test]
    fn atlas_text_candidate_rejects_long_runs() {
        assert!(!atlas_text_candidate("abcdefghi")); // 9 chars
        assert!(atlas_text_candidate("abcdefgh")); // 8 chars
    }

    #[test]
    fn atlas_text_candidate_accepts_ascii_with_combining_marks() {
        assert!(atlas_text_candidate("e\u{0301}\u{0308}"));
        // Precomposed single scalar uses the non-ASCII single-char atlas path.
        assert!(atlas_text_candidate("é"));
    }

    #[test]
    fn atlas_text_candidate_accepts_single_non_ascii_scalar() {
        assert!(atlas_text_candidate("你"));
        assert!(!atlas_text_candidate("你你"));
    }
}

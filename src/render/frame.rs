use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{
    point_to_viewport, viewport_to_point, LineDamageBounds, RenderableCursor, Term, TermDamage,
};
use alacritty_terminal::vte::ansi::CursorShape;

/// Describes which terminal rows changed since the last frame.
#[derive(Clone, Debug, Default)]
pub enum FrameDamage {
    #[default]
    Full,
    Partial(Vec<LineDamageBounds>),
    /// Terminal woke up but reported no changed rows.
    None,
}

impl FrameDamage {
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn damaged_lines(&self) -> Option<&[LineDamageBounds]> {
        match self {
            Self::Partial(lines) => Some(lines.as_slice()),
            Self::Full | Self::None => None,
        }
    }

    pub fn for_each_damaged_row(&self, row_count: usize, mut f: impl FnMut(usize)) {
        match self {
            Self::Full => {
                for row in 0..row_count {
                    f(row);
                }
            }
            Self::Partial(lines) => {
                for bounds in lines {
                    if bounds.line < row_count {
                        f(bounds.line);
                    }
                }
            }
            Self::None => {}
        }
    }
}

/// Owned snapshot of visible terminal content, captured under a short lock.
pub struct TerminalFrame {
    pub rows: Vec<Vec<(Point, Cell)>>,
    pub cursor: RenderableCursor,
    pub cursor_cell: Option<Cell>,
    pub selection: Option<SelectionRange>,
    pub search_matches: Vec<SearchMatch>,
    /// Index into `search_matches` for the currently focused result.
    pub search_active_match: Option<usize>,
    /// URL spans to highlight when Ctrl is held over a link.
    pub link_hovers: Vec<SearchMatch>,
    pub display_offset: usize,
    pub colors: Arc<Colors>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// Reusable row buckets used to avoid allocating a fresh `Vec` per row each frame.
pub type TerminalRowBuffer = Vec<Vec<(Point, Cell)>>;

pub fn ensure_row_buffer(buffer: &mut TerminalRowBuffer, row_count: usize) {
    if buffer.len() != row_count {
        buffer.resize_with(row_count, Vec::new);
    }
    for row in buffer.iter_mut() {
        row.clear();
    }
}

fn read_term_damage<T: EventListener>(term: &mut Term<T>, force_full: bool) -> FrameDamage {
    if force_full {
        return FrameDamage::Full;
    }

    match term.damage() {
        TermDamage::Full => FrameDamage::Full,
        TermDamage::Partial(iter) => {
            let lines: Vec<_> = iter.collect();
            if lines.is_empty() {
                FrameDamage::None
            } else {
                FrameDamage::Partial(lines)
            }
        }
    }
}

fn capture_row_columns_from_grid<T: EventListener>(
    term: &Term<T>,
    row: usize,
    left: usize,
    right: usize,
    col_count: usize,
    display_offset: usize,
    cursor_point: Point,
    cursor_cell: &mut Option<Cell>,
    out: &mut Vec<(Point, Cell)>,
) {
    out.clear();
    let grid = term.grid();
    let right = right.min(col_count.saturating_sub(1));
    if left > right || left >= col_count {
        return;
    }

    for col in left..=right {
        let point = viewport_to_point(display_offset, Point::new(row, Column(col)));
        let cell = &grid[point];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        if point == cursor_point {
            *cursor_cell = Some(cell.clone());
        }
        out.push((point, cell.clone()));
    }
}

fn capture_full_into<T: EventListener>(
    buffer: &mut TerminalRowBuffer,
    frame: &mut TerminalFrame,
    term: &Term<T>,
    row_count: usize,
    col_count: usize,
) {
    ensure_row_buffer(buffer, row_count);
    let content = term.renderable_content();
    frame.display_offset = content.display_offset;
    frame.selection = content.selection;
    frame.colors = Arc::new(*content.colors);
    frame.cursor = content.cursor;
    frame.cursor_cell = None;

    for indexed in content.display_iter {
        if indexed.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }

        let point = indexed.point;
        let Some(viewport) = point_to_viewport(content.display_offset, point) else {
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

        if point == content.cursor.point {
            frame.cursor_cell = Some(indexed.cell.clone());
        }

        buffer[row].push((point, indexed.cell.clone()));
    }

    frame.rows = (0..row_count)
        .map(|i| std::mem::take(&mut buffer[i]))
        .collect();
}

fn capture_partial_into<T: EventListener>(
    frame: &mut TerminalFrame,
    term: &Term<T>,
    row_count: usize,
    col_count: usize,
    damaged_lines: &[LineDamageBounds],
    scratch: &mut Vec<(Point, Cell)>,
) {
    let content = term.renderable_content();
    let display_offset = content.display_offset;
    let cursor_point = content.cursor.point;

    if frame.rows.len() != row_count {
        frame.rows.resize_with(row_count, Vec::new);
    }

    for bounds in damaged_lines {
        if bounds.line >= row_count {
            continue;
        }

        // Recapture the entire damaged row rather than just the reported
        // `[left, right]` span. Merging a sub-range can leave stale cells from a
        // previous frame outside the span (e.g. the rightmost fastfetch palette
        // blocks getting "copied" and stretched to the right), so we replace the
        // whole row to guarantee no leftover cells survive.
        capture_row_columns_from_grid(
            term,
            bounds.line,
            0,
            col_count.saturating_sub(1),
            col_count,
            display_offset,
            cursor_point,
            &mut frame.cursor_cell,
            scratch,
        );
        std::mem::swap(&mut frame.rows[bounds.line], scratch);
    }

    frame.cursor = content.cursor;
    frame.selection = content.selection;
    frame.display_offset = display_offset;
    frame.colors = Arc::new(*content.colors);
}

/// Update `frame` from the terminal grid and reset damage tracking.
pub fn capture_terminal_frame<T: EventListener>(
    buffer: &mut TerminalRowBuffer,
    scratch: &mut Vec<(Point, Cell)>,
    frame: &mut TerminalFrame,
    term: &mut Term<T>,
    row_count: usize,
    col_count: usize,
    force_full: bool,
) -> FrameDamage {
    let damage = read_term_damage(term, force_full);

    match &damage {
        FrameDamage::Full => {
            capture_full_into(buffer, frame, term, row_count, col_count);
        }
        FrameDamage::Partial(lines) => {
            capture_partial_into(frame, term, row_count, col_count, lines, scratch);
        }
        FrameDamage::None => {
            let content = term.renderable_content();
            frame.cursor = content.cursor;
            frame.selection = content.selection;
            frame.display_offset = content.display_offset;
        }
    }

    term.reset_damage();
    damage
}

pub fn empty_terminal_frame(row_count: usize, colors: Arc<Colors>) -> TerminalFrame {
    TerminalFrame {
        rows: vec![Vec::new(); row_count],
        cursor: RenderableCursor {
            point: Point::new(Line(0), Column(0)),
            shape: CursorShape::Hidden,
        },
        cursor_cell: None,
        selection: None,
        search_matches: Vec::new(),
        search_active_match: None,
        link_hovers: Vec::new(),
        display_offset: 0,
        colors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::term::Config as TermConfig;
    use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

    struct Dims {
        cols: usize,
        rows: usize,
    }
    impl Dimensions for Dims {
        fn total_lines(&self) -> usize {
            self.rows
        }
        fn screen_lines(&self) -> usize {
            self.rows
        }
        fn columns(&self) -> usize {
            self.cols
        }
    }

    #[test]
    fn fastfetch_palette_does_not_extend() {
        let cols = 40usize;
        let rows = 6usize;
        let dims = Dims { cols, rows };
        let mut term = Term::new(TermConfig::default(), &dims, VoidListener);
        let mut parser: Processor = Processor::new();

        // Exact bytes fastfetch emits for the normal color palette: 8 background
        // colors, each followed by three spaces, then SGR reset and CR/LF.
        let palette = b"\x1b[40m   \x1b[41m   \x1b[42m   \x1b[43m   \x1b[44m   \x1b[45m   \x1b[46m   \x1b[47m   \x1b[m\r\n";
        parser.advance(&mut term, palette);

        let mut buffer: TerminalRowBuffer = Vec::new();
        let mut scratch: Vec<(Point, Cell)> = Vec::new();
        let mut frame = empty_terminal_frame(rows, Arc::new(Colors::default()));
        capture_terminal_frame(
            &mut buffer,
            &mut scratch,
            &mut frame,
            &mut term,
            rows,
            cols,
            true,
        );

        // Collect the columns whose captured background is white. They must stop at
        // the 24th column (8 blocks * 3 cells); anything past that is the bug where
        // the last block "stretches" to the right edge.
        let mut white_cols: Vec<usize> = Vec::new();
        for row in &frame.rows {
            for (point, cell) in row {
                if matches!(cell.bg, Color::Named(NamedColor::White)) {
                    white_cols.push(point.column.0);
                }
            }
        }
        white_cols.sort_unstable();
        let max_white = white_cols.last().copied();
        assert!(
            max_white.map(|c| c < 24).unwrap_or(true),
            "white background extended past the palette: cols={white_cols:?}"
        );
    }

    fn white_bg_cols(rows: &[Vec<(Point, Cell)>]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (point, cell) in row {
                if matches!(cell.bg, Color::Named(NamedColor::White)) {
                    out.push((r, point.column.0));
                }
            }
        }
        out.sort_unstable();
        out
    }

    #[test]
    fn partial_capture_matches_full_after_scroll() {
        let cols = 40usize;
        let rows = 6usize;
        let dims = Dims { cols, rows };
        let mut term = Term::new(TermConfig::default(), &dims, VoidListener);
        let mut parser: Processor = Processor::new();

        let mut buffer: TerminalRowBuffer = Vec::new();
        let mut scratch: Vec<(Point, Cell)> = Vec::new();
        let mut frame = empty_terminal_frame(rows, Arc::new(Colors::default()));

        // Print the palette near the bottom, then keep emitting prompt-like lines
        // that scroll the palette upward. Capture *partially* each step, exactly as
        // the live render loop does (force_full = false).
        let steps: &[&[u8]] = &[
            b"line1\r\nline2\r\nline3\r\n",
            b"\x1b[40m   \x1b[41m   \x1b[42m   \x1b[43m   \x1b[44m   \x1b[45m   \x1b[46m   \x1b[47m   \x1b[m\r\n",
            b"prompt$ \r\n",
            b"output here\r\n",
            b"another$ \r\n",
        ];
        for bytes in steps {
            parser.advance(&mut term, bytes);
            capture_terminal_frame(
                &mut buffer,
                &mut scratch,
                &mut frame,
                &mut term,
                rows,
                cols,
                false,
            );
        }

        let partial = white_bg_cols(&frame.rows);

        // Ground truth: a full capture of the identical terminal state.
        let mut truth = empty_terminal_frame(rows, Arc::new(Colors::default()));
        capture_terminal_frame(
            &mut buffer,
            &mut scratch,
            &mut truth,
            &mut term,
            rows,
            cols,
            true,
        );
        let full = white_bg_cols(&truth.rows);

        assert_eq!(
            partial, full,
            "partial capture left stale white cells after scroll: partial={partial:?} full={full:?}"
        );
    }
}

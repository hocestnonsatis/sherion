use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

/// Plain-text snapshot of every row in the terminal grid (scrollback + viewport).
pub fn capture_grid_lines<T: EventListener>(term: &Term<T>) -> Vec<String> {
    let grid = term.grid();
    let top = grid.topmost_line().0;
    let bottom = grid.bottommost_line().0;
    let cols = grid.columns();
    let mut lines = Vec::with_capacity((bottom - top + 1) as usize);
    let mut row = vec![' '; cols];

    for line in top..=bottom {
        row.fill(' ');
        for col in 0..cols {
            row[col] = grid[Point::new(Line(line), Column(col))].c;
        }
        lines.push(row.iter().collect());
    }
    lines
}

/// Replay captured lines into a fresh terminal, rebuilding scrollback via the VT parser.
pub fn replay_grid_lines<T: EventListener>(term: &mut Term<T>, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    for line in lines {
        parser.advance(term, line.as_bytes());
        parser.advance(term, b"\n");
    }
}

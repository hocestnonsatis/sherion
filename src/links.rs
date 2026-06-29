use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::{Cell, Flags};

use crate::render::frame::SearchMatch;

/// A URL detected in the terminal grid, possibly spanning multiple rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedLink {
    pub url: String,
    pub spans: Vec<SearchMatch>,
}

/// Find the URL at `(target_row, target_col)` and return all highlight spans.
pub fn detect_link_at(
    rows: &[Vec<(Point, Cell)>],
    col_count: usize,
    target_row: usize,
    target_col: usize,
) -> Option<DetectedLink> {
    if col_count == 0 || rows.get(target_row).is_none() {
        return None;
    }

    if let Some(link) = detect_hyperlink_at(rows, col_count, target_row, target_col) {
        return Some(link);
    }

    let window_start = target_row.saturating_sub(1);
    let window_end = (target_row + 1).min(rows.len().saturating_sub(1));
    let (combined, mapping) = build_combined_text(rows, col_count, window_start, window_end);

    let target_index = mapping
        .iter()
        .position(|(row, col)| *row == target_row && *col == target_col)?;

    let (start, end, url) = find_url_in_text(&combined, target_index)?;
    let spans = spans_from_mapping(&mapping, start, end);
    if spans.is_empty() {
        return None;
    }

    Some(DetectedLink { url, spans })
}

/// Return the URL string at a grid position (for opening links).
pub fn url_at_position(
    rows: &[Vec<(Point, Cell)>],
    col_count: usize,
    target_row: usize,
    target_col: usize,
) -> Option<String> {
    detect_link_at(rows, col_count, target_row, target_col).map(|link| link.url)
}

fn detect_hyperlink_at(
    rows: &[Vec<(Point, Cell)>],
    col_count: usize,
    target_row: usize,
    target_col: usize,
) -> Option<DetectedLink> {
    let cells = rows.get(target_row)?;
    let target_cell = cells
        .iter()
        .find(|(point, _)| point.column.0 == target_col)?;
    let hyperlink = target_cell.1.hyperlink()?;
    let url = hyperlink.uri().to_owned();
    if url.is_empty() {
        return None;
    }

    let link_id = hyperlink.id();
    let mut spans = Vec::new();

    for (row_idx, row_cells) in rows.iter().enumerate() {
        if let Some(span) = hyperlink_span_in_row(row_cells, col_count, link_id, row_idx) {
            spans.push(span);
        }
    }

    if spans.is_empty() {
        return None;
    }

    Some(DetectedLink { url, spans })
}

fn hyperlink_span_in_row(
    cells: &[(Point, Cell)],
    col_count: usize,
    link_id: &str,
    row: usize,
) -> Option<SearchMatch> {
    let mut start_col = None;
    let mut end_col = 0;

    for col in 0..col_count {
        let Some((_, cell)) = cells.iter().find(|(point, _)| point.column.0 == col) else {
            continue;
        };
        if cell.hyperlink().is_some_and(|link| link.id() == link_id) {
            start_col.get_or_insert(col);
            end_col = col + 1;
        } else if start_col.is_some() {
            break;
        }
    }

    let start_col = start_col?;
    Some(SearchMatch {
        row,
        start_col,
        end_col,
    })
}

/// Build combined text from a row window and map each char index to `(row, col)`.
fn build_combined_text(
    rows: &[Vec<(Point, Cell)>],
    col_count: usize,
    window_start: usize,
    window_end: usize,
) -> (String, Vec<(usize, usize)>) {
    let mut combined = String::new();
    let mut mapping = Vec::new();

    for row_idx in window_start..=window_end {
        if row_idx >= rows.len() {
            break;
        }

        let row_text = row_as_string(rows[row_idx].as_slice(), col_count);
        if row_idx > window_start {
            if !rows_should_join(rows, col_count, row_idx - 1, row_idx) {
                continue;
            }
            trim_trailing_spaces(&mut combined, &mut mapping);
            let leading_spaces = row_text.chars().take_while(|ch| *ch == ' ').count();
            for (offset, ch) in row_text.chars().skip(leading_spaces).enumerate() {
                mapping.push((row_idx, leading_spaces + offset));
                combined.push(ch);
            }
            continue;
        }

        for (col, ch) in row_text.chars().enumerate() {
            mapping.push((row_idx, col));
            combined.push(ch);
        }
    }

    (combined, mapping)
}

fn trim_trailing_spaces(combined: &mut String, mapping: &mut Vec<(usize, usize)>) {
    while combined.ends_with(' ') {
        combined.pop();
        mapping.pop();
    }
}

fn rows_should_join(
    rows: &[Vec<(Point, Cell)>],
    col_count: usize,
    prev_row: usize,
    curr_row: usize,
) -> bool {
    let Some(prev_cells) = rows.get(prev_row) else {
        return false;
    };
    if rows.get(curr_row).is_none() {
        return false;
    }

    if row_has_wrapline(prev_cells, col_count) {
        return true;
    }

    let prev_text = row_as_string(prev_cells, col_count);
    prev_text
        .chars()
        .nth(col_count.saturating_sub(1))
        .is_some_and(|ch| !ch.is_whitespace())
}

fn row_has_wrapline(cells: &[(Point, Cell)], _col_count: usize) -> bool {
    cells
        .iter()
        .any(|(_, cell)| cell.flags.contains(Flags::WRAPLINE))
}

fn spans_from_mapping(mapping: &[(usize, usize)], start: usize, end: usize) -> Vec<SearchMatch> {
    if start >= end || start >= mapping.len() {
        return Vec::new();
    }
    let end = end.min(mapping.len());
    let mut spans = Vec::new();
    let mut current_row = mapping[start].0;
    let mut span_start = mapping[start].1;
    let mut span_end = mapping[start].1 + 1;

    for &(row, col) in &mapping[start + 1..end] {
        if row == current_row && col == span_end {
            span_end = col + 1;
        } else {
            spans.push(SearchMatch {
                row: current_row,
                start_col: span_start,
                end_col: span_end,
            });
            current_row = row;
            span_start = col;
            span_end = col + 1;
        }
    }

    spans.push(SearchMatch {
        row: current_row,
        start_col: span_start,
        end_col: span_end,
    });

    spans
}

fn row_as_string(cells: &[(Point, Cell)], col_count: usize) -> String {
    let mut chars = vec![' '; col_count];
    for (point, cell) in cells {
        let col = point.column.0;
        if col < col_count {
            chars[col] = cell.c;
        }
    }
    chars.iter().collect()
}

fn normalize_detected_url(url: &str) -> String {
    let trimmed = url.trim_end_matches(['.', ',', ')', ']']);
    if trimmed.starts_with("www.") {
        format!("https://{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

fn find_url_in_text(text: &str, target_index: usize) -> Option<(usize, usize, String)> {
    const SCHEMES: &[&str] = &["https://", "http://", "file://", "mailto:"];

    for scheme in SCHEMES {
        let mut search_from = 0;
        while let Some(offset) = text[search_from..].find(scheme) {
            let start = search_from + offset;
            let rest = &text[start..];
            let raw_len = rest
                .char_indices()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(idx, _)| idx)
                .unwrap_or(rest.len());
            let raw = &rest[..raw_len];
            let trimmed = raw.trim_end_matches(['.', ',', ')', ']']);
            let start_col = text[..start].chars().count();
            let end_col = start_col + trimmed.chars().count();
            if target_index >= start_col && target_index < end_col {
                return Some((start_col, end_col, normalize_detected_url(trimmed)));
            }
            search_from = start + raw_len.max(1);
            if search_from >= text.len() {
                break;
            }
        }
    }

    let mut search_from = 0;
    while let Some(offset) = text[search_from..].find("www.") {
        let start = search_from + offset;
        let rest = &text[start..];
        let raw_len = rest
            .char_indices()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(rest.len());
        let raw = &rest[..raw_len];
        let trimmed = raw.trim_end_matches(['.', ',', ')', ']']);
        let start_col = text[..start].chars().count();
        let end_col = start_col + trimmed.chars().count();
        if target_index >= start_col && target_index < end_col {
            return Some((start_col, end_col, normalize_detected_url(trimmed)));
        }
        search_from = start + raw_len.max(1);
        if search_from >= text.len() {
            break;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::Column;

    fn cell(ch: char) -> Cell {
        let mut cell = Cell::default();
        cell.c = ch;
        cell
    }

    fn row_from_text_sized(text: &str, col_count: usize) -> Vec<(Point, Cell)> {
        let mut chars = vec![' '; col_count];
        for (col, ch) in text.chars().enumerate() {
            if col < col_count {
                chars[col] = ch;
            }
        }
        chars
            .into_iter()
            .enumerate()
            .map(|(col, ch)| {
                (
                    Point::new(alacritty_terminal::index::Line(0), Column(col)),
                    cell(ch),
                )
            })
            .collect()
    }

    fn row_from_text_with_wrap(text: &str, col_count: usize) -> Vec<(Point, Cell)> {
        let trimmed_len = text.trim_end().chars().count();
        let wrap_col = trimmed_len.saturating_sub(1);
        let mut chars = vec![' '; col_count];
        for (col, ch) in text.chars().enumerate() {
            if col < col_count {
                chars[col] = ch;
            }
        }
        chars
            .into_iter()
            .enumerate()
            .map(|(col, ch)| {
                let mut c = cell(ch);
                if col == wrap_col {
                    c.flags.insert(Flags::WRAPLINE);
                }
                (Point::new(alacritty_terminal::index::Line(0), Column(col)), c)
            })
            .collect()
    }

    #[test]
    fn detects_url_wrapped_across_two_rows() {
        let row0 = row_from_text_with_wrap("See https://example.com/very/l", 32);
        let row1 = row_from_text_sized("ong/path/here", 32);
        let rows = vec![row0, row1];

        let link = detect_link_at(&rows, 32, 1, 10).expect("wrapped url");
        assert_eq!(link.url, "https://example.com/very/long/path/here");
        assert!(link.spans.len() >= 2);
        assert_eq!(link.spans[0].row, 0);
        assert_eq!(link.spans[1].row, 1);
    }

    #[test]
    fn ignores_rows_separated_by_whitespace() {
        let row0 = row_from_text_sized("Visit https://example.com", 32);
        let row1 = row_from_text_sized("unrelated text", 32);
        let rows = vec![row0, row1];

        let link = detect_link_at(&rows, 32, 0, 6).expect("single row url");
        assert_eq!(link.url, "https://example.com");
        assert_eq!(link.spans.len(), 1);
    }

    #[test]
    fn normalizes_www_prefix() {
        let row0 = row_from_text_sized("open www.example.com", 32);
        let rows = vec![row0];

        let link = detect_link_at(&rows, 32, 0, 5).expect("www url");
        assert_eq!(link.url, "https://www.example.com");
    }
}

use winit::window::{CursorIcon, ResizeDirection};

/// Logical resize hit zone; scaled by window DPI in `border_size`.
pub const RESIZE_BORDER_LOGICAL: f64 = 6.0;

pub fn border_size(scale_factor: f64) -> f64 {
    RESIZE_BORDER_LOGICAL * scale_factor
}

/// Returns a resize direction when `(x, y)` is on a window edge or corner.
pub fn resize_direction_at(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    border: f64,
) -> Option<ResizeDirection> {
    if width <= border * 2.0 || height <= border * 2.0 {
        return None;
    }

    let west = x < border;
    let east = x > width - border;
    let north = y < border;
    let south = y > height - border;

    match (west, east, north, south) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (true, _, _, _) => Some(ResizeDirection::West),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (_, true, _, _) => Some(ResizeDirection::East),
        (_, _, true, _) => Some(ResizeDirection::North),
        (_, _, _, true) => Some(ResizeDirection::South),
        _ => None,
    }
}

pub fn cursor_for_direction(direction: ResizeDirection) -> CursorIcon {
    direction.into()
}

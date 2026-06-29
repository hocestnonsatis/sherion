use alacritty_terminal::index::Point;
use alacritty_terminal::term::TermMode;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::ModifiersState;

/// Returns true when the terminal has requested mouse event reporting.
pub fn mouse_mode_active(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
}

pub fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 8,
        MouseButton::Forward => 9,
        MouseButton::Other(_) => 0,
    }
}

pub fn wheel_button(lines: i32) -> u8 {
    if lines > 0 {
        64
    } else {
        65
    }
}

fn mouse_modifiers(modifiers: ModifiersState) -> u8 {
    let mut mods = 0;
    if modifiers.shift_key() {
        mods += 4;
    }
    if modifiers.alt_key() {
        mods += 8;
    }
    if modifiers.control_key() {
        mods += 16;
    }
    mods
}

/// Encode a mouse report for the PTY (SGR when `SGR_MOUSE` is set, legacy X10 otherwise).
pub fn encode_mouse_report(
    point: Point,
    button: u8,
    state: ElementState,
    modifiers: ModifiersState,
    mode: TermMode,
) -> Vec<u8> {
    let button = button.saturating_add(mouse_modifiers(modifiers));
    let col = point.column.0 + 1;
    let row = point.line.0 + 1;

    if mode.contains(TermMode::SGR_MOUSE) {
        let c = match state {
            ElementState::Pressed => 'M',
            ElementState::Released => 'm',
        };
        format!("\x1b[<{button};{col};{row}{c}").into_bytes()
    } else if matches!(state, ElementState::Released) {
        vec![
            0x1b,
            b'M',
            3 + mouse_modifiers(modifiers),
            (col.min(223) as u8).saturating_add(32),
            (row.min(223) as u8).saturating_add(32),
        ]
    } else {
        vec![
            0x1b,
            b'M',
            button.saturating_add(32),
            (col.min(223) as u8).saturating_add(32),
            (row.min(223) as u8).saturating_add(32),
        ]
    }
}

/// Motion event button code: drag uses pressed button + 32; bare motion uses 35.
pub fn motion_button(pressed: Option<MouseButton>) -> u8 {
    match pressed {
        Some(button) => button_code(button).saturating_add(32),
        None => 35,
    }
}

pub fn grid_point_from_layout(
    layout: &crate::render::TerminalLayout,
    display_offset: usize,
    x: f64,
    y: f64,
) -> Point {
    use alacritty_terminal::index::Column;
    use alacritty_terminal::term::viewport_to_point;

    let x = x - f64::from(layout.content_offset_x);
    let y = y - f64::from(layout.content_offset_y);
    let col = (x / f64::from(layout.cell_width)).floor().max(0.0) as usize;
    let row = (y / f64::from(layout.cell_height)).floor().max(0.0) as usize;
    let max_row = layout.rows.saturating_sub(1) as usize;
    let max_col = layout.cols.saturating_sub(1) as usize;
    let viewport = Point::new(row.min(max_row), Column(col.min(max_col)));
    viewport_to_point(display_offset, viewport)
}

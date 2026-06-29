use alacritty_terminal::term::TermMode;
use vte::ansi::ModifyOtherKeys;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use crate::keybindings::KeybindingsConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    SendToTerminal(Vec<u8>),
    NewTab,
    DuplicateTab,
    CloseTab,
    DetachTab,
    NextTab,
    PrevTab,
    SelectTab(usize),
    RenameTab,
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ClearScrollback,
    OpenSearch,
    OpenCommandPalette,
    SplitVertical,
    SplitHorizontal,
    ClosePane,
    FocusPaneLeft,
    FocusPaneRight,
    FocusPaneUp,
    FocusPaneDown,
    ScrollPageUp,
    ScrollPageDown,
    JumpPromptPrev,
    JumpPromptNext,
    ToggleFullscreen,
}

pub fn key_event_to_action(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: TermMode,
    modify_other_keys: ModifyOtherKeys,
    ime_composing: bool,
    bindings: &KeybindingsConfig,
) -> Option<KeyAction> {
    if event.state == ElementState::Pressed && !event.repeat {
        if let Some(action) = bindings.resolve(event, modifiers) {
            return Some(action);
        }
    }

    if ime_composing {
        return None;
    }

    key_event_to_bytes(event, modifiers, mode, modify_other_keys).map(KeyAction::SendToTerminal)
}

pub fn key_event_to_bytes(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: TermMode,
    modify_other_keys: ModifyOtherKeys,
) -> Option<Vec<u8>> {
    if event.state != ElementState::Pressed
        && !(mode.contains(TermMode::REPORT_EVENT_TYPES)
            && mode.contains(TermMode::KITTY_KEYBOARD_PROTOCOL))
    {
        return None;
    }
    if event.repeat && !mode.contains(TermMode::REPORT_EVENT_TYPES) {
        return None;
    }

    let ctrl = modifiers.control_key();
    let alt = modifiers.alt_key();
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    if matches!(&event.logical_key, Key::Dead(_)) {
        return None;
    }

    if mode.contains(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        if let Some(bytes) = kitty_key_to_bytes(event, modifiers, mode) {
            return Some(bytes);
        }
    }

    if modify_other_keys != ModifyOtherKeys::Reset {
        if let Some(bytes) = modify_other_keys_to_bytes(event, modifiers, modify_other_keys) {
            return Some(bytes);
        }
    }

    if let Some(bytes) = physical_key_to_bytes(event.physical_key, mode) {
        return Some(bytes);
    }

    match &event.logical_key {
        Key::Named(NamedKey::Enter) => event
            .text
            .as_ref()
            .map(|text| text.as_bytes().to_vec())
            .or(Some(vec![b'\r'])),
        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
        Key::Named(NamedKey::Tab) => {
            if modifiers.shift_key() {
                Some(b"\x1b[Z".to_vec())
            } else {
                Some(vec![b'\t'])
            }
        }
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        Key::Named(NamedKey::Space) => {
            if ctrl {
                Some(vec![0])
            } else if alt {
                Some(vec![0x1b, b' '])
            } else {
                Some(vec![b' '])
            }
        }
        Key::Named(NamedKey::ArrowUp) => Some(cursor_key("\x1b[A", "\x1bOA", app_cursor)),
        Key::Named(NamedKey::ArrowDown) => Some(cursor_key("\x1b[B", "\x1bOB", app_cursor)),
        Key::Named(NamedKey::ArrowRight) => Some(cursor_key("\x1b[C", "\x1bOC", app_cursor)),
        Key::Named(NamedKey::ArrowLeft) => Some(cursor_key("\x1b[D", "\x1bOD", app_cursor)),
        Key::Named(NamedKey::Home) => Some(cursor_key("\x1b[H", "\x1bOH", app_cursor)),
        Key::Named(NamedKey::End) => Some(cursor_key("\x1b[F", "\x1bOF", app_cursor)),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
        Key::Character(text) => {
            if ctrl {
                return ctrl_char(text);
            }
            if alt && text.len() == 1 {
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(text.as_bytes());
                return Some(bytes);
            }
            if let Some(typed) = event.text.as_ref().filter(|text| !text.is_empty()) {
                return Some(typed.as_bytes().to_vec());
            }
            Some(text.as_bytes().to_vec())
        }
        _ => {
            if let Some(text) = event.text.as_ref().filter(|text| !text.is_empty()) {
                if !ctrl && !alt {
                    return Some(text.as_bytes().to_vec());
                }
            }
            None
        }
    }
}

fn physical_key_to_bytes(physical: PhysicalKey, mode: TermMode) -> Option<Vec<u8>> {
    match physical {
        PhysicalKey::Code(KeyCode::NumpadEnter) => Some(b"\r".to_vec()),
        PhysicalKey::Code(KeyCode::NumpadAdd) => keypad_key(mode, b"\x1b[Ol", b"+"),
        PhysicalKey::Code(KeyCode::NumpadSubtract) => keypad_key(mode, b"\x1b[Om", b"-"),
        PhysicalKey::Code(KeyCode::NumpadMultiply) => keypad_key(mode, b"\x1b[Oj", b"*"),
        PhysicalKey::Code(KeyCode::NumpadDivide) => keypad_key(mode, b"\x1b[Oo", b"/"),
        PhysicalKey::Code(KeyCode::NumpadDecimal) => keypad_key(mode, b"\x1b[On", b"."),
        PhysicalKey::Code(KeyCode::Numpad0) => keypad_key(mode, b"\x1b[Op", b"0"),
        PhysicalKey::Code(KeyCode::Numpad1) => keypad_key(mode, b"\x1b[Oq", b"1"),
        PhysicalKey::Code(KeyCode::Numpad2) => keypad_key(mode, b"\x1b[Or", b"2"),
        PhysicalKey::Code(KeyCode::Numpad3) => keypad_key(mode, b"\x1b[Os", b"3"),
        PhysicalKey::Code(KeyCode::Numpad4) => keypad_key(mode, b"\x1b[Ot", b"4"),
        PhysicalKey::Code(KeyCode::Numpad5) => keypad_key(mode, b"\x1b[Ou", b"5"),
        PhysicalKey::Code(KeyCode::Numpad6) => keypad_key(mode, b"\x1b[Ov", b"6"),
        PhysicalKey::Code(KeyCode::Numpad7) => keypad_key(mode, b"\x1b[Ow", b"7"),
        PhysicalKey::Code(KeyCode::Numpad8) => keypad_key(mode, b"\x1b[Ox", b"8"),
        PhysicalKey::Code(KeyCode::Numpad9) => keypad_key(mode, b"\x1b[Oy", b"9"),
        _ => None,
    }
}

fn cursor_key(normal: &str, application: &str, app_cursor: bool) -> Vec<u8> {
    if app_cursor {
        application.as_bytes().to_vec()
    } else {
        normal.as_bytes().to_vec()
    }
}

fn keypad_key(mode: TermMode, app: &[u8], normal: &[u8]) -> Option<Vec<u8>> {
    if mode.contains(TermMode::APP_KEYPAD) {
        Some(app.to_vec())
    } else {
        Some(normal.to_vec())
    }
}

fn ctrl_char(text: &str) -> Option<Vec<u8>> {
    let ch = text.chars().next()?;
    let upper = ch.to_ascii_uppercase();
    if upper.is_ascii_alphabetic() {
        let code = upper as u8 - b'A' + 1;
        return Some(vec![code]);
    }
    if ch == '@' {
        return Some(vec![0]);
    }
    None
}

fn kitty_key_to_bytes(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let report_all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) || report_all;
    let report_events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let has_modifier = modifiers.shift_key()
        || modifiers.alt_key()
        || modifiers.control_key()
        || modifiers.super_key();

    let event_type = match event.state {
        ElementState::Pressed if event.repeat => 2,
        ElementState::Pressed => 1,
        ElementState::Released => 3,
    };

    if event_type != 1 && !report_events {
        return None;
    }

    let key = kitty_functional_key(event)
        .or_else(|| kitty_character_code(event, mode).map(KittyKey::u))
        .or_else(|| kitty_physical_key_code(event.physical_key).map(KittyKey::u))?;

    let text = if mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) {
        event
            .text
            .as_ref()
            .filter(|text| !text.is_empty())
            .map(|text| {
                text.chars()
                    .map(|ch| (ch as u32).to_string())
                    .collect::<Vec<_>>()
                    .join(":")
            })
    } else {
        None
    };

    let should_encode = report_all
        || text.is_some()
        || event_type != 1
        || key.is_functional
        || (disambiguate && has_modifier);

    if !should_encode {
        return None;
    }

    let modifiers_value = kitty_modifier_value(modifiers);
    let second_param = if report_events || modifiers_value != 1 {
        if report_events {
            format!(";{modifiers_value}:{event_type}")
        } else {
            format!(";{modifiers_value}")
        }
    } else {
        String::new()
    };

    let third_param = text.map(|text| format!(";{text}")).unwrap_or_default();
    if key.uses_short_csi_form() && second_param.is_empty() && third_param.is_empty() {
        Some(format!("\x1b[{}", key.suffix).into_bytes())
    } else {
        Some(format!("\x1b[{}{second_param}{third_param}{}", key.code, key.suffix).into_bytes())
    }
}

#[derive(Clone, Copy)]
struct KittyKey {
    code: u32,
    suffix: char,
    is_functional: bool,
}

impl KittyKey {
    fn u(code: u32) -> Self {
        Self {
            code,
            suffix: 'u',
            is_functional: false,
        }
    }

    fn functional(code: u32, suffix: char) -> Self {
        Self {
            code,
            suffix,
            is_functional: true,
        }
    }

    fn uses_short_csi_form(self) -> bool {
        self.is_functional && self.code == 1 && self.suffix != 'u' && self.suffix != '~'
    }
}

fn kitty_modifier_value(modifiers: ModifiersState) -> u8 {
    let mut value = 1;
    if modifiers.shift_key() {
        value += 1;
    }
    if modifiers.alt_key() {
        value += 2;
    }
    if modifiers.control_key() {
        value += 4;
    }
    if modifiers.super_key() {
        value += 8;
    }
    value
}

fn kitty_character_code(event: &KeyEvent, mode: TermMode) -> Option<u32> {
    match &event.logical_key {
        Key::Named(NamedKey::Space) => Some(' ' as u32),
        Key::Character(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_none() {
                Some(ch as u32)
            } else if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
                event.text.as_ref()?.chars().next().map(|ch| ch as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn kitty_functional_key(event: &KeyEvent) -> Option<KittyKey> {
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => Some(KittyKey::functional(27, 'u')),
        Key::Named(NamedKey::Enter) => Some(KittyKey::functional(13, 'u')),
        Key::Named(NamedKey::Tab) => Some(KittyKey::functional(9, 'u')),
        Key::Named(NamedKey::Backspace) => Some(KittyKey::functional(127, 'u')),
        Key::Named(NamedKey::Insert) => Some(KittyKey::functional(2, '~')),
        Key::Named(NamedKey::Delete) => Some(KittyKey::functional(3, '~')),
        Key::Named(NamedKey::ArrowLeft) => Some(KittyKey::functional(1, 'D')),
        Key::Named(NamedKey::ArrowRight) => Some(KittyKey::functional(1, 'C')),
        Key::Named(NamedKey::ArrowUp) => Some(KittyKey::functional(1, 'A')),
        Key::Named(NamedKey::ArrowDown) => Some(KittyKey::functional(1, 'B')),
        Key::Named(NamedKey::PageUp) => Some(KittyKey::functional(5, '~')),
        Key::Named(NamedKey::PageDown) => Some(KittyKey::functional(6, '~')),
        Key::Named(NamedKey::Home) => Some(KittyKey::functional(1, 'H')),
        Key::Named(NamedKey::End) => Some(KittyKey::functional(1, 'F')),
        Key::Named(NamedKey::CapsLock) => Some(KittyKey::functional(57358, 'u')),
        Key::Named(NamedKey::ScrollLock) => Some(KittyKey::functional(57359, 'u')),
        Key::Named(NamedKey::NumLock) => Some(KittyKey::functional(57360, 'u')),
        Key::Named(NamedKey::PrintScreen) => Some(KittyKey::functional(57361, 'u')),
        Key::Named(NamedKey::Pause) => Some(KittyKey::functional(57362, 'u')),
        Key::Named(NamedKey::ContextMenu) => Some(KittyKey::functional(57363, 'u')),
        Key::Named(NamedKey::F1) => Some(KittyKey::functional(1, 'P')),
        Key::Named(NamedKey::F2) => Some(KittyKey::functional(1, 'Q')),
        Key::Named(NamedKey::F3) => Some(KittyKey::functional(13, '~')),
        Key::Named(NamedKey::F4) => Some(KittyKey::functional(1, 'S')),
        Key::Named(NamedKey::F5) => Some(KittyKey::functional(15, '~')),
        Key::Named(NamedKey::F6) => Some(KittyKey::functional(17, '~')),
        Key::Named(NamedKey::F7) => Some(KittyKey::functional(18, '~')),
        Key::Named(NamedKey::F8) => Some(KittyKey::functional(19, '~')),
        Key::Named(NamedKey::F9) => Some(KittyKey::functional(20, '~')),
        Key::Named(NamedKey::F10) => Some(KittyKey::functional(21, '~')),
        Key::Named(NamedKey::F11) => Some(KittyKey::functional(23, '~')),
        Key::Named(NamedKey::F12) => Some(KittyKey::functional(24, '~')),
        Key::Named(NamedKey::F13) => Some(KittyKey::functional(57376, 'u')),
        Key::Named(NamedKey::F14) => Some(KittyKey::functional(57377, 'u')),
        Key::Named(NamedKey::F15) => Some(KittyKey::functional(57378, 'u')),
        Key::Named(NamedKey::F16) => Some(KittyKey::functional(57379, 'u')),
        Key::Named(NamedKey::F17) => Some(KittyKey::functional(57380, 'u')),
        Key::Named(NamedKey::F18) => Some(KittyKey::functional(57381, 'u')),
        Key::Named(NamedKey::F19) => Some(KittyKey::functional(57382, 'u')),
        Key::Named(NamedKey::F20) => Some(KittyKey::functional(57383, 'u')),
        Key::Named(NamedKey::F21) => Some(KittyKey::functional(57384, 'u')),
        Key::Named(NamedKey::F22) => Some(KittyKey::functional(57385, 'u')),
        Key::Named(NamedKey::F23) => Some(KittyKey::functional(57386, 'u')),
        Key::Named(NamedKey::F24) => Some(KittyKey::functional(57387, 'u')),
        Key::Named(NamedKey::F25) => Some(KittyKey::functional(57388, 'u')),
        Key::Named(NamedKey::F26) => Some(KittyKey::functional(57389, 'u')),
        Key::Named(NamedKey::F27) => Some(KittyKey::functional(57390, 'u')),
        Key::Named(NamedKey::F28) => Some(KittyKey::functional(57391, 'u')),
        Key::Named(NamedKey::F29) => Some(KittyKey::functional(57392, 'u')),
        Key::Named(NamedKey::F30) => Some(KittyKey::functional(57393, 'u')),
        Key::Named(NamedKey::F31) => Some(KittyKey::functional(57394, 'u')),
        Key::Named(NamedKey::F32) => Some(KittyKey::functional(57395, 'u')),
        Key::Named(NamedKey::F33) => Some(KittyKey::functional(57396, 'u')),
        Key::Named(NamedKey::F34) => Some(KittyKey::functional(57397, 'u')),
        Key::Named(NamedKey::F35) => Some(KittyKey::functional(57398, 'u')),
        _ => None,
    }
}

fn kitty_physical_key_code(physical: PhysicalKey) -> Option<u32> {
    match physical {
        PhysicalKey::Code(KeyCode::Numpad0) => Some(57399),
        PhysicalKey::Code(KeyCode::Numpad1) => Some(57400),
        PhysicalKey::Code(KeyCode::Numpad2) => Some(57401),
        PhysicalKey::Code(KeyCode::Numpad3) => Some(57402),
        PhysicalKey::Code(KeyCode::Numpad4) => Some(57403),
        PhysicalKey::Code(KeyCode::Numpad5) => Some(57404),
        PhysicalKey::Code(KeyCode::Numpad6) => Some(57405),
        PhysicalKey::Code(KeyCode::Numpad7) => Some(57406),
        PhysicalKey::Code(KeyCode::Numpad8) => Some(57407),
        PhysicalKey::Code(KeyCode::Numpad9) => Some(57408),
        PhysicalKey::Code(KeyCode::NumpadDecimal) => Some(57409),
        PhysicalKey::Code(KeyCode::NumpadDivide) => Some(57410),
        PhysicalKey::Code(KeyCode::NumpadMultiply) => Some(57411),
        PhysicalKey::Code(KeyCode::NumpadSubtract) => Some(57412),
        PhysicalKey::Code(KeyCode::NumpadAdd) => Some(57413),
        PhysicalKey::Code(KeyCode::NumpadEnter) => Some(57414),
        PhysicalKey::Code(KeyCode::NumpadEqual) => Some(57415),
        _ => None,
    }
}

fn modify_other_keys_to_bytes(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: ModifyOtherKeys,
) -> Option<Vec<u8>> {
    let mod_val = modify_other_modifier_value(modifiers);
    if mod_val == 1 {
        return None;
    }

    let keycode = modify_other_keycode(event)?;

    if mode == ModifyOtherKeys::EnableExceptWellDefined {
        if modifiers.control_key() && !modifiers.alt_key() {
            if well_defined_ctrl_byte(event, modifiers).is_some() {
                return None;
            }
        }
        if modifiers.shift_key() && !modifiers.control_key() && !modifiers.alt_key() {
            return None;
        }
    }

    Some(format!("\x1b[{keycode};{mod_val}u").into_bytes())
}

fn modify_other_keycode(event: &KeyEvent) -> Option<u32> {
    match &event.logical_key {
        Key::Named(NamedKey::Enter) => Some(13),
        Key::Named(NamedKey::Tab) => Some(9),
        Key::Named(NamedKey::Backspace) => Some(127),
        Key::Named(NamedKey::Escape) => Some(27),
        Key::Named(NamedKey::Space) => Some(32),
        Key::Character(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(ch as u32)
        }
        _ => None,
    }
}

fn well_defined_ctrl_byte(event: &KeyEvent, modifiers: ModifiersState) -> Option<u8> {
    match &event.logical_key {
        Key::Character(text) => {
            let ch = text.chars().next()?;
            if ch.is_ascii_alphabetic() {
                let upper = ch.to_ascii_uppercase();
                return Some(upper as u8 - b'A' + 1);
            }
            match ch {
                '@' => Some(0),
                '[' => Some(27),
                '\\' => Some(28),
                ']' => Some(29),
                '^' => Some(30),
                '_' => Some(31),
                ' ' => Some(0),
                _ => None,
            }
        }
        Key::Named(NamedKey::Space) if modifiers.control_key() => Some(0),
        _ => None,
    }
}

fn modify_other_modifier_value(modifiers: ModifiersState) -> u8 {
    let mut value = 1;
    if modifiers.shift_key() {
        value += 1;
    }
    if modifiers.alt_key() {
        value += 2;
    }
    if modifiers.control_key() {
        value += 4;
    }
    if modifiers.super_key() {
        value += 8;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_value_encodes_ctrl_shift() {
        assert_eq!(modify_other_modifier_value(ModifiersState::CONTROL), 5);
        assert_eq!(modify_other_modifier_value(ModifiersState::SHIFT), 2);
    }
}

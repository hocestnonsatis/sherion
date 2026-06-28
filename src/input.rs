use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Debug, Clone)]
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
}

pub fn key_event_to_action(event: &KeyEvent, modifiers: ModifiersState) -> Option<KeyAction> {
    if event.repeat || event.state != ElementState::Pressed {
        return None;
    }

    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();

    if ctrl && shift {
        return match &event.logical_key {
            Key::Character(text) if text.eq_ignore_ascii_case("t") => Some(KeyAction::NewTab),
            Key::Character(text) if text.eq_ignore_ascii_case("d") => Some(KeyAction::DuplicateTab),
            Key::Character(text) if text.eq_ignore_ascii_case("w") => Some(KeyAction::CloseTab),
            Key::Character(text) if text.eq_ignore_ascii_case("n") => Some(KeyAction::DetachTab),
            Key::Character(text) if text.eq_ignore_ascii_case("c") => Some(KeyAction::Copy),
            Key::Character(text) if text.eq_ignore_ascii_case("v") => Some(KeyAction::Paste),
            Key::Character(text) if text.eq_ignore_ascii_case("k") => {
                Some(KeyAction::ClearScrollback)
            }
            Key::Named(NamedKey::Tab) => Some(KeyAction::PrevTab),
            Key::Named(NamedKey::Insert) => Some(KeyAction::Copy),
            _ => None,
        };
    }

    if shift && !ctrl {
        if let Key::Named(NamedKey::Insert) = &event.logical_key {
            return Some(KeyAction::Paste);
        }
    }

    if let Key::Named(NamedKey::F2) = &event.logical_key {
        return Some(KeyAction::RenameTab);
    }

    if ctrl {
        if let Key::Named(NamedKey::Tab) = &event.logical_key {
            return Some(KeyAction::NextTab);
        }
        if let Key::Character(text) = &event.logical_key {
            if text == "+" || text == "=" {
                return Some(KeyAction::ZoomIn);
            }
            if text == "-" {
                return Some(KeyAction::ZoomOut);
            }
            if text == "0" {
                return Some(KeyAction::ZoomReset);
            }
            if let Some(digit) = text.chars().next().filter(|ch| ch.is_ascii_digit()) {
                let number = digit.to_digit(10)? as usize;
                if number > 0 {
                    return Some(KeyAction::SelectTab(number));
                }
            }
        }
    }

    key_event_to_bytes(event, modifiers).map(KeyAction::SendToTerminal)
}

pub fn key_event_to_bytes(event: &KeyEvent, modifiers: ModifiersState) -> Option<Vec<u8>> {
    if event.repeat || event.state != ElementState::Pressed {
        return None;
    }

    let ctrl = modifiers.control_key();
    let alt = modifiers.alt_key();

    match &event.logical_key {
        Key::Named(NamedKey::Enter) => Some(vec![b'\r']),
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
        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
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
            Some(text.as_bytes().to_vec())
        }
        _ => None,
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

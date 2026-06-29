use std::str::FromStr;

use serde::{Deserialize, Serialize};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::input::KeyAction;

/// Parsed key chord: modifiers + logical key identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: ChordKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChordKey {
    Character(String),
    Named(NamedKey),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeybindingsConfig {
    #[serde(default = "default_new_tab")]
    pub new_tab: String,
    #[serde(default = "default_duplicate_tab")]
    pub duplicate_tab: String,
    #[serde(default = "default_close_tab")]
    pub close_tab: String,
    #[serde(default = "default_detach_tab")]
    pub detach_tab: String,
    #[serde(default = "default_next_tab")]
    pub next_tab: String,
    #[serde(default = "default_prev_tab")]
    pub prev_tab: String,
    #[serde(default = "default_rename_tab")]
    pub rename_tab: String,
    #[serde(default = "default_copy")]
    pub copy: String,
    #[serde(default = "default_copy_insert")]
    pub copy_insert: String,
    #[serde(default = "default_paste")]
    pub paste: String,
    #[serde(default = "default_paste_insert")]
    pub paste_insert: String,
    #[serde(default = "default_clear_scrollback")]
    pub clear_scrollback: String,
    #[serde(default = "default_zoom_in")]
    pub zoom_in: String,
    #[serde(default = "default_zoom_out")]
    pub zoom_out: String,
    #[serde(default = "default_zoom_reset")]
    pub zoom_reset: String,
    #[serde(default = "default_open_search")]
    pub open_search: String,
    #[serde(default = "default_open_palette")]
    pub open_palette: String,
    #[serde(default = "default_select_tab_1")]
    pub select_tab_1: String,
    #[serde(default = "default_select_tab_2")]
    pub select_tab_2: String,
    #[serde(default = "default_select_tab_3")]
    pub select_tab_3: String,
    #[serde(default = "default_select_tab_4")]
    pub select_tab_4: String,
    #[serde(default = "default_select_tab_5")]
    pub select_tab_5: String,
    #[serde(default = "default_select_tab_6")]
    pub select_tab_6: String,
    #[serde(default = "default_select_tab_7")]
    pub select_tab_7: String,
    #[serde(default = "default_select_tab_8")]
    pub select_tab_8: String,
    #[serde(default = "default_select_tab_9")]
    pub select_tab_9: String,
    #[serde(default = "default_split_vertical")]
    pub split_vertical: String,
    #[serde(default = "default_split_horizontal")]
    pub split_horizontal: String,
    #[serde(default = "default_close_pane")]
    pub close_pane: String,
    #[serde(default = "default_focus_pane_left")]
    pub focus_pane_left: String,
    #[serde(default = "default_focus_pane_right")]
    pub focus_pane_right: String,
    #[serde(default = "default_focus_pane_up")]
    pub focus_pane_up: String,
    #[serde(default = "default_focus_pane_down")]
    pub focus_pane_down: String,
    #[serde(default = "default_scroll_page_up")]
    pub scroll_page_up: String,
    #[serde(default = "default_scroll_page_down")]
    pub scroll_page_down: String,
    #[serde(default = "default_jump_prompt_prev")]
    pub jump_prompt_prev: String,
    #[serde(default = "default_jump_prompt_next")]
    pub jump_prompt_next: String,
    #[serde(default = "default_toggle_fullscreen")]
    pub toggle_fullscreen: String,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            new_tab: default_new_tab(),
            duplicate_tab: default_duplicate_tab(),
            close_tab: default_close_tab(),
            detach_tab: default_detach_tab(),
            next_tab: default_next_tab(),
            prev_tab: default_prev_tab(),
            rename_tab: default_rename_tab(),
            copy: default_copy(),
            copy_insert: default_copy_insert(),
            paste: default_paste(),
            paste_insert: default_paste_insert(),
            clear_scrollback: default_clear_scrollback(),
            zoom_in: default_zoom_in(),
            zoom_out: default_zoom_out(),
            zoom_reset: default_zoom_reset(),
            open_search: default_open_search(),
            open_palette: default_open_palette(),
            select_tab_1: default_select_tab_1(),
            select_tab_2: default_select_tab_2(),
            select_tab_3: default_select_tab_3(),
            select_tab_4: default_select_tab_4(),
            select_tab_5: default_select_tab_5(),
            select_tab_6: default_select_tab_6(),
            select_tab_7: default_select_tab_7(),
            select_tab_8: default_select_tab_8(),
            select_tab_9: default_select_tab_9(),
            split_vertical: default_split_vertical(),
            split_horizontal: default_split_horizontal(),
            close_pane: default_close_pane(),
            focus_pane_left: default_focus_pane_left(),
            focus_pane_right: default_focus_pane_right(),
            focus_pane_up: default_focus_pane_up(),
            focus_pane_down: default_focus_pane_down(),
            scroll_page_up: default_scroll_page_up(),
            scroll_page_down: default_scroll_page_down(),
            jump_prompt_prev: default_jump_prompt_prev(),
            jump_prompt_next: default_jump_prompt_next(),
            toggle_fullscreen: default_toggle_fullscreen(),
        }
    }
}

impl KeybindingsConfig {
    pub fn resolve(&self, event: &KeyEvent, modifiers: ModifiersState) -> Option<KeyAction> {
        let chord = event_to_chord(event, modifiers);
        for (binding, action) in self.bindings() {
            if binding == chord {
                return Some(action);
            }
        }
        None
    }

    fn bindings(&self) -> Vec<(KeyChord, KeyAction)> {
        let mut out = Vec::new();
        let pairs: [(&str, KeyAction); 30] = [
            (&self.new_tab, KeyAction::NewTab),
            (&self.duplicate_tab, KeyAction::DuplicateTab),
            (&self.close_tab, KeyAction::CloseTab),
            (&self.detach_tab, KeyAction::DetachTab),
            (&self.next_tab, KeyAction::NextTab),
            (&self.prev_tab, KeyAction::PrevTab),
            (&self.rename_tab, KeyAction::RenameTab),
            (&self.copy, KeyAction::Copy),
            (&self.copy_insert, KeyAction::Copy),
            (&self.paste, KeyAction::Paste),
            (&self.paste_insert, KeyAction::Paste),
            (&self.clear_scrollback, KeyAction::ClearScrollback),
            (&self.zoom_in, KeyAction::ZoomIn),
            (&self.zoom_out, KeyAction::ZoomOut),
            (&self.zoom_reset, KeyAction::ZoomReset),
            (&self.open_search, KeyAction::OpenSearch),
            (&self.open_palette, KeyAction::OpenCommandPalette),
            (&self.select_tab_1, KeyAction::SelectTab(1)),
            (&self.select_tab_2, KeyAction::SelectTab(2)),
            (&self.select_tab_3, KeyAction::SelectTab(3)),
            (&self.select_tab_4, KeyAction::SelectTab(4)),
            (&self.select_tab_5, KeyAction::SelectTab(5)),
            (&self.select_tab_6, KeyAction::SelectTab(6)),
            (&self.select_tab_7, KeyAction::SelectTab(7)),
            (&self.select_tab_8, KeyAction::SelectTab(8)),
            (&self.select_tab_9, KeyAction::SelectTab(9)),
            (&self.split_vertical, KeyAction::SplitVertical),
            (&self.split_horizontal, KeyAction::SplitHorizontal),
            (&self.close_pane, KeyAction::ClosePane),
            (&self.focus_pane_left, KeyAction::FocusPaneLeft),
        ];
        for (s, action) in pairs {
            if let Ok(chord) = KeyChord::from_str(s) {
                out.push((chord, action));
            }
        }
        let extra: [(&str, KeyAction); 5] = [
            (&self.focus_pane_right, KeyAction::FocusPaneRight),
            (&self.focus_pane_up, KeyAction::FocusPaneUp),
            (&self.focus_pane_down, KeyAction::FocusPaneDown),
            (&self.scroll_page_up, KeyAction::ScrollPageUp),
            (&self.scroll_page_down, KeyAction::ScrollPageDown),
        ];
        for (s, action) in extra {
            if let Ok(chord) = KeyChord::from_str(s) {
                out.push((chord, action));
            }
        }
        let jump: [(&str, KeyAction); 2] = [
            (&self.jump_prompt_prev, KeyAction::JumpPromptPrev),
            (&self.jump_prompt_next, KeyAction::JumpPromptNext),
        ];
        for (s, action) in jump {
            if let Ok(chord) = KeyChord::from_str(s) {
                out.push((chord, action));
            }
        }
        if let Ok(chord) = KeyChord::from_str(&self.toggle_fullscreen) {
            out.push((chord, KeyAction::ToggleFullscreen));
        }
        out
    }

    pub fn chord_label_for_action(&self, action: KeyAction) -> Option<String> {
        for (chord, bound) in self.bindings() {
            if bound == action {
                return Some(chord.display_label());
            }
        }
        None
    }
}

impl FromStr for KeyChord {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key_part: Option<String> = None;

        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                _ => {
                    if key_part.is_some() {
                        return Err(());
                    }
                    key_part = Some(part.to_ascii_lowercase());
                }
            }
        }

        let key_str = key_part.ok_or(())?;
        let key = parse_chord_key(&key_str)?;
        Ok(Self {
            ctrl,
            shift,
            alt,
            key,
        })
    }
}

impl KeyChord {
    pub fn display_label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        let key_label = self.key.display_label();
        parts.push(key_label.as_str());
        parts.join("+")
    }
}

impl ChordKey {
    fn display_label(&self) -> String {
        match self {
            ChordKey::Character(s) => {
                if s.len() == 1 {
                    s.to_uppercase()
                } else {
                    s.clone()
                }
            }
            ChordKey::Named(k) => named_key_label(*k),
        }
    }
}

fn named_key_label(key: NamedKey) -> String {
    match key {
        NamedKey::Tab => "Tab".to_owned(),
        NamedKey::Insert => "Insert".to_owned(),
        NamedKey::Escape => "Esc".to_owned(),
        NamedKey::Enter => "Enter".to_owned(),
        NamedKey::Backspace => "Backspace".to_owned(),
        NamedKey::Delete => "Delete".to_owned(),
        NamedKey::Space => "Space".to_owned(),
        NamedKey::PageUp => "PageUp".to_owned(),
        NamedKey::PageDown => "PageDown".to_owned(),
        NamedKey::Home => "Home".to_owned(),
        NamedKey::End => "End".to_owned(),
        NamedKey::ArrowUp => "Up".to_owned(),
        NamedKey::ArrowDown => "Down".to_owned(),
        NamedKey::ArrowLeft => "Left".to_owned(),
        NamedKey::ArrowRight => "Right".to_owned(),
        NamedKey::F1 => "F1".to_owned(),
        NamedKey::F2 => "F2".to_owned(),
        NamedKey::F3 => "F3".to_owned(),
        NamedKey::F4 => "F4".to_owned(),
        NamedKey::F5 => "F5".to_owned(),
        NamedKey::F6 => "F6".to_owned(),
        NamedKey::F7 => "F7".to_owned(),
        NamedKey::F8 => "F8".to_owned(),
        NamedKey::F9 => "F9".to_owned(),
        NamedKey::F10 => "F10".to_owned(),
        NamedKey::F11 => "F11".to_owned(),
        NamedKey::F12 => "F12".to_owned(),
        _ => format!("{key:?}"),
    }
}

fn parse_chord_key(s: &str) -> Result<ChordKey, ()> {
    match s {
        "tab" => Ok(ChordKey::Named(NamedKey::Tab)),
        "insert" => Ok(ChordKey::Named(NamedKey::Insert)),
        "escape" | "esc" => Ok(ChordKey::Named(NamedKey::Escape)),
        "enter" | "return" => Ok(ChordKey::Named(NamedKey::Enter)),
        "backspace" => Ok(ChordKey::Named(NamedKey::Backspace)),
        "delete" => Ok(ChordKey::Named(NamedKey::Delete)),
        "space" => Ok(ChordKey::Named(NamedKey::Space)),
        "pageup" => Ok(ChordKey::Named(NamedKey::PageUp)),
        "pagedown" => Ok(ChordKey::Named(NamedKey::PageDown)),
        "home" => Ok(ChordKey::Named(NamedKey::Home)),
        "end" => Ok(ChordKey::Named(NamedKey::End)),
        "arrowup" | "up" => Ok(ChordKey::Named(NamedKey::ArrowUp)),
        "arrowdown" | "down" => Ok(ChordKey::Named(NamedKey::ArrowDown)),
        "arrowleft" | "left" => Ok(ChordKey::Named(NamedKey::ArrowLeft)),
        "arrowright" | "right" => Ok(ChordKey::Named(NamedKey::ArrowRight)),
        "f1" => Ok(ChordKey::Named(NamedKey::F1)),
        "f2" => Ok(ChordKey::Named(NamedKey::F2)),
        "f3" => Ok(ChordKey::Named(NamedKey::F3)),
        "f4" => Ok(ChordKey::Named(NamedKey::F4)),
        "f5" => Ok(ChordKey::Named(NamedKey::F5)),
        "f6" => Ok(ChordKey::Named(NamedKey::F6)),
        "f7" => Ok(ChordKey::Named(NamedKey::F7)),
        "f8" => Ok(ChordKey::Named(NamedKey::F8)),
        "f9" => Ok(ChordKey::Named(NamedKey::F9)),
        "f10" => Ok(ChordKey::Named(NamedKey::F10)),
        "f11" => Ok(ChordKey::Named(NamedKey::F11)),
        "f12" => Ok(ChordKey::Named(NamedKey::F12)),
        other if other.len() == 1 => Ok(ChordKey::Character(other.to_owned())),
        other if other == "=" || other == "+" || other == "-" || other == "0" => {
            Ok(ChordKey::Character(other.to_owned()))
        }
        _ => Err(()),
    }
}

pub fn event_to_chord(event: &KeyEvent, modifiers: ModifiersState) -> KeyChord {
    let key = match &event.logical_key {
        Key::Character(text) => ChordKey::Character(text.to_ascii_lowercase()),
        Key::Named(named) => ChordKey::Named(*named),
        _ => ChordKey::Character(String::new()),
    };
    KeyChord {
        ctrl: modifiers.control_key(),
        shift: modifiers.shift_key(),
        alt: modifiers.alt_key(),
        key,
    }
}

fn default_new_tab() -> String {
    "ctrl+shift+t".to_owned()
}
fn default_duplicate_tab() -> String {
    "ctrl+shift+u".to_owned()
}
fn default_close_tab() -> String {
    "ctrl+shift+w".to_owned()
}
fn default_detach_tab() -> String {
    "ctrl+shift+n".to_owned()
}
fn default_next_tab() -> String {
    "ctrl+tab".to_owned()
}
fn default_prev_tab() -> String {
    "ctrl+shift+tab".to_owned()
}
fn default_rename_tab() -> String {
    "f2".to_owned()
}
fn default_copy() -> String {
    "ctrl+shift+c".to_owned()
}
fn default_copy_insert() -> String {
    "ctrl+shift+insert".to_owned()
}
fn default_paste() -> String {
    "ctrl+shift+v".to_owned()
}
fn default_paste_insert() -> String {
    "shift+insert".to_owned()
}
fn default_clear_scrollback() -> String {
    "ctrl+shift+k".to_owned()
}
fn default_zoom_in() -> String {
    "ctrl+=".to_owned()
}
fn default_zoom_out() -> String {
    "ctrl+-".to_owned()
}
fn default_zoom_reset() -> String {
    "ctrl+0".to_owned()
}
fn default_open_search() -> String {
    "ctrl+shift+f".to_owned()
}
fn default_open_palette() -> String {
    "ctrl+shift+p".to_owned()
}
fn default_select_tab_1() -> String {
    "ctrl+1".to_owned()
}
fn default_select_tab_2() -> String {
    "ctrl+2".to_owned()
}
fn default_select_tab_3() -> String {
    "ctrl+3".to_owned()
}
fn default_select_tab_4() -> String {
    "ctrl+4".to_owned()
}
fn default_select_tab_5() -> String {
    "ctrl+5".to_owned()
}
fn default_select_tab_6() -> String {
    "ctrl+6".to_owned()
}
fn default_select_tab_7() -> String {
    "ctrl+7".to_owned()
}
fn default_select_tab_8() -> String {
    "ctrl+8".to_owned()
}
fn default_select_tab_9() -> String {
    "ctrl+9".to_owned()
}
fn default_split_vertical() -> String {
    "ctrl+shift+d".to_owned()
}
fn default_split_horizontal() -> String {
    "ctrl+shift+e".to_owned()
}
fn default_close_pane() -> String {
    "ctrl+shift+q".to_owned()
}
fn default_focus_pane_left() -> String {
    "ctrl+shift+left".to_owned()
}
fn default_focus_pane_right() -> String {
    "ctrl+shift+right".to_owned()
}
fn default_focus_pane_up() -> String {
    "ctrl+shift+up".to_owned()
}
fn default_focus_pane_down() -> String {
    "ctrl+shift+down".to_owned()
}
fn default_scroll_page_up() -> String {
    "shift+pageup".to_owned()
}
fn default_scroll_page_down() -> String {
    "shift+pagedown".to_owned()
}
fn default_jump_prompt_prev() -> String {
    "alt+shift+up".to_owned()
}
fn default_jump_prompt_next() -> String {
    "alt+shift+down".to_owned()
}
fn default_toggle_fullscreen() -> String {
    "f11".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chord() {
        let c = KeyChord::from_str("ctrl+shift+t").unwrap();
        assert!(c.ctrl && c.shift && !c.alt);
        assert_eq!(c.key, ChordKey::Character("t".to_owned()));
    }
}

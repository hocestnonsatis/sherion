use std::path::PathBuf;

use vte::ansi::ModifyOtherKeys;

use crate::osc::parse_osc7_cwd;

/// Side effects observed while PTY output passes through to the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapEvent {
    Cwd(PathBuf),
    ModifyOtherKeys(ModifyOtherKeys),
    QueryModifyOtherKeys,
}

/// Incremental scanner for OSC 7 and modifyOtherKeys control sequences in PTY output.
#[derive(Default)]
pub struct PtyOutputTap {
    state: TapState,
    osc_payload: Vec<u8>,
    csi_params: String,
    csi_intermediate: String,
}

#[derive(Default)]
enum TapState {
    #[default]
    Normal,
    Esc,
    Osc,
    OscEsc,
    Csi,
}

impl PtyOutputTap {
    pub fn feed(&mut self, data: &[u8]) -> Vec<TapEvent> {
        let mut events = Vec::new();
        for &byte in data {
            if let Some(event) = self.feed_byte(byte) {
                events.push(event);
            }
        }
        events
    }

    fn feed_byte(&mut self, byte: u8) -> Option<TapEvent> {
        match self.state {
            TapState::Normal => {
                if byte == 0x1b {
                    self.state = TapState::Esc;
                }
                None
            }
            TapState::Esc => {
                self.state = match byte {
                    b']' => {
                        self.osc_payload.clear();
                        TapState::Osc
                    }
                    b'[' => {
                        self.csi_params.clear();
                        self.csi_intermediate.clear();
                        TapState::Csi
                    }
                    _ => TapState::Normal,
                };
                None
            }
            TapState::Osc => {
                if byte == 0x07 {
                    let event = self.finish_osc();
                    self.state = TapState::Normal;
                    return event;
                }
                if byte == 0x1b {
                    self.state = TapState::OscEsc;
                    return None;
                }
                self.osc_payload.push(byte);
                None
            }
            TapState::OscEsc => {
                if byte == b'\\' {
                    let event = self.finish_osc();
                    self.state = TapState::Normal;
                    return event;
                }
                self.osc_payload.push(0x1b);
                self.osc_payload.push(byte);
                self.state = TapState::Osc;
                None
            }
            TapState::Csi => {
                if byte.is_ascii_alphabetic() || byte == b'~' {
                    let event = self.finish_csi(byte);
                    self.state = TapState::Normal;
                    return event;
                }
                if byte.is_ascii_digit() || byte == b';' || byte == b':' {
                    self.csi_params.push(byte as char);
                } else if (0x20..=0x2f).contains(&byte) || (0x3c..=0x3f).contains(&byte) {
                    self.csi_intermediate.push(byte as char);
                } else {
                    self.state = TapState::Normal;
                }
                None
            }
        }
    }

    fn finish_osc(&mut self) -> Option<TapEvent> {
        let payload = validated_utf8(&self.osc_payload)?;
        parse_osc7_cwd(payload).map(TapEvent::Cwd)
    }

    fn finish_csi(&self, final_byte: u8) -> Option<TapEvent> {
        if final_byte == b'm' && self.csi_intermediate == ">" {
            return parse_modify_other_keys_set(&self.csi_params);
        }
        if final_byte == b'g' && self.csi_intermediate == "?" && self.csi_params == "4" {
            return Some(TapEvent::QueryModifyOtherKeys);
        }
        None
    }
}

fn validated_utf8(payload: &[u8]) -> Option<&str> {
    #[cfg(feature = "simd-utf8")]
    {
        return simdutf8::basic::from_utf8(payload).ok();
    }
    #[cfg(not(feature = "simd-utf8"))]
    {
        std::str::from_utf8(payload).ok()
    }
}

fn parse_modify_other_keys_set(params: &str) -> Option<TapEvent> {
    let mut parts = params.split(';');
    if parts.next()? != "4" {
        return None;
    }
    let mode = match parts.next() {
        None | Some("0") => ModifyOtherKeys::Reset,
        Some("1") => ModifyOtherKeys::EnableExceptWellDefined,
        Some("2") => ModifyOtherKeys::EnableAll,
        _ => return None,
    };
    Some(TapEvent::ModifyOtherKeys(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_modify_other_keys_enable_all() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b[>4;2m");
        assert_eq!(
            events,
            vec![TapEvent::ModifyOtherKeys(ModifyOtherKeys::EnableAll)]
        );
    }

    #[test]
    fn detects_modify_other_keys_reset() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b[>4;0m");
        assert_eq!(
            events,
            vec![TapEvent::ModifyOtherKeys(ModifyOtherKeys::Reset)]
        );
    }

    #[test]
    fn detects_modify_other_keys_query() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b[?4g");
        assert_eq!(events, vec![TapEvent::QueryModifyOtherKeys]);
    }

    #[test]
    fn detects_osc7_cwd() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b]7;file://localhost/tmp\x07");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], TapEvent::Cwd(path) if path == &PathBuf::from("/tmp")));
    }

    #[test]
    fn validated_utf8_accepts_osc_payload() {
        let payload = b"7;file://localhost/tmp";
        assert_eq!(validated_utf8(payload), Some("7;file://localhost/tmp"));
    }

    #[test]
    fn validated_utf8_rejects_invalid_bytes() {
        let payload = b"7;\xfffile://localhost/tmp";
        assert!(validated_utf8(payload).is_none());
    }

    #[test]
    fn detects_osc7_cwd_with_st_terminator() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b]7;file://localhost/home\x1b\\");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            TapEvent::Cwd(path) if path == &PathBuf::from("/home")
        ));
    }

    #[test]
    fn detects_osc7_cwd_across_split_feeds() {
        let mut tap = PtyOutputTap::default();
        let first = tap.feed(b"\x1b]7;file://localhost/");
        assert!(first.is_empty());
        let second = tap.feed(b"var/tmp\x07");
        assert_eq!(second.len(), 1);
        assert!(matches!(
            &second[0],
            TapEvent::Cwd(path) if path == &PathBuf::from("/var/tmp")
        ));
    }

    #[test]
    fn invalid_utf8_in_osc_payload_is_ignored() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b]7;file://\xfflocalhost/tmp\x07");
        assert!(events.is_empty());
    }

    #[test]
    fn osc7_and_modify_other_keys_in_same_feed() {
        let mut tap = PtyOutputTap::default();
        let events = tap.feed(b"\x1b]7;file://localhost/tmp\x07\x1b[>4;1m");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], TapEvent::Cwd(path) if path == &PathBuf::from("/tmp")));
        assert_eq!(
            events[1],
            TapEvent::ModifyOtherKeys(ModifyOtherKeys::EnableExceptWellDefined)
        );
    }
}

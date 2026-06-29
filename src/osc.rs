use std::path::PathBuf;

/// Incremental OSC scanner for PTY output; detects OSC 7 working-directory reports.
#[derive(Default)]
pub struct Osc7Tap {
    state: OscTapState,
    payload: Vec<u8>,
}

#[derive(Default)]
enum OscTapState {
    #[default]
    Normal,
    Esc,
    Osc,
    OscEsc,
}

impl Osc7Tap {
    /// Feed PTY bytes; returns the latest parsed CWD when an OSC 7 sequence completes.
    pub fn feed(&mut self, data: &[u8]) -> Option<PathBuf> {
        let mut latest = None;
        for &byte in data {
            if let Some(cwd) = self.feed_byte(byte) {
                latest = Some(cwd);
            }
        }
        latest
    }

    fn feed_byte(&mut self, byte: u8) -> Option<PathBuf> {
        match self.state {
            OscTapState::Normal => {
                if byte == 0x1b {
                    self.state = OscTapState::Esc;
                }
                None
            }
            OscTapState::Esc => {
                self.state = if byte == b']' {
                    self.payload.clear();
                    OscTapState::Osc
                } else {
                    OscTapState::Normal
                };
                None
            }
            OscTapState::Osc => {
                if byte == 0x07 {
                    let cwd = self.finish_osc();
                    self.state = OscTapState::Normal;
                    return cwd;
                }
                if byte == 0x1b {
                    self.state = OscTapState::OscEsc;
                    return None;
                }
                self.payload.push(byte);
                None
            }
            OscTapState::OscEsc => {
                if byte == b'\\' {
                    let cwd = self.finish_osc();
                    self.state = OscTapState::Normal;
                    return cwd;
                }
                self.payload.push(0x1b);
                self.payload.push(byte);
                self.state = OscTapState::Osc;
                None
            }
        }
    }

    fn finish_osc(&mut self) -> Option<PathBuf> {
        let payload = std::str::from_utf8(&self.payload).ok()?;
        parse_osc7_cwd(payload)
    }
}

/// Parse OSC 7 working-directory report: `\x1b]7;file://host/path\x07`.
pub fn parse_osc7_cwd(payload: &str) -> Option<PathBuf> {
    let payload = payload.strip_prefix("7;")?;
    let file_url = payload.strip_prefix("file://")?;
    let decoded = if file_url.starts_with('/') {
        url_decode_path(file_url)
    } else if let Some((_host, path)) = file_url.split_once('/') {
        format!("/{}", url_decode_path(path))
    } else {
        url_decode_path(file_url)
    };
    if decoded.is_empty() {
        return None;
    }
    Some(PathBuf::from(decoded))
}

fn url_decode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo) {
                if let Ok(byte) = u8::from_str_radix(&format!("{h}{l}"), 16) {
                    out.push(byte as char);
                    continue;
                }
            }
            out.push(ch);
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc7_file_url() {
        let cwd = parse_osc7_cwd("7;file://localhost/home/user/proj").unwrap();
        assert_eq!(cwd, PathBuf::from("/home/user/proj"));
    }

    #[test]
    fn tap_parses_osc7_from_stream() {
        let mut tap = Osc7Tap::default();
        let seq = b"\x1b]7;file://localhost/tmp/test\x07";
        let cwd = tap.feed(seq).unwrap();
        assert_eq!(cwd, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn tap_handles_st_terminator() {
        let mut tap = Osc7Tap::default();
        let seq = b"\x1b]7;file://host/home\x1b\\";
        let cwd = tap.feed(seq).unwrap();
        assert_eq!(cwd, PathBuf::from("/home"));
    }
}

use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{CursorStyle, NamedColor, Processor, Rgb, StdSyncHandler};

use crate::config::{parse_color, Config};

pub fn resolve_color_at_index<T: EventListener>(
    term: &Term<T>,
    config: &Config,
    index: usize,
) -> Rgb {
    if let Some(rgb) = term.colors()[index] {
        return rgb;
    }

    match index {
        i if i == NamedColor::Foreground as usize => config_rgb(&config.colors.foreground),
        i if i == NamedColor::Background as usize => config_rgb(&config.colors.background),
        i if i == NamedColor::Cursor as usize => config_rgb(&config.colors.cursor),
        i if i < 256 => indexed_color(i as u8),
        i if i == NamedColor::BrightForeground as usize => config_rgb(&config.colors.foreground),
        _ => indexed_color((index % 256) as u8),
    }
}

fn config_rgb(hex: &str) -> Rgb {
    rgb_from_hex(hex).unwrap_or(Rgb {
        r: 204,
        g: 204,
        b: 204,
    })
}

fn indexed_color(index: u8) -> Rgb {
    if index < 16 {
        named_fallback(match index {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        })
    } else if index < 232 {
        let index = index - 16;
        Rgb {
            r: (index / 36) * 51,
            g: ((index / 6) % 6) * 51,
            b: (index % 6) * 51,
        }
    } else {
        let gray = 8 + (index - 232) * 10;
        Rgb {
            r: gray,
            g: gray,
            b: gray,
        }
    }
}

fn named_fallback(named: NamedColor) -> Rgb {
    match named {
        NamedColor::Black => Rgb { r: 0, g: 0, b: 0 },
        NamedColor::Red => Rgb {
            r: 205,
            g: 49,
            b: 49,
        },
        NamedColor::Green => Rgb {
            r: 13,
            g: 188,
            b: 121,
        },
        NamedColor::Yellow => Rgb {
            r: 229,
            g: 229,
            b: 16,
        },
        NamedColor::Blue => Rgb {
            r: 36,
            g: 114,
            b: 200,
        },
        NamedColor::Magenta => Rgb {
            r: 188,
            g: 63,
            b: 188,
        },
        NamedColor::Cyan => Rgb {
            r: 17,
            g: 168,
            b: 205,
        },
        NamedColor::White => Rgb {
            r: 229,
            g: 229,
            b: 229,
        },
        NamedColor::BrightBlack => Rgb {
            r: 102,
            g: 102,
            b: 102,
        },
        NamedColor::BrightRed => Rgb {
            r: 241,
            g: 76,
            b: 76,
        },
        NamedColor::BrightGreen => Rgb {
            r: 35,
            g: 209,
            b: 139,
        },
        NamedColor::BrightYellow => Rgb {
            r: 245,
            g: 245,
            b: 67,
        },
        NamedColor::BrightBlue => Rgb {
            r: 59,
            g: 142,
            b: 234,
        },
        NamedColor::BrightMagenta => Rgb {
            r: 214,
            g: 112,
            b: 214,
        },
        NamedColor::BrightCyan => Rgb {
            r: 41,
            g: 184,
            b: 219,
        },
        NamedColor::BrightWhite => Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        _ => Rgb {
            r: 204,
            g: 204,
            b: 204,
        },
    }
}

pub fn apply_config_to_term<T: EventListener>(config: &Config, term: &mut Term<T>) {
    let mut seq = Vec::new();
    push_osc_dynamic(&mut seq, 10, &config.colors.foreground);
    push_osc_dynamic(&mut seq, 11, &config.colors.background);
    push_osc_dynamic(&mut seq, 12, &config.colors.cursor);

    if let Some(palette) = &config.colors.palette_16 {
        for (index, hex) in palette.iter().enumerate().take(16) {
            push_osc_indexed(&mut seq, index, hex);
        }
    }
    if let Some(palette) = &config.colors.palette_256 {
        for (index, hex) in palette.iter().enumerate().take(256) {
            push_osc_indexed(&mut seq, index, hex);
        }
    }

    if !seq.is_empty() {
        let mut processor = Processor::<StdSyncHandler>::new();
        processor.advance(term, &seq);
    }

    term.set_options(config.term_config());
}

fn push_osc_indexed(buf: &mut Vec<u8>, index: usize, hex: &str) {
    if let Some((r, g, b)) = rgb_bytes(hex) {
        buf.extend_from_slice(format!("\x1b]4;{index};rgb:{r}/{g}/{b}\x1b\\").as_bytes());
    }
}

fn push_osc_dynamic(buf: &mut Vec<u8>, code: u8, hex: &str) {
    if let Some((r, g, b)) = rgb_bytes(hex) {
        buf.extend_from_slice(format!("\x1b]{code};rgb:{r}/{g}/{b}\x1b\\").as_bytes());
    }
}

fn rgb_bytes(hex: &str) -> Option<(u8, u8, u8)> {
    let color = parse_color(hex).ok()?;
    let components = color.components;
    Some((
        (components[0] * 255.0).round() as u8,
        (components[1] * 255.0).round() as u8,
        (components[2] * 255.0).round() as u8,
    ))
}

pub fn cursor_style_from_config(config: &Config) -> CursorStyle {
    use alacritty_terminal::vte::ansi::CursorShape;

    let shape = match config.terminal.cursor_style.as_str() {
        "beam" | "steady_beam" => CursorShape::Beam,
        "underline" | "steady_underline" => CursorShape::Underline,
        "hollow" | "steady_hollow" => CursorShape::HollowBlock,
        "hidden" => CursorShape::Hidden,
        "block" | "steady_block" => CursorShape::Block,
        _ => CursorShape::Block,
    };
    let blinking = !matches!(
        config.terminal.cursor_style.as_str(),
        "steady_block" | "steady_beam" | "steady_underline" | "steady_hollow"
    );
    CursorStyle { shape, blinking }
}

pub fn rgb_from_hex(hex: &str) -> Option<Rgb> {
    rgb_bytes(hex).map(|(r, g, b)| Rgb { r, g, b })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::CursorShape;

    fn config_with_cursor(style: &str) -> Config {
        let mut config = Config::default();
        config.terminal.cursor_style = style.to_owned();
        config
    }

    #[test]
    fn cursor_style_maps_block_and_steady_variants() {
        let block = cursor_style_from_config(&config_with_cursor("block"));
        assert_eq!(block.shape, CursorShape::Block);
        assert!(block.blinking);

        let steady = cursor_style_from_config(&config_with_cursor("steady_block"));
        assert_eq!(steady.shape, CursorShape::Block);
        assert!(!steady.blinking);
    }

    #[test]
    fn cursor_style_maps_extended_decscusr_aliases() {
        for (style, shape) in [
            ("beam", CursorShape::Beam),
            ("steady_beam", CursorShape::Beam),
            ("underline", CursorShape::Underline),
            ("steady_underline", CursorShape::Underline),
            ("hollow", CursorShape::HollowBlock),
            ("steady_hollow", CursorShape::HollowBlock),
            ("hidden", CursorShape::Hidden),
        ] {
            let parsed = cursor_style_from_config(&config_with_cursor(style));
            assert_eq!(parsed.shape, shape, "style {style}");
        }
    }
}

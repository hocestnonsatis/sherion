use vello::peniko::color::{AlphaColor, Srgb};
use vello::peniko::Brush;

use crate::config::Config;

/// UI colors derived from the terminal palette so chrome and content feel cohesive.
#[derive(Clone, Copy, Debug)]
pub struct UiTheme {
    /// Terminal grid and active tab — matches `colors.background`.
    pub surface: AlphaColor<Srgb>,
    /// Title bar and tab strip — slightly darker than the surface.
    pub chrome: AlphaColor<Srgb>,
    /// Inactive tab pills on the chrome strip.
    pub tab_inactive: AlphaColor<Srgb>,
    /// Hover states for buttons and menu rows.
    pub elevated: AlphaColor<Srgb>,
    pub foreground: AlphaColor<Srgb>,
    pub accent: AlphaColor<Srgb>,
    pub muted: AlphaColor<Srgb>,
}

impl UiTheme {
    pub fn from_config(config: &Config) -> Self {
        let surface = config.background_brush();
        let chrome = darken(surface, 8);
        Self {
            surface,
            chrome,
            tab_inactive: lighten(chrome, 6),
            elevated: lighten(chrome, 14),
            foreground: config.foreground_brush(),
            accent: config.cursor_brush(),
            muted: lighten(chrome, 4),
        }
    }

    pub fn brush(&self, color: AlphaColor<Srgb>) -> Brush {
        Brush::Solid(color)
    }

    pub fn surface_brush(&self) -> Brush {
        self.brush(self.surface)
    }

    pub fn chrome_brush(&self) -> Brush {
        self.brush(self.chrome)
    }

    pub fn tab_inactive_brush(&self) -> Brush {
        self.brush(self.tab_inactive)
    }

    pub fn elevated_brush(&self) -> Brush {
        self.brush(self.elevated)
    }

    pub fn foreground_brush(&self) -> Brush {
        self.brush(self.foreground)
    }

    pub fn accent_brush(&self) -> Brush {
        self.brush(self.accent)
    }

    pub fn muted_brush(&self) -> Brush {
        self.brush(self.muted)
    }
}

fn rgb8(color: AlphaColor<Srgb>) -> (u8, u8, u8) {
    let rgba = color.to_rgba8();
    (rgba.r, rgba.g, rgba.b)
}

fn from_rgb8(r: u8, g: u8, b: u8) -> AlphaColor<Srgb> {
    AlphaColor::from_rgb8(r, g, b)
}

fn darken(color: AlphaColor<Srgb>, amount: u8) -> AlphaColor<Srgb> {
    let (r, g, b) = rgb8(color);
    let d = i16::from(amount);
    from_rgb8(
        (i16::from(r) - d).clamp(0, 255) as u8,
        (i16::from(g) - d).clamp(0, 255) as u8,
        (i16::from(b) - d).clamp(0, 255) as u8,
    )
}

fn lighten(color: AlphaColor<Srgb>, amount: u8) -> AlphaColor<Srgb> {
    let (r, g, b) = rgb8(color);
    let d = i16::from(amount);
    from_rgb8(
        (i16::from(r) + d).clamp(0, 255) as u8,
        (i16::from(g) + d).clamp(0, 255) as u8,
        (i16::from(b) + d).clamp(0, 255) as u8,
    )
}

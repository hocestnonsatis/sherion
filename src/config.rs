use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use vello::peniko::color::{AlphaColor, Srgb};
use winit::window::Theme;

const DEFAULT_CONFIG_PATH: &str = "sherion.toml";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub bell: BellConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub session: SessionConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewModeConfig {
    Single,
    Grid,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_theme_mode")]
    pub theme: ThemeMode,
    /// Terminal background opacity from 0.2 (very transparent) to 1.0 (opaque).
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    #[serde(default = "default_font_zoom")]
    pub font_zoom: f32,
    #[serde(default)]
    pub sidebar_width: f32,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default = "default_view_mode")]
    pub view_mode: ViewModeConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SessionConfig {
    #[serde(default = "default_restore_tabs")]
    pub restore_tabs: bool,
    #[serde(default)]
    pub cwd: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowConfig {
    #[serde(default = "default_title")]
    pub title: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FontConfig {
    #[serde(default = "default_font_family")]
    pub family: String,
    #[serde(default = "default_font_size")]
    pub size: f32,
    /// Ordered fallback families used when the primary font lacks a glyph
    /// (e.g. Nerd Font icons in `ls`/`exa` output). Missing fonts are skipped.
    #[serde(default = "default_font_fallback")]
    pub fallback: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ColorsConfig {
    #[serde(default = "default_foreground")]
    pub foreground: String,
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_cursor")]
    pub cursor: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerminalConfig {
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: usize,
    #[serde(default)]
    pub shell: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BellConfig {
    #[serde(default = "default_bell_visual")]
    pub visual: bool,
}

impl ColorsConfig {
    pub fn dark() -> Self {
        Self {
            foreground: default_foreground(),
            background: default_background(),
            cursor: default_cursor(),
        }
    }

    pub fn light() -> Self {
        Self {
            foreground: "#2e2e2e".to_owned(),
            background: "#f4f4f4".to_owned(),
            cursor: "#0057d9".to_owned(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: default_title(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: default_font_family(),
            size: default_font_size(),
            fallback: default_font_fallback(),
        }
    }
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            foreground: default_foreground(),
            background: default_background(),
            cursor: default_cursor(),
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: default_scrollback(),
            shell: String::new(),
        }
    }
}

impl Default for BellConfig {
    fn default() -> Self {
        Self {
            visual: default_bell_visual(),
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: default_theme_mode(),
            opacity: default_opacity(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            font_zoom: default_font_zoom(),
            sidebar_width: 0.0,
            sidebar_collapsed: false,
            view_mode: default_view_mode(),
        }
    }
}

fn default_title() -> String {
    "Sherion".to_owned()
}

fn default_font_family() -> String {
    "DejaVu Sans Mono".to_owned()
}

fn default_font_size() -> f32 {
    14.0
}

fn default_font_fallback() -> Vec<String> {
    vec![
        "Symbols Nerd Font Mono".to_owned(),
        "Symbols Nerd Font".to_owned(),
        "JetBrainsMono Nerd Font Mono".to_owned(),
        "JetBrainsMono Nerd Font".to_owned(),
        "MesloLGS Nerd Font".to_owned(),
        "Noto Sans Symbols".to_owned(),
        "Noto Sans Symbols 2".to_owned(),
        "Noto Color Emoji".to_owned(),
    ]
}

fn default_foreground() -> String {
    "#cccccc".to_owned()
}

fn default_background() -> String {
    "#1e1e1e".to_owned()
}

fn default_cursor() -> String {
    "#ffffff".to_owned()
}

fn default_scrollback() -> usize {
    10_000
}

fn default_bell_visual() -> bool {
    true
}

fn default_theme_mode() -> ThemeMode {
    ThemeMode::Auto
}

fn default_opacity() -> f32 {
    1.0
}

fn default_font_zoom() -> f32 {
    1.0
}

fn default_view_mode() -> ViewModeConfig {
    ViewModeConfig::Single
}

fn default_restore_tabs() -> bool {
    true
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            tracing::info!(path = %path.display(), "config file not found, using defaults");
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&path, contents)
            .with_context(|| format!("failed to write config to {}", path.display()))
    }

    pub fn term_config(&self) -> alacritty_terminal::term::Config {
        alacritty_terminal::term::Config {
            scrolling_history: self.terminal.scrollback_lines,
            ..Default::default()
        }
    }

    pub fn foreground_brush(&self) -> AlphaColor<Srgb> {
        parse_color(&self.colors.foreground)
            .unwrap_or_else(|_| AlphaColor::from_rgb8(0xcc, 0xcc, 0xcc))
    }

    pub fn background_brush(&self) -> AlphaColor<Srgb> {
        parse_color(&self.colors.background)
            .unwrap_or_else(|_| AlphaColor::from_rgb8(0x1e, 0x1e, 0x1e))
    }

    pub fn cursor_brush(&self) -> AlphaColor<Srgb> {
        parse_color(&self.colors.cursor).unwrap_or_else(|_| AlphaColor::from_rgb8(0xff, 0xff, 0xff))
    }

    pub fn apply_system_theme(&mut self, theme: Theme) {
        self.colors = match theme {
            Theme::Light => ColorsConfig::light(),
            Theme::Dark => ColorsConfig::dark(),
        };
    }
}

fn config_path() -> PathBuf {
    std::env::var_os("SHERION_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

pub fn parse_color(hex: &str) -> Result<AlphaColor<Srgb>> {
    let hex = hex.trim().trim_start_matches('#');
    anyhow::ensure!(hex.len() == 6, "expected 6-digit hex color");

    let r = u8::from_str_radix(&hex[0..2], 16).context("invalid red channel")?;
    let g = u8::from_str_radix(&hex[2..4], 16).context("invalid green channel")?;
    let b = u8::from_str_radix(&hex[4..6], 16).context("invalid blue channel")?;

    Ok(AlphaColor::from_rgb8(r, g, b))
}

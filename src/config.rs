use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use vello::peniko::color::{AlphaColor, Srgb};
use winit::window::Theme;

const DEFAULT_CONFIG_PATH: &str = "sherion.toml";
const XDG_CONFIG_DIR: &str = "sherion";
const XDG_CONFIG_FILE: &str = "sherion.toml";

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
    #[serde(default)]
    pub keybindings: crate::keybindings::KeybindingsConfig,
    /// Profile names discovered in the config file (not persisted on save).
    #[serde(skip)]
    pub profile_names: Vec<String>,
    /// Active profile name applied during load.
    #[serde(skip)]
    pub active_profile: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundMode {
    Cover,
    Contain,
    Tile,
    Center,
}

impl Default for BackgroundMode {
    fn default() -> Self {
        Self::Cover
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundShader {
    None,
    Vignette,
    Scanlines,
    Noise,
}

impl Default for BackgroundShader {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_theme_mode")]
    pub theme: ThemeMode,
    /// Terminal background opacity from 0.2 (very transparent) to 1.0 (opaque).
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Optional image drawn behind terminal panes.
    #[serde(default)]
    pub background_image: Option<PathBuf>,
    #[serde(default)]
    pub background_mode: BackgroundMode,
    /// Background image opacity from 0.0 to 1.0.
    #[serde(default = "default_background_opacity")]
    pub background_opacity: f32,
    /// Optional scene-based shader preset drawn over the background image.
    #[serde(default)]
    pub background_shader: BackgroundShader,
    /// Shader effect strength from 0.0 to 1.0.
    #[serde(default = "default_background_shader_intensity")]
    pub background_shader_intensity: f32,
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
    #[serde(default = "default_true")]
    pub follow_output: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SessionConfig {
    #[serde(default = "default_restore_tabs")]
    pub restore_tabs: bool,
    #[serde(default)]
    pub cwd: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowDecorations {
    Borderless,
    Native,
}

impl Default for WindowDecorations {
    fn default() -> Self {
        Self::Borderless
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowConfig {
    #[serde(default = "default_title")]
    pub title: String,
    /// Logical width in pixels; 0 uses the built-in default.
    #[serde(default)]
    pub width: f64,
    /// Logical height in pixels; 0 uses the built-in default.
    #[serde(default)]
    pub height: f64,
    /// Outer window position; omitted lets the window manager decide.
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub always_on_top: bool,
    /// `borderless` uses the custom title bar; `native` uses the OS window frame.
    #[serde(default)]
    pub decorations: WindowDecorations,
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
    /// Enable OpenType ligatures (`fi`, `-->`) when shaping adjacent cells.
    #[serde(default)]
    pub ligatures: bool,
    /// Rasterize glyphs via swash into GPU-friendly images instead of vector paths.
    #[serde(default)]
    pub glyph_atlas: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ColorsConfig {
    #[serde(default = "default_foreground")]
    pub foreground: String,
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_cursor")]
    pub cursor: String,
    #[serde(default)]
    pub palette_16: Option<Vec<String>>,
    #[serde(default)]
    pub palette_256: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerminalConfig {
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: usize,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub shell_args: Vec<String>,
    /// Remove C0 control characters from pasted text (except tab/newline).
    #[serde(default = "default_true")]
    pub sanitize_paste: bool,
    #[serde(default = "default_cursor_style")]
    pub cursor_style: String,
    /// Enable parsing of Kitty keyboard protocol enable/disable escape sequences.
    #[serde(default = "default_true")]
    pub kitty_keyboard: bool,
    /// OSC 52 clipboard integration: `disabled`, `copy`, `paste`, or `copypaste`.
    #[serde(default = "default_osc52")]
    pub osc52: Osc52Config,
    /// Non-Unix busy detection window in milliseconds.
    #[serde(default = "default_busy_heuristic_ms")]
    pub busy_heuristic_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Osc52Config {
    Disabled,
    Copy,
    Paste,
    CopyPaste,
}

fn default_osc52() -> Osc52Config {
    Osc52Config::CopyPaste
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BellConfig {
    #[serde(default = "default_bell_visual")]
    pub visual: bool,
    #[serde(default = "default_bell_audible")]
    pub audible: bool,
    #[serde(default)]
    pub urgency: bool,
}

impl ColorsConfig {
    pub fn dark() -> Self {
        Self {
            foreground: default_foreground(),
            background: default_background(),
            cursor: default_cursor(),
            palette_16: None,
            palette_256: None,
        }
    }

    pub fn light() -> Self {
        Self {
            foreground: "#2e2e2e".to_owned(),
            background: "#f4f4f4".to_owned(),
            cursor: "#0057d9".to_owned(),
            palette_16: None,
            palette_256: None,
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: default_title(),
            width: 0.0,
            height: 0.0,
            x: None,
            y: None,
            always_on_top: false,
            decorations: WindowDecorations::default(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: default_font_family(),
            size: default_font_size(),
            fallback: default_font_fallback(),
            ligatures: false,
            glyph_atlas: false,
        }
    }
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            foreground: default_foreground(),
            background: default_background(),
            cursor: default_cursor(),
            palette_16: None,
            palette_256: None,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: default_scrollback(),
            shell: String::new(),
            shell_args: Vec::new(),
            sanitize_paste: default_true(),
            cursor_style: default_cursor_style(),
            kitty_keyboard: default_true(),
            osc52: default_osc52(),
            busy_heuristic_ms: default_busy_heuristic_ms(),
        }
    }
}

impl Default for BellConfig {
    fn default() -> Self {
        Self {
            visual: default_bell_visual(),
            audible: default_bell_audible(),
            urgency: false,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: default_theme_mode(),
            opacity: default_opacity(),
            background_image: None,
            background_mode: BackgroundMode::default(),
            background_opacity: default_background_opacity(),
            background_shader: BackgroundShader::default(),
            background_shader_intensity: default_background_shader_intensity(),
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
            follow_output: true,
        }
    }
}

fn default_true() -> bool {
    true
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

fn default_bell_audible() -> bool {
    true
}

fn default_theme_mode() -> ThemeMode {
    ThemeMode::Auto
}

fn default_opacity() -> f32 {
    1.0
}

fn default_background_opacity() -> f32 {
    0.35
}

fn default_background_shader_intensity() -> f32 {
    0.5
}

fn default_busy_heuristic_ms() -> u64 {
    800
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

fn default_cursor_style() -> String {
    "block".to_owned()
}

pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SHERION_CONFIG") {
        return PathBuf::from(path);
    }

    let xdg = xdg_config_path();
    if xdg.exists() {
        return xdg;
    }

    let local = PathBuf::from(DEFAULT_CONFIG_PATH);
    if local.exists() {
        return local;
    }

    xdg
}

fn xdg_config_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home)
            .join(XDG_CONFIG_DIR)
            .join(XDG_CONFIG_FILE);
    }

    std::env::var_os("HOME")
        .map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join(XDG_CONFIG_DIR)
                .join(XDG_CONFIG_FILE)
        })
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

fn ensure_config_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config directory {}", parent.display()))?;
        }
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_profile(None)
    }

    pub fn load_profile(active_profile: Option<&str>) -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            tracing::info!(path = %path.display(), "config file not found, using defaults");
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        let mut root: toml::Value =
            toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;

        let profile_names = root
            .get("profiles")
            .and_then(|value| value.as_table())
            .map(|table| table.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let profile = active_profile
            .map(str::to_owned)
            .or_else(|| std::env::var("SHERION_PROFILE").ok());

        if let Some(name) = profile.as_deref() {
            let overlay = root
                .get("profiles")
                .and_then(|profiles| profiles.get(name))
                .cloned();
            if let Some(overlay) = overlay {
                merge_toml_values(&mut root, &overlay);
            } else {
                tracing::warn!(profile = name, "profile not found in config");
            }
        }

        if let Some(table) = root.as_table_mut() {
            table.remove("profiles");
        }

        let mut config: Config = root
            .try_into()
            .with_context(|| format!("failed to deserialize {}", path.display()))?;
        config.profile_names = profile_names;
        config.active_profile = profile;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        ensure_config_parent(&path)?;
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&path, contents)
            .with_context(|| format!("failed to write config to {}", path.display()))
    }

    pub fn term_config(&self) -> alacritty_terminal::term::Config {
        use alacritty_terminal::term::Osc52;

        let osc52 = match self.terminal.osc52 {
            Osc52Config::Disabled => Osc52::Disabled,
            Osc52Config::Copy => Osc52::OnlyCopy,
            Osc52Config::Paste => Osc52::OnlyPaste,
            Osc52Config::CopyPaste => Osc52::CopyPaste,
        };

        alacritty_terminal::term::Config {
            scrolling_history: self.terminal.scrollback_lines,
            default_cursor_style: crate::terminal_setup::cursor_style_from_config(self),
            kitty_keyboard: self.terminal.kitty_keyboard,
            osc52,
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

pub fn parse_color(hex: &str) -> Result<AlphaColor<Srgb>> {
    let hex = hex.trim().trim_start_matches('#');
    anyhow::ensure!(hex.len() == 6, "expected 6-digit hex color");

    let r = u8::from_str_radix(&hex[0..2], 16).context("invalid red channel")?;
    let g = u8::from_str_radix(&hex[2..4], 16).context("invalid green channel")?;
    let b = u8::from_str_radix(&hex[4..6], 16).context("invalid blue channel")?;

    Ok(AlphaColor::from_rgb8(r, g, b))
}

fn merge_toml_values(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if key == "profiles" {
                    continue;
                }
                match base.get_mut(key) {
                    Some(existing) => merge_toml_values(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_background_appearance_fields() {
        let raw = r#"
            [appearance]
            background_image = "/tmp/wall.png"
            background_mode = "tile"
            background_opacity = 0.5
            background_shader = "vignette"
            background_shader_intensity = 0.75
            [terminal]
            busy_heuristic_ms = 1200
        "#;
        let config: Config = toml::from_str(raw).unwrap();
        assert_eq!(
            config.appearance.background_image,
            Some(PathBuf::from("/tmp/wall.png"))
        );
        assert_eq!(config.appearance.background_mode, BackgroundMode::Tile);
        assert!((config.appearance.background_opacity - 0.5).abs() < f32::EPSILON);
        assert_eq!(config.appearance.background_shader, BackgroundShader::Vignette);
        assert!(
            (config.appearance.background_shader_intensity - 0.75).abs() < f32::EPSILON
        );
        assert_eq!(config.terminal.busy_heuristic_ms, 1200);
    }
}

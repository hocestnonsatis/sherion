use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use pollster::block_on;
use vello::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii, Stroke};
use vello::peniko::color::AlphaColor;
use vello::peniko::{Brush, Fill};
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::config::Config;
use crate::render::background::BackgroundImage;
use crate::render::background_shader::draw_background_shader;
use crate::render::chrome::ChromeMetrics;
use crate::render::frame::{FrameDamage, TerminalFrame};
use crate::render::layout::GRID_GUTTER;
use crate::render::menu::{MenuEntry, MenuLayout, MenuRenderer};
use crate::render::perf::{PerfStatsSnapshot, RenderTimings};
use crate::render::scene::SceneBuilder;
use crate::render::tab_bar::TabBarEntry;
use crate::render::tab_strip::{TabStripLayout, TabStripRenderer};
use crate::render::text::{draw_text, font_family_from_config};
use crate::render::theme::UiTheme;
use crate::render::title_bar::{TitleBarLayout, TitleBarRenderer};
use crate::render::TerminalLayout;

pub struct PaneRender<'a> {
    pub frame: &'a TerminalFrame,
    pub layout: TerminalLayout,
    pub damage: &'a FrameDamage,
    pub ime_preedit: Option<&'a str>,
}

pub struct ChromeFrame<'a> {
    pub metrics: ChromeMetrics,
    pub tab_entries: &'a [TabBarEntry],
    pub window_title: &'a str,
    pub menu_open: bool,
    pub menu_entries: &'a [MenuEntry],
    pub tab_scroll_offset: f64,
    pub window_height: f64,
    pub sidebar_collapsed: bool,
    pub bell_flash: bool,
    pub terminal_opacity: f32,
    pub chrome_dirty: bool,
    pub perf_overlay: Option<&'a PerfStatsSnapshot>,
    pub search_overlay: Option<SearchOverlayFrame<'a>>,
    pub rename_overlay: Option<RenameOverlayFrame<'a>>,
    pub command_palette: Option<CommandPaletteFrame<'a>>,
}

#[derive(Clone, Copy)]
pub struct SearchOverlayFrame<'a> {
    pub query: &'a str,
    pub match_count: usize,
    pub current_match: usize,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub whole_word: bool,
    pub regex_error: bool,
}

#[derive(Clone, Copy)]
pub struct RenameOverlayFrame<'a> {
    pub draft: &'a str,
}

#[derive(Clone, Copy)]
pub struct CommandPaletteFrame<'a> {
    pub query: &'a str,
    pub entries: &'a [String],
    pub selected_index: usize,
}

pub struct GpuRenderer {
    render_context: RenderContext,
    surface: Option<RenderSurface<'static>>,
    vello: Option<VelloRenderer>,
    scene: Scene,
    chrome_scene: Scene,
    chrome_scene_valid: bool,
    pane_terminal_scenes: Vec<Scene>,
    pane_scene_initialized: Vec<bool>,
    cached_theme: UiTheme,
    scene_builder: SceneBuilder,
    tab_strip_renderer: TabStripRenderer,
    title_bar_renderer: TitleBarRenderer,
    menu_renderer: MenuRenderer,
    font_cx: parley::FontContext,
    layout: TerminalLayout,
    tab_strip_layout: Option<TabStripLayout>,
    title_bar_layout: Option<TitleBarLayout>,
    menu_layout: Option<MenuLayout>,
    search_panel_bounds: Option<Rect>,
    search_close_bounds: Option<Rect>,
    config: Config,
    background: Option<BackgroundImage>,
}

/// Result of a mouse click while the search overlay is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOverlayHit {
    /// The close button was clicked.
    Close,
    /// A click landed inside the panel but not on the close button.
    Inside,
}

impl GpuRenderer {
    pub fn new(window: Arc<Window>, layout: TerminalLayout, config: Config) -> Result<Self> {
        let size = window.inner_size();
        let mut render_context = RenderContext::new();

        let mut surface = block_on(render_context.create_surface(
            Arc::clone(&window),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .context("failed to create wgpu surface")?;

        // Enable real window transparency. Vello's helper defaults to
        // `CompositeAlphaMode::Auto`, which resolves to `Opaque` on most
        // compositors and discards the alpha channel entirely. Pick an alpha
        // mode that actually blends the window with whatever is behind it.
        if let Some(alpha_mode) = block_on(preferred_alpha_mode(&render_context, &surface)) {
            if surface.config.alpha_mode != alpha_mode {
                surface.config.alpha_mode = alpha_mode;
                render_context.configure_surface(&surface);
            }
        }

        let device = &render_context.devices[surface.dev_id].device;
        let options = RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
            ..RendererOptions::default()
        };

        let vello =
            VelloRenderer::new(device, options).context("failed to create vello renderer")?;

        let cached_theme = UiTheme::from_config(&config);
        let background = BackgroundImage::from_config(&config).ok().flatten();

        Ok(Self {
            render_context,
            surface: Some(surface),
            vello: Some(vello),
            scene: Scene::new(),
            chrome_scene: Scene::new(),
            chrome_scene_valid: false,
            pane_terminal_scenes: Vec::new(),
            pane_scene_initialized: Vec::new(),
            cached_theme,
            scene_builder: SceneBuilder::new(),
            tab_strip_renderer: TabStripRenderer,
            title_bar_renderer: TitleBarRenderer,
            menu_renderer: MenuRenderer,
            font_cx: parley::FontContext::new(),
            layout,
            tab_strip_layout: None,
            title_bar_layout: None,
            menu_layout: None,
            search_panel_bounds: None,
            search_close_bounds: None,
            config,
            background,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if size.width > 0 && size.height > 0 {
            self.render_context
                .resize_surface(surface, size.width, size.height);
        }
    }

    pub fn set_layout(&mut self, layout: TerminalLayout) {
        self.layout = layout;
    }

    pub fn set_config(&mut self, config: Config) {
        self.cached_theme = UiTheme::from_config(&config);
        self.background = BackgroundImage::from_config(&config).ok().flatten();
        self.config = config;
        self.invalidate_text_cache();
    }

    pub fn present_clear(&mut self) -> Result<()> {
        let width = self
            .surface
            .as_ref()
            .map(|surface| surface.config.width)
            .unwrap_or(1);
        let height = self
            .surface
            .as_ref()
            .map(|surface| surface.config.height)
            .unwrap_or(1);

        self.scene.reset();
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &self.cached_theme.chrome_brush(),
            None,
            &Rect::new(0.0, 0.0, width as f64, height as f64),
        );
        self.render_scene(1.0)?;
        Ok(())
    }

    pub fn invalidate_text_cache(&mut self) {
        self.scene_builder.invalidate_font_cache();
        self.pane_terminal_scenes.clear();
        self.pane_scene_initialized.clear();
        self.chrome_scene_valid = false;
    }

    pub fn sync_pane_count(&mut self, count: usize) {
        if self.pane_terminal_scenes.len() > count {
            self.pane_terminal_scenes.truncate(count);
            self.pane_scene_initialized.truncate(count);
        }
    }

    fn ensure_pane_scenes(&mut self, count: usize) {
        while self.pane_terminal_scenes.len() < count {
            self.pane_terminal_scenes.push(Scene::new());
            self.pane_scene_initialized.push(false);
        }
    }

    pub fn tab_strip_layout(&self) -> Option<&TabStripLayout> {
        self.tab_strip_layout.as_ref()
    }

    pub fn title_bar_layout(&self) -> Option<&TitleBarLayout> {
        self.title_bar_layout.as_ref()
    }

    pub fn menu_layout(&self) -> Option<&MenuLayout> {
        self.menu_layout.as_ref()
    }

    pub fn render(
        &mut self,
        panes: &[PaneRender<'_>],
        focused_pane: usize,
        frame: ChromeFrame<'_>,
    ) -> Result<RenderTimings> {
        let _render_span = tracing::trace_span!("render_panes").entered();
        let scene_start = Instant::now();
        let width = self
            .surface
            .as_ref()
            .map(|surface| surface.config.width)
            .unwrap_or(1) as f64;

        self.ensure_pane_scenes(panes.len());

        // Partial damage updates append to a persistent per-pane scene and rely on
        // `clear_row_band` painting an opaque rect over the previous content. With
        // transparency the clear rect is semi-transparent, so it cannot erase the
        // old display-list contents and they accumulate (ghosted glyphs, colored
        // blocks "stretching" to the right). When the terminal is translucent we
        // must rebuild the whole pane scene from scratch each frame instead.
        let translucent = frame.terminal_opacity < 1.0 - f32::EPSILON;

        for (index, pane) in panes.iter().enumerate() {
            let effective_damage =
                if translucent || !self.pane_scene_initialized[index] || pane.damage.is_full() {
                    &FrameDamage::Full
                } else {
                    pane.damage
                };
            self.scene_builder.update_terminal(
                &mut self.pane_terminal_scenes[index],
                pane.frame,
                pane.layout,
                &self.config,
                &mut self.font_cx,
                frame.terminal_opacity,
                effective_damage,
                pane.ime_preedit,
            );
            if effective_damage.is_full() {
                self.pane_scene_initialized[index] = true;
            }
        }

        self.scene.reset();

        if let Some(background) = self.background.as_ref() {
            if !background.matches_config(&self.config) {
                self.background = BackgroundImage::from_config(&self.config).ok().flatten();
            }
        } else if self.config.appearance.background_image.is_some() {
            self.background = BackgroundImage::from_config(&self.config).ok().flatten();
        }

        if let Some(background) = self.background.as_ref() {
            let content_x = f64::from(frame.metrics.content_offset_x());
            let content_y = f64::from(frame.metrics.content_offset_y());
            let width = self
                .surface
                .as_ref()
                .map(|surface| surface.config.width)
                .unwrap_or(1) as f64;
            let height = self
                .surface
                .as_ref()
                .map(|surface| surface.config.height)
                .unwrap_or(1) as f64;
            background.draw(
                &mut self.scene,
                Rect::new(content_x, content_y, width, height),
            );
        }

        let content_x = f64::from(frame.metrics.content_offset_x());
        let content_y = f64::from(frame.metrics.content_offset_y());
        let content_width = self
            .surface
            .as_ref()
            .map(|surface| surface.config.width)
            .unwrap_or(1) as f64;
        let content_height = self
            .surface
            .as_ref()
            .map(|surface| surface.config.height)
            .unwrap_or(1) as f64;
        draw_background_shader(
            &mut self.scene,
            Rect::new(content_x, content_y, content_width, content_height),
            &self.config,
        );

        for terminal_scene in self.pane_terminal_scenes.iter().take(panes.len()) {
            self.scene.append(terminal_scene, None);
        }

        if panes.len() > 1 {
            let separator = self.cached_theme.muted_brush();
            let accent = self.cached_theme.accent_brush();
            let gutter = f64::from(GRID_GUTTER);

            for (index, pane) in panes.iter().enumerate() {
                let bounds = pane.layout.pixel_bounds();
                let x0 = f64::from(bounds.x0);
                let y0 = f64::from(bounds.y0);
                let x1 = f64::from(bounds.x1);
                let y1 = f64::from(bounds.y1);

                if index == focused_pane {
                    let inset = 1.0;
                    let focus_rect = RoundedRect::new(
                        x0 + inset,
                        y0 + inset,
                        x1 - inset,
                        y1 - inset,
                        RoundedRectRadii::from_single_radius(2.0),
                    );
                    self.scene.stroke(
                        &Stroke::new(2.0),
                        Affine::IDENTITY,
                        &accent,
                        None,
                        &focus_rect,
                    );
                }

                if x0 > f64::from(panes[0].layout.content_offset_x) {
                    self.scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &separator,
                        None,
                        &Rect::new(x0 - gutter, y0, x0, y1),
                    );
                }
                if y0 > f64::from(panes[0].layout.content_offset_y) {
                    self.scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &separator,
                        None,
                        &Rect::new(x0, y0 - gutter, x1, y0),
                    );
                }
            }
        }

        if frame.bell_flash {
            if let Some(pane) = panes.get(focused_pane) {
                let bounds = pane.layout.pixel_bounds();
                let x_off = f64::from(bounds.x0);
                let y_off = f64::from(bounds.y0);
                let w = f64::from(bounds.x1) - x_off;
                let h = f64::from(bounds.y1) - y_off;
                let flash = Brush::Solid(AlphaColor::from_rgba8(255, 255, 180, 64));
                self.scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &flash,
                    None,
                    &Rect::new(x_off, y_off, x_off + w, y_off + h),
                );
            }
        }

        let title_height = f64::from(frame.metrics.title_bar.height);
        let sidebar_height = (frame.window_height - title_height).max(0.0);

        if frame.chrome_dirty || !self.chrome_scene_valid {
            self.chrome_scene.reset();
            let tab_layout = TabStripLayout::compute(
                frame.metrics.tab_strip,
                title_height,
                sidebar_height,
                frame.tab_entries,
                frame.sidebar_collapsed,
            );
            self.tab_strip_renderer.render(
                &mut self.chrome_scene,
                &tab_layout,
                frame.tab_entries,
                frame.tab_scroll_offset,
                &self.config,
                &mut self.font_cx,
                &mut self.scene_builder.layout_cx,
            );
            self.tab_strip_layout = Some(tab_layout);

            if frame.metrics.title_bar.height > 0.0 {
                let title_layout = TitleBarLayout::compute(frame.metrics.title_bar, 0.0, width);
                let menu_anchor = if frame.menu_open {
                    Some(title_layout.hamburger_anchor())
                } else {
                    None
                };
                self.title_bar_renderer.render(
                    &mut self.chrome_scene,
                    &title_layout,
                    frame.window_title,
                    &self.config,
                    &mut self.font_cx,
                    &mut self.scene_builder.layout_cx,
                    self.layout.font_size,
                    frame.menu_open,
                );
                self.title_bar_layout = Some(title_layout);

                if let Some((anchor_x, anchor_y)) = menu_anchor {
                    let menu_layout =
                        MenuLayout::compute(anchor_x, anchor_y, width, frame.menu_entries);
                    self.menu_renderer.render(
                        &mut self.chrome_scene,
                        &menu_layout,
                        frame.menu_entries,
                        &self.config,
                        &mut self.font_cx,
                        &mut self.scene_builder.layout_cx,
                        self.layout.font_size,
                    );
                    self.menu_layout = Some(menu_layout);
                } else {
                    self.menu_layout = None;
                }
            } else {
                self.title_bar_layout = None;
                self.menu_layout = None;
            }

            self.chrome_scene_valid = true;
        }

        self.scene.append(&self.chrome_scene, None);

        if let Some(stats) = frame.perf_overlay {
            self.render_perf_overlay(stats, width, &frame);
        }

        if let Some(search) = frame.search_overlay {
            self.render_search_overlay(search, width, &frame);
        } else {
            self.search_panel_bounds = None;
            self.search_close_bounds = None;
        }

        if let Some(rename) = frame.rename_overlay {
            self.render_rename_overlay(rename, width, &frame);
        }

        if let Some(palette) = frame.command_palette {
            self.render_command_palette(palette, width, &frame);
        }

        let scene_elapsed = scene_start.elapsed();
        let gpu_start = Instant::now();
        let presented = self.render_scene(frame.terminal_opacity)?;
        Ok(RenderTimings {
            scene: scene_elapsed,
            gpu: gpu_start.elapsed(),
            presented,
        })
    }

    fn render_command_palette(
        &mut self,
        palette: CommandPaletteFrame<'_>,
        width: f64,
        frame: &ChromeFrame,
    ) {
        let panel_w = 430.0;
        let row_h = 26.0;
        let visible_entries = palette.entries.len().min(6);
        let panel_h = 48.0 + row_h * visible_entries as f64 + 10.0;
        let title_h = f64::from(frame.metrics.title_bar.height);
        let content_left = f64::from(frame.metrics.content_offset_x());
        let x0 = content_left + ((width - content_left - panel_w) * 0.5).max(12.0);
        let y0 = title_h + 18.0;
        let x1 = x0 + panel_w;
        let y1 = y0 + panel_h;

        let panel = RoundedRect::new(x0, y0, x1, y1, RoundedRectRadii::from_single_radius(10.0));
        let panel_brush = Brush::Solid(AlphaColor::from_rgba8(16, 16, 16, 232));
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, &panel_brush, None, &panel);
        self.scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            &self.cached_theme.accent_brush(),
            None,
            &panel,
        );

        let family = font_family_from_config(&self.config);
        let title_font = (self.layout.font_size * 0.8).clamp(11.0, 14.0);
        let entry_font = (self.layout.font_size * 0.76).clamp(10.0, 13.0);
        let text_brush = Brush::Solid(AlphaColor::from_rgba8(238, 238, 238, 255));
        let muted_brush = Brush::Solid(AlphaColor::from_rgba8(170, 170, 170, 255));

        let query = if palette.query.is_empty() {
            "Type a command...".to_owned()
        } else {
            format!("> {}", palette.query)
        };
        draw_text(
            &mut self.scene_builder.layout_cx,
            &mut self.scene,
            &mut self.font_cx,
            &query,
            x0 + 14.0,
            y0 + 11.0,
            title_font,
            family.clone(),
            &text_brush,
            Some((panel_w - 28.0) as f32),
        );

        let list_top = y0 + 42.0;
        for (index, entry) in palette.entries.iter().take(visible_entries).enumerate() {
            let row_y = list_top + index as f64 * row_h;
            if index == palette.selected_index {
                let selected = RoundedRect::new(
                    x0 + 8.0,
                    row_y - 3.0,
                    x1 - 8.0,
                    row_y + row_h - 3.0,
                    RoundedRectRadii::from_single_radius(5.0),
                );
                self.scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &self.cached_theme.elevated_brush(),
                    None,
                    &selected,
                );
            }
            draw_text(
                &mut self.scene_builder.layout_cx,
                &mut self.scene,
                &mut self.font_cx,
                entry,
                x0 + 14.0,
                row_y,
                entry_font,
                family.clone(),
                if index == 0 {
                    &text_brush
                } else {
                    &muted_brush
                },
                Some((panel_w - 28.0) as f32),
            );
        }
    }

    fn render_search_overlay(
        &mut self,
        search: SearchOverlayFrame<'_>,
        width: f64,
        frame: &ChromeFrame,
    ) {
        let panel_w = 420.0;
        let panel_h = 38.0;
        let title_h = f64::from(frame.metrics.title_bar.height);
        let x0 = (width - panel_w - 14.0).max(f64::from(frame.metrics.content_offset_x()) + 12.0);
        let y0 = title_h + 12.0;
        let x1 = x0 + panel_w;
        let y1 = y0 + panel_h;

        let panel = RoundedRect::new(x0, y0, x1, y1, RoundedRectRadii::from_single_radius(8.0));
        let panel_brush = Brush::Solid(AlphaColor::from_rgba8(18, 18, 18, 220));
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, &panel_brush, None, &panel);
        self.scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            &self.cached_theme.accent_brush(),
            None,
            &panel,
        );

        // Close button (×) pinned to the right edge of the panel.
        let close_size = 22.0;
        let close_x0 = x1 - close_size - 8.0;
        let close_y0 = y0 + (panel_h - close_size) * 0.5;
        let close_bounds = Rect::new(
            close_x0,
            close_y0,
            close_x0 + close_size,
            close_y0 + close_size,
        );
        let close_rect = RoundedRect::new(
            close_bounds.x0,
            close_bounds.y0,
            close_bounds.x1,
            close_bounds.y1,
            RoundedRectRadii::from_single_radius(5.0),
        );
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &self.cached_theme.elevated_brush(),
            None,
            &close_rect,
        );

        let label = {
            let mut modes = Vec::new();
            if search.case_sensitive {
                modes.push("case");
            } else {
                modes.push("ignore case");
            }
            if search.use_regex {
                modes.push("regex");
            }
            if search.whole_word {
                modes.push("word");
            }
            let mode = modes.join(", ");
            if search.regex_error {
                format!("Search: {}  (invalid regex)", search.query)
            } else if search.query.is_empty() {
                format!("Search...  ({mode}, Alt+C/R/W, Esc close)")
            } else if search.match_count == 0 {
                format!("Search: {}  (no matches, {mode})", search.query)
            } else {
                format!(
                    "Search: {}  ({}/{})  ({mode})",
                    search.query,
                    search.current_match + 1,
                    search.match_count
                )
            }
        };
        let family = font_family_from_config(&self.config);
        let font_size = (self.layout.font_size * 0.78).clamp(11.0, 14.0);
        let text_brush = Brush::Solid(AlphaColor::from_rgba8(238, 238, 238, 255));
        let label_max_width = (close_bounds.x0 - x0 - 24.0).max(20.0) as f32;
        draw_text(
            &mut self.scene_builder.layout_cx,
            &mut self.scene,
            &mut self.font_cx,
            &label,
            x0 + 12.0,
            y0 + 9.0,
            font_size,
            family.clone(),
            &text_brush,
            Some(label_max_width),
        );

        draw_text(
            &mut self.scene_builder.layout_cx,
            &mut self.scene,
            &mut self.font_cx,
            "×",
            close_bounds.x0 + 6.0,
            close_bounds.y0 + 2.0,
            (font_size + 2.0).min(16.0),
            family,
            &text_brush,
            Some(close_size as f32),
        );

        self.search_panel_bounds = Some(Rect::new(x0, y0, x1, y1));
        self.search_close_bounds = Some(close_bounds);
    }

    fn render_rename_overlay(
        &mut self,
        rename: RenameOverlayFrame<'_>,
        width: f64,
        frame: &ChromeFrame,
    ) {
        let panel_w = 360.0;
        let panel_h = 38.0;
        let title_h = f64::from(frame.metrics.title_bar.height);
        let x0 = (width - panel_w - 14.0).max(f64::from(frame.metrics.content_offset_x()) + 12.0);
        let y0 = title_h + 12.0;
        let x1 = x0 + panel_w;
        let y1 = y0 + panel_h;

        let panel = RoundedRect::new(x0, y0, x1, y1, RoundedRectRadii::from_single_radius(8.0));
        let panel_brush = Brush::Solid(AlphaColor::from_rgba8(18, 18, 18, 220));
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, &panel_brush, None, &panel);
        self.scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            &self.cached_theme.accent_brush(),
            None,
            &panel,
        );

        let label = if rename.draft.is_empty() {
            "Rename tab...  (Enter to save, Esc to cancel)".to_owned()
        } else {
            format!("Rename: {}", rename.draft)
        };
        let family = font_family_from_config(&self.config);
        let font_size = (self.layout.font_size * 0.78).clamp(11.0, 14.0);
        let text_brush = Brush::Solid(AlphaColor::from_rgba8(238, 238, 238, 255));
        draw_text(
            &mut self.scene_builder.layout_cx,
            &mut self.scene,
            &mut self.font_cx,
            &label,
            x0 + 12.0,
            y0 + 9.0,
            font_size,
            family,
            &text_brush,
            Some((panel_w - 24.0) as f32),
        );
    }

    pub fn search_overlay_hit(&self, x: f64, y: f64) -> Option<SearchOverlayHit> {
        let panel = self.search_panel_bounds?;
        if let Some(close) = self.search_close_bounds {
            if close.contains((x, y)) {
                return Some(SearchOverlayHit::Close);
            }
        }
        panel.contains((x, y)).then_some(SearchOverlayHit::Inside)
    }

    fn render_perf_overlay(&mut self, stats: &PerfStatsSnapshot, width: f64, frame: &ChromeFrame) {
        let panel_w = 220.0;
        let line_h = 14.0;
        let panel_h = 118.0;
        let title_h = f64::from(frame.metrics.title_bar.height);
        let x0 = (width - panel_w - 12.0).max(f64::from(frame.metrics.content_offset_x()) + 12.0);
        let y0 = title_h + 10.0;
        let x1 = x0 + panel_w;
        let y1 = y0 + panel_h;

        let panel = RoundedRect::new(x0, y0, x1, y1, RoundedRectRadii::from_single_radius(6.0));
        let panel_brush = Brush::Solid(AlphaColor::from_rgba8(0, 0, 0, 184));
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, &panel_brush, None, &panel);
        self.scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            &self.cached_theme.accent_brush(),
            None,
            &panel,
        );

        let family = font_family_from_config(&self.config);
        let font_size = (self.layout.font_size * 0.72).clamp(10.0, 13.0);
        let text_brush = Brush::Solid(AlphaColor::from_rgba8(238, 238, 238, 255));
        let lines = [
            format!("Frame {:>5.2} ms  {:>4.0} fps", stats.frame_ms, stats.fps),
            format!(
                "Capture {:>5.2} ms  Scene {:>5.2} ms",
                stats.capture_ms, stats.scene_ms
            ),
            format!(
                "GPU {:>5.2} ms  Skipped {:>4}",
                stats.gpu_ms, stats.skipped_frames
            ),
            format!(
                "Panes {}  Full {}  Dirty rows {}",
                stats.panes, stats.full_panes, stats.dirty_rows
            ),
            format!("Opacity {:>3.0}%", frame.terminal_opacity * 100.0),
        ];

        for (index, line) in lines.iter().enumerate() {
            draw_text(
                &mut self.scene_builder.layout_cx,
                &mut self.scene,
                &mut self.font_cx,
                line,
                x0 + 10.0,
                y0 + 10.0 + index as f64 * line_h,
                font_size,
                family.clone(),
                &text_brush,
                Some((panel_w - 20.0) as f32),
            );
        }

        let graph_y = y0 + 88.0;
        let graph_h = 20.0;
        let graph_w = panel_w - 20.0;
        let count = stats.fps_history_len.min(60);
        if count > 1 {
            let max_fps = stats
                .fps_history
                .iter()
                .take(count)
                .copied()
                .fold(1.0f32, f32::max);
            for i in 0..count {
                let fps = stats.fps_history[i];
                let bar_h = (fps / max_fps) * graph_h as f32;
                let bx0 = x0 + 10.0 + (i as f64 / count as f64) * graph_w;
                let bx1 = x0 + 10.0 + ((i + 1) as f64 / count as f64) * graph_w;
                self.scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &self.cached_theme.accent_brush(),
                    None,
                    &Rect::new(
                        bx0,
                        graph_y + graph_h - f64::from(bar_h),
                        bx1 - 0.5,
                        graph_y + graph_h,
                    ),
                );
            }
        }
    }

    /// Renders the current scene and presents it. Returns `false` when the
    /// surface texture could not be acquired (the frame was dropped), so the
    /// caller can retry instead of leaving a stale frame on screen.
    fn render_scene(&mut self, terminal_opacity: f32) -> Result<bool> {
        let surface = self
            .surface
            .as_mut()
            .context("renderer surface not initialized")?;
        let vello = self
            .vello
            .as_mut()
            .context("vello renderer not initialized")?;

        let width = surface.config.width;
        let height = surface.config.height;
        if width == 0 || height == 0 {
            return Ok(false);
        }

        let device = &self.render_context.devices[surface.dev_id].device;
        let queue = &self.render_context.devices[surface.dev_id].queue;

        let base_color = if terminal_opacity < 1.0 - f32::EPSILON {
            AlphaColor::from_rgba8(0, 0, 0, 0)
        } else {
            self.cached_theme.chrome
        };

        vello
            .render_to_texture(
                device,
                queue,
                &self.scene,
                &surface.target_view,
                &RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .context("failed to render vello scene")?;

        let surface_texture = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => return Ok(false),
            other => {
                return Err(anyhow::anyhow!(
                    "failed to acquire surface texture: {other:?}"
                ));
            }
        };
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sherion-blit"),
        });

        surface
            .blitter
            .copy(device, &mut encoder, &surface.target_view, &surface_view);

        queue.submit(Some(encoder.finish()));
        surface_texture.present();

        Ok(true)
    }
}

/// Picks a non-opaque surface alpha mode so the compositor blends the window
/// with the desktop behind it. Returns `None` if only opaque modes exist.
async fn preferred_alpha_mode(
    render_context: &RenderContext,
    surface: &RenderSurface<'_>,
) -> Option<wgpu::CompositeAlphaMode> {
    let adapter = render_context
        .instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface.surface),
        })
        .await
        .ok()?;

    let caps = surface.surface.get_capabilities(&adapter);
    // Vello writes premultiplied-alpha pixels into its target texture, so
    // PreMultiplied is the natural match. Fall back to the other blending
    // modes if the compositor doesn't offer it.
    [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
    ]
    .into_iter()
    .find(|mode| caps.alpha_modes.contains(mode))
}

pub type Renderer = GpuRenderer;

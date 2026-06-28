use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TerminalEvent, Notify, OnResize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::viewport_to_point;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::clipboard::{copy_text, paste_text};
use crate::config::{Config, ThemeMode, ViewModeConfig};
use crate::input::KeyAction;
use crate::pty::PtySession;
use crate::render::frame::{
    capture_terminal_frame, FrameDamage, SearchMatch, TerminalFrame, TerminalRowBuffer,
};
use crate::render::{
    border_size, collapsed_sidebar_width, cursor_for_direction, pane_rects, resize_direction_at,
    sidebar_width_bounds, ChromeFrame, ChromeMetrics, CommandPaletteFrame, MenuAction, MenuEntry,
    PaneRender, PerfStatsSnapshot, RenameOverlayFrame, Renderer, SearchOverlayFrame,
    SearchOverlayHit, TabBarEntry, TabStripHit, TabStripLayout, TerminalLayout, TitleBarHit,
    MAX_GRID_PANES,
};
use crate::tabs::{EventLoopProxyFactory, Tab, TabId, TabManager};

// Open wide enough that typical startup output (e.g. fastfetch's logo + color
// palette, ~106 columns) fits without wrapping. A narrow window forces a wrap at
// the bottom row, and the scroll that follows fills the new line with the active
// background (standard BCE), which looks like the last palette color "stretching"
// to the right edge.
pub const INITIAL_WIDTH: u32 = 1200;
pub const INITIAL_HEIGHT: u32 = 760;
pub const PTY_RESIZE_DEBOUNCE: Duration = Duration::from_millis(150);
pub const STARTUP_SETTLE: Duration = Duration::from_millis(120);
pub const BELL_FLASH_DURATION: Duration = Duration::from_millis(150);
pub const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(300);
pub const DOUBLE_CLICK_SLOP: f64 = 5.0;
pub const FONT_ZOOM_MIN: f32 = 0.5;
pub const FONT_ZOOM_MAX: f32 = 3.0;
pub const FONT_ZOOM_STEP: f32 = 0.1;
pub const OPACITY_MIN: f32 = 0.2;
pub const OPACITY_MAX: f32 = 1.0;
pub const OPACITY_STEP: f32 = 0.05;
pub const BUSY_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Single,
    Grid,
}

impl From<ViewModeConfig> for ViewMode {
    fn from(value: ViewModeConfig) -> Self {
        match value {
            ViewModeConfig::Single => Self::Single,
            ViewModeConfig::Grid => Self::Grid,
        }
    }
}

impl From<ViewMode> for ViewModeConfig {
    fn from(value: ViewMode) -> Self {
        match value {
            ViewMode::Single => Self::Single,
            ViewMode::Grid => Self::Grid,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PaneSlot {
    pub tab_index: usize,
    pub layout: TerminalLayout,
}

#[derive(Default)]
struct SearchState {
    active: bool,
    query: String,
    match_count: usize,
    current_match: usize,
}

#[derive(Default)]
struct RenameState {
    active: bool,
    draft: String,
}

#[derive(Default)]
struct CommandPaletteState {
    active: bool,
    query: String,
}

pub struct WindowState {
    pub window: Arc<Window>,
    pub renderer: Option<Renderer>,
    pub tabs: TabManager,
    pub proxy_factory: EventLoopProxyFactory,
    pub chrome_metrics: ChromeMetrics,
    pub layout: TerminalLayout,
    pub view_mode: ViewMode,
    pub panes: Vec<PaneSlot>,
    pub modifiers: ModifiersState,
    pub pending_pty_layouts: Vec<(usize, TerminalLayout)>,
    pub pty_resize_deadline: Option<Instant>,
    pub needs_redraw: bool,
    pub selecting: bool,
    pub menu_open: bool,
    pub menu_slider_dragging: bool,
    pub tab_scroll_offset: f64,
    pub cursor_position: (f64, f64),
    pub initial_tab_pending: bool,
    pub startup_deadline: Option<Instant>,
    pub sidebar_width: f32,
    pub sidebar_dragging: bool,
    pub sidebar_collapsed: bool,
    pub font_zoom: f32,
    pub terminal_opacity: f32,
    pub perf_overlay_enabled: bool,
    search: SearchState,
    rename: RenameState,
    command_palette: CommandPaletteState,
    pub menu_entries: Vec<MenuEntry>,
    pub bell_flash_until: Option<Instant>,
    pub last_click_time: Instant,
    pub last_click_pos: (f64, f64),
    pub click_count: u32,
    pane_frame_bufs: Vec<TerminalRowBuffer>,
    pane_frames: Vec<TerminalFrame>,
    pane_row_scratch: Vec<Vec<(Point, Cell)>>,
    pane_damage: Vec<FrameDamage>,
    force_full_capture: bool,
    chrome_dirty: bool,
    cached_tab_entries: Vec<TabBarEntry>,
    cached_window_title: String,
    last_busy_elapsed_secs: Option<u64>,
    bell_flash_was_active: bool,
    perf_stats: PerfStatsSnapshot,
}

pub enum TabStripAction {
    None,
    NewTab,
    Detach(TabId),
    WindowEmpty,
}

pub enum MenuAppAction {
    NewTab,
    DuplicateTab,
    CloseTab,
    DetachTab,
    Quit,
    Theme(ThemeMode),
}

impl WindowState {
    pub fn new_pending(
        window: Arc<Window>,
        proxy_factory: EventLoopProxyFactory,
        config: &Config,
        terminal_opacity: f32,
    ) -> Self {
        let mut chrome_metrics =
            ChromeMetrics::from_font_size(config.font.size, window.scale_factor() as f32);
        let sidebar_width = if config.ui.sidebar_width > 0.0 {
            config.ui.sidebar_width
        } else {
            chrome_metrics.tab_strip.width
        };
        chrome_metrics.tab_strip.width = if config.ui.sidebar_collapsed {
            collapsed_sidebar_width(window.scale_factor() as f32)
        } else {
            sidebar_width
        };
        let layout = TerminalLayout::from_pixels(
            window.inner_size(),
            window.scale_factor(),
            config.font.size * config.ui.font_zoom,
            chrome_metrics.content_offset_x(),
            chrome_metrics.content_offset_y(),
        );

        Self {
            window,
            renderer: None,
            tabs: TabManager::empty(),
            proxy_factory,
            chrome_metrics,
            layout,
            view_mode: ViewMode::from(config.ui.view_mode),
            panes: Vec::new(),
            modifiers: ModifiersState::empty(),
            pending_pty_layouts: Vec::new(),
            pty_resize_deadline: None,
            needs_redraw: true,
            selecting: false,
            menu_open: false,
            menu_slider_dragging: false,
            tab_scroll_offset: 0.0,
            cursor_position: (0.0, 0.0),
            initial_tab_pending: true,
            startup_deadline: Some(Instant::now() + STARTUP_SETTLE),
            sidebar_width,
            sidebar_dragging: false,
            sidebar_collapsed: config.ui.sidebar_collapsed,
            font_zoom: config.ui.font_zoom,
            terminal_opacity,
            perf_overlay_enabled: false,
            search: SearchState::default(),
            rename: RenameState::default(),
            command_palette: CommandPaletteState::default(),
            menu_entries: Vec::new(),
            bell_flash_until: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            pane_frame_bufs: Vec::new(),
            pane_frames: Vec::new(),
            pane_row_scratch: Vec::new(),
            pane_damage: Vec::new(),
            force_full_capture: true,
            chrome_dirty: true,
            cached_tab_entries: Vec::new(),
            cached_window_title: String::new(),
            last_busy_elapsed_secs: None,
            bell_flash_was_active: false,
            perf_stats: PerfStatsSnapshot::default(),
        }
    }

    pub fn new_with_tab(
        window: Arc<Window>,
        tab: Tab,
        proxy_factory: EventLoopProxyFactory,
        config: &Config,
        terminal_opacity: f32,
        font_zoom: f32,
        sidebar_width: f32,
        sidebar_collapsed: bool,
    ) -> Self {
        let mut chrome_metrics = ChromeMetrics::from_font_size(
            config.font.size * font_zoom,
            window.scale_factor() as f32,
        );
        chrome_metrics.tab_strip.width = if sidebar_collapsed {
            collapsed_sidebar_width(window.scale_factor() as f32)
        } else {
            sidebar_width
        };
        let layout = TerminalLayout::from_pixels(
            window.inner_size(),
            window.scale_factor(),
            config.font.size * font_zoom,
            chrome_metrics.content_offset_x(),
            chrome_metrics.content_offset_y(),
        );

        Self {
            window,
            renderer: None,
            tabs: TabManager::with_initial(tab),
            proxy_factory,
            chrome_metrics,
            layout,
            view_mode: ViewMode::Single,
            panes: vec![PaneSlot {
                tab_index: 0,
                layout,
            }],
            modifiers: ModifiersState::empty(),
            pending_pty_layouts: Vec::new(),
            pty_resize_deadline: None,
            needs_redraw: true,
            selecting: false,
            menu_open: false,
            menu_slider_dragging: false,
            tab_scroll_offset: 0.0,
            cursor_position: (0.0, 0.0),
            initial_tab_pending: false,
            startup_deadline: None,
            sidebar_width,
            sidebar_dragging: false,
            sidebar_collapsed,
            font_zoom,
            terminal_opacity,
            perf_overlay_enabled: false,
            search: SearchState::default(),
            rename: RenameState::default(),
            command_palette: CommandPaletteState::default(),
            menu_entries: Vec::new(),
            bell_flash_until: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            pane_frame_bufs: Vec::new(),
            pane_frames: Vec::new(),
            pane_row_scratch: Vec::new(),
            pane_damage: Vec::new(),
            force_full_capture: true,
            chrome_dirty: true,
            cached_tab_entries: Vec::new(),
            cached_window_title: String::new(),
            last_busy_elapsed_secs: None,
            bell_flash_was_active: false,
            perf_stats: PerfStatsSnapshot::default(),
        }
    }

    fn mark_chrome_dirty(&mut self) {
        self.chrome_dirty = true;
    }

    pub fn dismiss_menu(&mut self) {
        if self.menu_open {
            self.menu_open = false;
            self.menu_slider_dragging = false;
            self.mark_chrome_dirty();
        }
    }

    pub fn request_full_capture(&mut self) {
        self.force_full_capture = true;
    }

    fn ensure_pane_buffers(&mut self, pane_count: usize) {
        if self.pane_frame_bufs.len() < pane_count {
            self.pane_frame_bufs.resize_with(pane_count, Vec::new);
        }
        if self.pane_row_scratch.len() < pane_count {
            self.pane_row_scratch.resize_with(pane_count, Vec::new);
        }
        if self.pane_frames.len() < pane_count {
            let colors = std::sync::Arc::new(alacritty_terminal::term::color::Colors::default());
            while self.pane_frames.len() < pane_count {
                let row_count = self
                    .panes
                    .get(self.pane_frames.len())
                    .map(|pane| pane.layout.rows as usize)
                    .unwrap_or(1);
                self.pane_frames
                    .push(crate::render::frame::empty_terminal_frame(
                        row_count,
                        colors.clone(),
                    ));
            }
        }
        if self.pane_damage.len() < pane_count {
            self.pane_damage.resize(pane_count, FrameDamage::Full);
        }
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn effective_font_size(&self, config: &Config) -> f32 {
        config.font.size * self.font_zoom
    }

    pub fn chrome_metrics_for_window(&self, config: &Config) -> ChromeMetrics {
        let mut metrics = ChromeMetrics::from_font_size(
            self.effective_font_size(config),
            self.window.scale_factor() as f32,
        );
        metrics.tab_strip.width = if self.sidebar_collapsed {
            collapsed_sidebar_width(self.window.scale_factor() as f32)
        } else {
            self.sidebar_width
        };
        metrics
    }

    pub fn window_height(&self) -> f64 {
        self.window.inner_size().height as f64
    }

    pub fn terminal_layout_for_size(
        &self,
        config: &Config,
        size: PhysicalSize<u32>,
        scale: f64,
    ) -> TerminalLayout {
        let font_size = self.effective_font_size(config);
        TerminalLayout::from_pixels(
            size,
            scale,
            font_size,
            self.chrome_metrics.content_offset_x(),
            self.chrome_metrics.content_offset_y(),
        )
    }

    pub fn focused_pane_slot(&self) -> usize {
        let active = self.tabs.active_index();
        self.panes
            .iter()
            .position(|pane| pane.tab_index == active)
            .unwrap_or(0)
    }

    pub fn recompute_panes(&mut self, config: &Config) {
        let base = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        let size = self.window.inner_size();
        let content_x = base.content_offset_x;
        let content_y = base.content_offset_y;
        let content_w = size.width as f32 - content_x;
        let content_h = size.height as f32 - content_y;
        let scale = self.window.scale_factor();
        let font_size = self.effective_font_size(config);

        match self.view_mode {
            ViewMode::Single => {
                if self.tabs.is_empty() {
                    self.panes.clear();
                    self.layout = base;
                    return;
                }
                let active = self.tabs.active_index();
                self.panes = vec![PaneSlot {
                    tab_index: active,
                    layout: base,
                }];
                self.layout = base;
            }
            ViewMode::Grid => {
                let n = self.tabs.len().min(MAX_GRID_PANES);
                if n == 0 {
                    self.panes.clear();
                    self.layout = base;
                    return;
                }

                let rects = pane_rects(content_x, content_y, content_w, content_h, n);
                self.panes = rects
                    .iter()
                    .enumerate()
                    .map(|(index, rect)| PaneSlot {
                        tab_index: index,
                        layout: TerminalLayout::from_rect(
                            rect.x0,
                            rect.y0,
                            rect.width(),
                            rect.height(),
                            scale,
                            font_size,
                        ),
                    })
                    .collect();

                let active = self.tabs.active_index();
                self.layout = self
                    .panes
                    .iter()
                    .find(|pane| pane.tab_index == active)
                    .map(|pane| pane.layout)
                    .unwrap_or(base);
            }
        }

        self.ensure_pane_buffers(self.panes.len());

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(self.layout);
            renderer.invalidate_text_cache();
            renderer.sync_pane_count(self.panes.len());
        }
    }

    fn pane_layout_for_tab(&self, tab_index: usize, full_layout: TerminalLayout) -> TerminalLayout {
        self.panes
            .iter()
            .find(|pane| pane.tab_index == tab_index)
            .map(|pane| pane.layout)
            .unwrap_or(full_layout)
    }

    fn resize_all_pane_terminals(&mut self, config: &Config) -> bool {
        let full_layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        let mut any_grid_changed = false;

        for tab_index in 0..self.tabs.len() {
            let layout = self.pane_layout_for_tab(tab_index, full_layout);
            if let Some(tab) = self.tabs.tab_at_index_mut(tab_index) {
                let mut term = tab.session.terminal.lock();
                if term.columns() != layout.cols as usize
                    || term.screen_lines() != layout.rows as usize
                {
                    any_grid_changed = true;
                    term.resize(layout);
                }
            }
        }

        if any_grid_changed {
            self.request_full_capture();
        }
        any_grid_changed
    }

    fn notify_all_pane_ptys(&mut self, config: &Config) {
        let full_layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        for tab_index in 0..self.tabs.len() {
            let layout = self.pane_layout_for_tab(tab_index, full_layout);
            self.notify_pty_resize_tab(tab_index, layout);
        }
    }

    pub fn apply_pane_layouts(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        let full_layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );

        if !self.resize_all_pane_terminals(config) {
            return;
        }

        self.pending_pty_layouts = (0..self.tabs.len())
            .map(|tab_index| (tab_index, self.pane_layout_for_tab(tab_index, full_layout)))
            .collect();
        self.schedule_pty_resize(event_loop);
    }

    pub fn sync_terminal_layout(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        self.chrome_metrics = self.chrome_metrics_for_window(config);
        self.recompute_panes(config);
        self.apply_pane_layouts(config, event_loop);
        self.clamp_tab_scroll();
        self.request_full_capture();
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn set_view_mode(&mut self, config: &Config, mode: ViewMode, event_loop: &ActiveEventLoop) {
        if self.view_mode == mode {
            return;
        }
        self.view_mode = mode;
        if mode == ViewMode::Grid {
            let n = self.tabs.len().min(MAX_GRID_PANES);
            if n > 0 && self.tabs.active_index() >= n {
                if let Some(tab) = self.tabs.tab_at_index(n - 1) {
                    self.tabs.set_active(tab.id);
                }
            }
        }
        self.sync_terminal_layout(config, event_loop);
    }

    pub fn focus_tab(&mut self, id: TabId, config: &Config) {
        if !self.tabs.set_active(id) {
            return;
        }
        let active = self.tabs.active_index();
        if let Some(pane) = self.panes.iter().find(|pane| pane.tab_index == active) {
            self.layout = pane.layout;
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_layout(self.layout);
            }
        }
        self.update_window_title(config);
        self.ensure_active_tab_visible();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn focus_pane_slot(&mut self, slot: usize, config: &Config) {
        let Some(pane) = self.panes.get(slot) else {
            return;
        };
        let tab_index = pane.tab_index;
        let layout = pane.layout;
        let Some(tab) = self.tabs.tab_at_index(tab_index) else {
            return;
        };
        let id = tab.id;
        if !self.tabs.set_active(id) {
            return;
        }
        self.layout = layout;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(layout);
        }
        self.update_window_title(config);
        self.ensure_active_tab_visible();
        self.request_full_capture();
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn pane_at(&self, x: f64, y: f64) -> Option<usize> {
        for (index, pane) in self.panes.iter().enumerate() {
            let bounds = pane.layout.pixel_bounds();
            if x >= f64::from(bounds.x0)
                && x < f64::from(bounds.x1)
                && y >= f64::from(bounds.y0)
                && y < f64::from(bounds.y1)
            {
                return Some(index);
            }
        }
        None
    }

    pub fn tab_index_for_scroll(&self, x: f64, y: f64) -> Option<usize> {
        if self.view_mode == ViewMode::Grid {
            self.pane_at(x, y)
                .and_then(|slot| self.panes.get(slot).map(|pane| pane.tab_index))
        } else if self.tabs.is_empty() {
            None
        } else {
            Some(self.tabs.active_index())
        }
    }

    pub fn url_at_position(&self, x: f64, y: f64) -> Option<String> {
        let slot = self
            .pane_at(x, y)
            .unwrap_or_else(|| self.focused_pane_slot());
        let pane = self.panes.get(slot)?;
        let frame = self.pane_frames.get(slot)?;
        let row = ((y - f64::from(pane.layout.content_offset_y))
            / f64::from(pane.layout.cell_height))
        .floor()
        .max(0.0) as usize;
        let col = ((x - f64::from(pane.layout.content_offset_x))
            / f64::from(pane.layout.cell_width))
        .floor()
        .max(0.0) as usize;
        if row >= pane.layout.rows as usize || col >= pane.layout.cols as usize {
            return None;
        }

        url_in_row(frame.rows.get(row)?, pane.layout.cols as usize, col)
    }

    pub fn open_url_at_position(&self, x: f64, y: f64) -> bool {
        let Some(url) = self.url_at_position(x, y) else {
            return false;
        };
        match std::process::Command::new("xdg-open").arg(&url).spawn() {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, %url, "failed to open URL");
                false
            }
        }
    }

    pub fn tab_strip_layout_snapshot(&self) -> TabStripLayout {
        let top = f64::from(self.chrome_metrics.title_bar.height);
        let sidebar_height = (self.window_height() - top).max(0.0);
        TabStripLayout::compute(
            self.chrome_metrics.tab_strip,
            top,
            sidebar_height,
            &self.tab_bar_entries(),
            self.sidebar_collapsed,
        )
    }

    pub fn clamp_tab_scroll(&mut self) {
        let max = self.tab_strip_layout_snapshot().max_scroll_offset();
        self.tab_scroll_offset = self.tab_scroll_offset.clamp(0.0, max);
    }

    pub fn scroll_tab_strip(&mut self, delta_lines: i32) {
        let delta = -f64::from(delta_lines) * self.chrome_metrics.tab_strip.row_height();
        self.tab_scroll_offset += delta;
        self.clamp_tab_scroll();
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn ensure_active_tab_visible(&mut self) {
        let layout = self.tab_strip_layout_snapshot();
        let Some(active_id) = self.tabs.active_id() else {
            return;
        };
        let Some(range) = layout.active_tab_range(active_id) else {
            return;
        };

        let visible_top = self.tab_scroll_offset;
        let visible_bottom = self.tab_scroll_offset + layout.list_height;
        if range.start < visible_top {
            self.tab_scroll_offset = range.start;
        } else if range.end > visible_bottom {
            self.tab_scroll_offset = (range.end - layout.list_height).max(0.0);
        }
        self.clamp_tab_scroll();
    }

    pub fn maybe_spawn_initial_tab(
        &mut self,
        config: &Config,
        next_tab_id: &mut usize,
        event_loop: &ActiveEventLoop,
    ) {
        if !self.initial_tab_pending {
            return;
        }
        let Some(deadline) = self.startup_deadline else {
            return;
        };
        if Instant::now() < deadline {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
            return;
        }

        self.ensure_renderer(config);
        self.chrome_metrics = self.chrome_metrics_for_window(config);
        self.layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(self.layout);
        }

        self.initial_tab_pending = false;
        self.startup_deadline = None;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        let restored_cwds = if config.session.restore_tabs {
            config.session.cwd.clone()
        } else {
            Vec::new()
        };
        let first_cwd = restored_cwds.first().cloned();

        let id = TabId(*next_tab_id);
        *next_tab_id += 1;
        let event_proxy = self.proxy_factory.for_tab(id);
        tracing::info!(
            cols = self.layout.cols,
            rows = self.layout.rows,
            "spawning initial tab"
        );
        match PtySession::spawn_with_working_directory(config, self.layout, event_proxy, first_cwd)
        {
            Ok(session) => {
                let tab = Tab {
                    id,
                    title: format!("Tab {}", id.0),
                    session,
                    running_since: None,
                };
                self.tabs = TabManager::with_initial(tab);
                for cwd in restored_cwds.into_iter().skip(1).take(15) {
                    let _ = self.tabs.spawn_tab(
                        config,
                        self.layout,
                        self.proxy_factory.clone(),
                        next_tab_id,
                        Some(cwd),
                    );
                }
                if let Some(first_tab) = self.tabs.tab_at_index(0) {
                    self.tabs.set_active(first_tab.id);
                }
                self.recompute_panes(config);
            }
            Err(error) => {
                tracing::error!(%error, "failed to spawn initial tab");
            }
        }
        self.update_window_title(config);
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn ensure_renderer(&mut self, config: &Config) {
        if self.renderer.is_some() {
            return;
        }

        self.chrome_metrics = self.chrome_metrics_for_window(config);
        self.layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );

        tracing::info!("initializing GPU renderer");
        match Renderer::new(Arc::clone(&self.window), self.layout, config.clone()) {
            Ok(mut renderer) => {
                renderer.resize(self.window.inner_size());
                let _ = renderer.present_clear();
                self.renderer = Some(renderer);
                self.layout = self.terminal_layout_for_size(
                    config,
                    self.window.inner_size(),
                    self.window.scale_factor(),
                );
                self.recompute_panes(config);
                self.resize_all_pane_terminals(config);
                self.notify_all_pane_ptys(config);
            }
            Err(error) => {
                tracing::error!(%error, "failed to initialize renderer");
            }
        }
    }

    pub fn spawn_tab(
        &mut self,
        config: &Config,
        next_tab_id: &mut usize,
        event_loop: &ActiveEventLoop,
    ) {
        let spawn_layout = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        let working_directory = self
            .tabs
            .active_tab()
            .and_then(|tab| tab.session.current_working_directory());
        if self
            .tabs
            .spawn_tab(
                config,
                spawn_layout,
                self.proxy_factory.clone(),
                next_tab_id,
                working_directory,
            )
            .is_some()
        {
            self.sync_terminal_layout(config, event_loop);
            self.ensure_active_tab_visible();
        }
    }

    pub fn close_tab(&mut self, id: TabId, config: &Config, event_loop: &ActiveEventLoop) -> bool {
        self.tabs.close_tab(id);
        self.sync_terminal_layout(config, event_loop);
        self.tabs.is_empty()
    }

    pub fn notify_pty_resize_tab(&mut self, tab_index: usize, layout: TerminalLayout) {
        if let Some(tab) = self.tabs.tab_at_index_mut(tab_index) {
            tab.session.notifier.on_resize(layout.window_size());
        }
    }

    pub fn handle_window_resize(
        &mut self,
        config: &Config,
        size: PhysicalSize<u32>,
        event_loop: &ActiveEventLoop,
    ) {
        self.ensure_renderer(config);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
        }

        self.recompute_panes(config);
        self.apply_pane_layouts(config, event_loop);
        self.clamp_tab_scroll();
        self.request_full_capture();
        self.mark_chrome_dirty();

        if self.initial_tab_pending {
            let deadline = Instant::now() + STARTUP_SETTLE;
            self.startup_deadline = Some(deadline);
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
        }

        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn schedule_pty_resize(&mut self, event_loop: &ActiveEventLoop) {
        if self.pending_pty_layouts.is_empty() {
            return;
        }
        let deadline = Instant::now() + PTY_RESIZE_DEBOUNCE;
        self.pty_resize_deadline = Some(deadline);
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(deadline));
    }

    pub fn flush_pty_resize(&mut self, event_loop: &ActiveEventLoop) {
        let Some(deadline) = self.pty_resize_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }

        let layouts: Vec<_> = self.pending_pty_layouts.drain(..).collect();
        for (tab_index, layout) in layouts {
            self.notify_pty_resize_tab(tab_index, layout);
        }

        self.needs_redraw = true;
        self.request_redraw();
        self.pty_resize_deadline = None;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    }

    pub fn handle_terminal_event(&mut self, config: &Config, tab_id: TabId, event: TerminalEvent) {
        match event {
            TerminalEvent::Wakeup => {
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::Title(title) => {
                self.tabs.set_title(tab_id, title);
                self.update_window_title(config);
                self.mark_chrome_dirty();
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::ResetTitle => {
                let fallback = format!("Tab {}", tab_id.0);
                self.tabs.reset_title(tab_id, &fallback);
                self.update_window_title(config);
                self.mark_chrome_dirty();
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::PtyWrite(data) => {
                if let Some(tab) = self.tabs.tab_by_id(tab_id) {
                    tab.session.notifier.notify(Cow::Owned(data.into_bytes()));
                }
            }
            TerminalEvent::Bell => {
                if config.bell.visual {
                    self.bell_flash_until = Some(Instant::now() + BELL_FLASH_DURATION);
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            TerminalEvent::Exit => {}
            _ => {}
        }
    }

    pub fn update_window_title(&self, config: &Config) {
        let title = self
            .tabs
            .active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| config.window.title.clone());
        self.window.set_title(&title);
    }

    pub fn window_title(&self, config: &Config) -> String {
        self.tabs
            .active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| config.window.title.clone())
    }

    pub fn reflow_terminal_layout(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        self.sync_terminal_layout(config, event_loop);
    }

    pub fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let term = tab.session.terminal.lock();
        if let Some(text) = term.selection_to_string() {
            if !text.is_empty() {
                copy_text(&text);
            }
        }
    }

    pub fn paste_from_clipboard(&mut self) {
        let Some(text) = paste_text() else {
            return;
        };
        if let Some(tab) = self.tabs.active_tab() {
            tab.session.notifier.notify(Cow::Owned(text.into_bytes()));
        }
    }

    pub fn clear_scrollback(&mut self) {
        if let Some(tab) = self.tabs.active_tab() {
            let mut term = tab.session.terminal.lock();
            term.grid_mut().clear_history();
            term.scroll_display(Scroll::Bottom);
        }
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn apply_font_zoom(
        &mut self,
        config: &Config,
        new_zoom: f32,
        event_loop: &ActiveEventLoop,
    ) {
        let clamped = new_zoom.clamp(FONT_ZOOM_MIN, FONT_ZOOM_MAX);
        if (clamped - self.font_zoom).abs() < f32::EPSILON {
            return;
        }
        self.font_zoom = clamped;
        self.reflow_terminal_layout(config, event_loop);
    }

    pub fn apply_terminal_opacity(&mut self, new_opacity: f32) {
        let clamped = new_opacity.clamp(OPACITY_MIN, OPACITY_MAX);
        if (clamped - self.terminal_opacity).abs() < f32::EPSILON {
            return;
        }
        self.terminal_opacity = clamped;
        // The terminal background fill bakes the opacity into the persistent
        // pane scene, which is only rebuilt on a full capture. Without this the
        // cached scene keeps its old alpha and the change never shows.
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn build_menu_entries(&self, theme_mode: ThemeMode) -> Vec<MenuEntry> {
        vec![
            MenuEntry::Action {
                action: MenuAction::NewTab,
                label: "New Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::DuplicateTab,
                label: "Duplicate Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::RenameTab,
                label: "Rename Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::CloseTab,
                label: "Close Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::DetachTab,
                label: "Move Tab to New Window".to_owned(),
            },
            MenuEntry::Separator,
            MenuEntry::Action {
                action: MenuAction::Copy,
                label: "Copy".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::Paste,
                label: "Paste".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::ClearScrollback,
                label: "Clear Scrollback".to_owned(),
            },
            MenuEntry::Separator,
            MenuEntry::ViewSelector {
                current: self.view_mode,
            },
            MenuEntry::ThemeSelector {
                current: theme_mode,
            },
            MenuEntry::Separator,
            MenuEntry::Action {
                action: MenuAction::ZoomIn,
                label: format!("Zoom In ({:.0}%)", self.font_zoom * 100.0),
            },
            MenuEntry::Action {
                action: MenuAction::ZoomOut,
                label: "Zoom Out".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::ZoomReset,
                label: "Reset Zoom".to_owned(),
            },
            MenuEntry::Separator,
            MenuEntry::OpacitySlider {
                value: self.terminal_opacity,
            },
            MenuEntry::Action {
                action: MenuAction::TogglePerfOverlay,
                label: format!(
                    "Perf Overlay: {}",
                    if self.perf_overlay_enabled {
                        "On"
                    } else {
                        "Off"
                    }
                ),
            },
            MenuEntry::Separator,
            MenuEntry::Action {
                action: MenuAction::Quit,
                label: "Quit".to_owned(),
            },
        ]
    }

    /// Apply an opacity value chosen via the slider and refresh the open menu so
    /// the knob and percentage label track the change.
    pub fn apply_slider_opacity(&mut self, theme_mode: ThemeMode, value: f32) {
        self.apply_terminal_opacity(value);
        if self.menu_open {
            self.refresh_menu_entries(theme_mode);
        }
    }

    pub fn refresh_menu_entries(&mut self, theme_mode: ThemeMode) {
        self.menu_entries = self.build_menu_entries(theme_mode);
        self.mark_chrome_dirty();
    }

    pub fn handle_command_palette_key(
        &mut self,
        config: &Config,
        theme_mode: ThemeMode,
        event_loop: &ActiveEventLoop,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> (bool, Option<MenuAppAction>) {
        if event.repeat || event.state != ElementState::Pressed {
            return (false, None);
        }

        if modifiers.control_key() && modifiers.shift_key() {
            if let Key::Character(text) = &event.logical_key {
                if text.eq_ignore_ascii_case("p") {
                    self.command_palette.active = true;
                    self.command_palette.query.clear();
                    self.search.active = false;
                    self.dismiss_menu();
                    self.needs_redraw = true;
                    self.request_redraw();
                    return (true, None);
                }
            }
        }

        if !self.command_palette.active {
            return (false, None);
        }

        let mut app_action = None;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.command_palette.active = false;
            }
            Key::Named(NamedKey::Backspace) => {
                self.command_palette.query.pop();
            }
            Key::Named(NamedKey::Enter) => match self.filtered_palette_items().first() {
                Some(PaletteItem::Command { action, .. }) => {
                    self.command_palette.active = false;
                    app_action = self.handle_menu_action(config, theme_mode, *action, event_loop);
                }
                Some(PaletteItem::Tab { id, .. }) => {
                    self.command_palette.active = false;
                    self.focus_tab(*id, config);
                }
                None => {}
            },
            Key::Character(text) if !modifiers.control_key() && !modifiers.alt_key() => {
                self.command_palette.query.push_str(text);
            }
            _ => {}
        }

        self.needs_redraw = true;
        self.request_redraw();
        (true, app_action)
    }

    fn filtered_palette_items(&self) -> Vec<PaletteItem> {
        let query = self.command_palette.query.to_ascii_lowercase();
        let mut items = Vec::new();

        for info in self.tabs.infos() {
            if query.is_empty() || info.title.to_ascii_lowercase().contains(&query) {
                items.push(PaletteItem::Tab {
                    id: info.id,
                    title: info.title,
                });
            }
        }

        for cmd in palette_commands() {
            if query.is_empty() || cmd.label.to_ascii_lowercase().contains(&query) {
                items.push(PaletteItem::Command {
                    label: cmd.label,
                    action: cmd.action,
                });
            }
        }

        items
    }

    fn command_palette_labels(&self) -> Vec<String> {
        self.filtered_palette_items()
            .into_iter()
            .map(|item| match item {
                PaletteItem::Tab { title, .. } => format!("Tab: {title}"),
                PaletteItem::Command { label, .. } => label.to_owned(),
            })
            .collect()
    }

    pub fn start_rename(&mut self) {
        self.rename.draft = self
            .tabs
            .active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_default();
        self.rename.active = true;
        self.search.active = false;
        self.command_palette.active = false;
        self.dismiss_menu();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn handle_rename_key(&mut self, config: &Config, event: &KeyEvent) -> bool {
        if event.repeat || event.state != ElementState::Pressed {
            return false;
        }

        if !self.rename.active {
            return false;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.rename.active = false;
            }
            Key::Named(NamedKey::Backspace) => {
                self.rename.draft.pop();
            }
            Key::Named(NamedKey::Enter) => {
                let title = self.rename.draft.trim().to_owned();
                if !title.is_empty() {
                    if let Some(id) = self.tabs.active_id() {
                        self.tabs.set_title(id, title);
                        self.update_window_title(config);
                        self.mark_chrome_dirty();
                    }
                }
                self.rename.active = false;
            }
            Key::Character(text) if !self.modifiers.control_key() && !self.modifiers.alt_key() => {
                self.rename.draft.push_str(text);
            }
            _ => {}
        }

        self.needs_redraw = true;
        self.request_redraw();
        true
    }

    pub fn search_active(&self) -> bool {
        self.search.active
    }

    pub fn close_search(&mut self) {
        if !self.search.active {
            return;
        }
        self.search.active = false;
        self.search.current_match = 0;
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    /// Handle a click while the search overlay is open. Returns true when the
    /// click was consumed (close button or a click outside the panel), so the
    /// caller can swallow it instead of starting a terminal selection.
    pub fn handle_search_overlay_click(&mut self, x: f64, y: f64) -> bool {
        if !self.search.active {
            return false;
        }
        let hit = self
            .renderer
            .as_ref()
            .and_then(|renderer| renderer.search_overlay_hit(x, y));
        match hit {
            Some(SearchOverlayHit::Close) => {
                self.close_search();
                true
            }
            Some(SearchOverlayHit::Inside) => true,
            None => {
                self.close_search();
                true
            }
        }
    }

    pub fn handle_search_key(&mut self, event: &KeyEvent, modifiers: ModifiersState) -> bool {
        if event.repeat || event.state != ElementState::Pressed {
            return false;
        }

        if modifiers.control_key() && modifiers.shift_key() {
            if let Key::Character(text) = &event.logical_key {
                if text.eq_ignore_ascii_case("f") {
                    self.search.active = true;
                    self.search.current_match = 0;
                    self.dismiss_menu();
                    self.request_full_capture();
                    self.needs_redraw = true;
                    self.request_redraw();
                    return true;
                }
            }
        }

        if !self.search.active {
            return false;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.search.active = false;
                self.search.current_match = 0;
                self.request_full_capture();
            }
            Key::Named(NamedKey::Backspace) => {
                self.search.query.pop();
                self.search.current_match = 0;
                self.request_full_capture();
            }
            Key::Named(NamedKey::Enter) if self.search.match_count > 0 => {
                if modifiers.shift_key() {
                    self.search.current_match = if self.search.current_match == 0 {
                        self.search.match_count - 1
                    } else {
                        self.search.current_match - 1
                    };
                } else {
                    self.search.current_match =
                        (self.search.current_match + 1) % self.search.match_count;
                }
                self.request_full_capture();
            }
            Key::Named(NamedKey::Enter) => {}
            Key::Character(text) if !modifiers.control_key() && !modifiers.alt_key() => {
                self.search.query.push_str(text);
                self.search.current_match = 0;
                self.request_full_capture();
            }
            _ => {}
        }

        self.needs_redraw = true;
        self.request_redraw();
        true
    }

    pub fn register_click(&mut self, x: f64, y: f64) -> u32 {
        let now = Instant::now();
        if now.duration_since(self.last_click_time) <= DOUBLE_CLICK_INTERVAL
            && (x - self.last_click_pos.0).abs() <= DOUBLE_CLICK_SLOP
            && (y - self.last_click_pos.1).abs() <= DOUBLE_CLICK_SLOP
        {
            self.click_count += 1;
        } else {
            self.click_count = 1;
        }
        self.last_click_time = now;
        self.last_click_pos = (x, y);
        self.click_count
    }

    pub fn start_selection_at(
        &mut self,
        x: f64,
        y: f64,
        selection_type: SelectionType,
        copy_immediately: bool,
    ) {
        if let Some(tab) = self.tabs.active_tab() {
            let mut term = tab.session.terminal.lock();
            let point = self.grid_point_from_position(term.grid().display_offset(), x, y);
            let side = self.side_from_position(x);
            term.selection = Some(Selection::new(selection_type, point, side));
        }
        if copy_immediately {
            self.copy_selection();
        }
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn bell_flash_active(&self) -> bool {
        self.bell_flash_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn terminal_content_x(&self, x: f64) -> f64 {
        x - f64::from(self.layout.content_offset_x)
    }

    fn terminal_content_y(&self, y: f64) -> f64 {
        y - f64::from(self.layout.content_offset_y)
    }

    pub fn is_in_tab_strip(&self, x: f64, y: f64) -> bool {
        x < f64::from(self.chrome_metrics.tab_strip.width)
            && y >= f64::from(self.chrome_metrics.title_bar.height)
    }

    pub fn is_in_title_bar(&self, _x: f64, y: f64) -> bool {
        y < f64::from(self.chrome_metrics.title_bar.height)
    }

    pub fn sidebar_divider_hit(&self, x: f64, y: f64) -> bool {
        const HANDLE: f64 = 4.0;
        if self.sidebar_collapsed {
            return false;
        }
        let divider = f64::from(self.chrome_metrics.tab_strip.width);
        y >= f64::from(self.chrome_metrics.title_bar.height) && (x - divider).abs() <= HANDLE
    }

    pub fn toggle_sidebar_collapsed(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.sidebar_dragging = false;
        self.sync_terminal_layout(config, event_loop);
    }

    pub fn apply_sidebar_width(
        &mut self,
        config: &Config,
        new_width: f32,
        event_loop: &ActiveEventLoop,
    ) {
        let (min_width, max_width) = sidebar_width_bounds(self.window.scale_factor() as f32);
        let clamped = new_width.clamp(min_width, max_width);
        if (clamped - self.sidebar_width).abs() < 0.5 {
            return;
        }
        self.sidebar_width = clamped;
        self.sync_terminal_layout(config, event_loop);
    }

    pub fn is_in_terminal(&self, x: f64, y: f64) -> bool {
        // Use the chrome content origin (right of the sidebar, below the title
        // bar) rather than `self.layout`, which in grid view is the focused
        // pane's rectangle and would reject clicks on the other panes.
        x >= f64::from(self.chrome_metrics.content_offset_x())
            && y >= f64::from(self.chrome_metrics.content_offset_y())
    }

    fn viewport_point_from_position(&self, x: f64, y: f64) -> Point<usize> {
        let x = self.terminal_content_x(x);
        let y = self.terminal_content_y(y);
        let col = (x / f64::from(self.layout.cell_width)).floor().max(0.0) as usize;
        let row = (y / f64::from(self.layout.cell_height)).floor().max(0.0) as usize;
        let max_row = self.layout.rows.saturating_sub(1) as usize;
        let max_col = self.layout.cols.saturating_sub(1) as usize;
        Point::new(row.min(max_row), Column(col.min(max_col)))
    }

    fn grid_point_from_position(&self, display_offset: usize, x: f64, y: f64) -> Point {
        let viewport = self.viewport_point_from_position(x, y);
        viewport_to_point(display_offset, viewport)
    }

    fn side_from_position(&self, x: f64) -> Side {
        let col = self.terminal_content_x(x) / f64::from(self.layout.cell_width);
        if col.fract() > 0.5 {
            Side::Right
        } else {
            Side::Left
        }
    }

    pub fn tab_bar_entries(&self) -> Vec<TabBarEntry> {
        let now = Instant::now();
        self.tabs
            .infos()
            .into_iter()
            .map(|info| TabBarEntry {
                id: info.id,
                title: info.title,
                cwd: info.cwd.as_deref().map(short_path),
                active: info.active,
                busy: info.running_since.is_some(),
                elapsed: info
                    .running_since
                    .map(|since| now.saturating_duration_since(since)),
            })
            .collect()
    }

    pub fn refresh_tab_activity(&mut self) -> bool {
        let now = Instant::now();
        let mut any_busy = false;
        let mut changed = false;

        for tab in self.tabs.iter_mut() {
            if tab.session.is_busy() {
                if tab.running_since.is_none() {
                    tab.running_since = Some(now);
                    changed = true;
                }
            } else if tab.running_since.take().is_some() {
                changed = true;
            }

            if tab.running_since.is_some() {
                any_busy = true;
            }
        }

        if changed {
            self.needs_redraw = true;
            self.mark_chrome_dirty();
            self.last_busy_elapsed_secs = None;
        }
        any_busy
    }

    /// Returns true when a busy-tab elapsed label needs a new paint.
    pub fn tick_busy_timer(&mut self) -> bool {
        let now = Instant::now();
        let max_secs = self
            .tabs
            .infos()
            .into_iter()
            .filter_map(|info| info.running_since)
            .map(|since| now.saturating_duration_since(since).as_secs())
            .max();

        let Some(secs) = max_secs else {
            self.last_busy_elapsed_secs = None;
            return false;
        };

        if self.last_busy_elapsed_secs == Some(secs) {
            return false;
        }
        self.last_busy_elapsed_secs = Some(secs);
        self.mark_chrome_dirty();
        true
    }

    pub fn handle_tab_strip_click(
        &mut self,
        config: &Config,
        x: f64,
        y: f64,
        button: MouseButton,
        event_loop: &ActiveEventLoop,
    ) -> TabStripAction {
        let layout = self.tab_strip_layout_snapshot();
        match layout.hit_test(x, y, self.tab_scroll_offset) {
            TabStripHit::Tab(id) => {
                if button == MouseButton::Left {
                    self.focus_tab(id, config);
                    TabStripAction::None
                } else if button == MouseButton::Middle && self.close_tab(id, config, event_loop) {
                    TabStripAction::WindowEmpty
                } else {
                    TabStripAction::None
                }
            }
            TabStripHit::Close(id) => {
                if (button == MouseButton::Left || button == MouseButton::Middle)
                    && self.close_tab(id, config, event_loop)
                {
                    TabStripAction::WindowEmpty
                } else {
                    TabStripAction::None
                }
            }
            TabStripHit::Detach(id) if button == MouseButton::Left => TabStripAction::Detach(id),
            TabStripHit::NewTab if button == MouseButton::Left => TabStripAction::NewTab,
            TabStripHit::ToggleCollapse if button == MouseButton::Left => {
                self.toggle_sidebar_collapsed(config, event_loop);
                TabStripAction::None
            }
            _ => TabStripAction::None,
        }
    }

    pub fn handle_title_bar_click(
        &mut self,
        _config: &Config,
        theme_mode: ThemeMode,
        x: f64,
        y: f64,
        button: MouseButton,
    ) -> bool {
        if button != MouseButton::Left {
            return false;
        }

        let Some(renderer) = self.renderer.as_ref() else {
            return false;
        };
        let Some(layout) = renderer.title_bar_layout() else {
            return false;
        };

        match layout.hit_test(x, y) {
            TitleBarHit::Close => return true,
            TitleBarHit::Hamburger => {
                self.menu_open = !self.menu_open;
                if self.menu_open {
                    self.refresh_menu_entries(theme_mode);
                } else {
                    self.mark_chrome_dirty();
                }
                self.needs_redraw = true;
                self.request_redraw();
            }
            TitleBarHit::Drag => {
                let _ = self.window.drag_window();
            }
            TitleBarHit::None => {}
        }
        false
    }

    pub fn handle_menu_action(
        &mut self,
        config: &Config,
        theme_mode: ThemeMode,
        action: MenuAction,
        event_loop: &ActiveEventLoop,
    ) -> Option<MenuAppAction> {
        let keep_open = matches!(
            action,
            MenuAction::ThemeLight
                | MenuAction::ThemeDark
                | MenuAction::ThemeAuto
                | MenuAction::ViewSingle
                | MenuAction::ViewGrid
                | MenuAction::TogglePerfOverlay
                | MenuAction::ZoomIn
                | MenuAction::ZoomOut
                | MenuAction::ZoomReset
        );
        if !keep_open {
            self.menu_open = false;
            self.mark_chrome_dirty();
        }

        let app_action = match action {
            MenuAction::NewTab => Some(MenuAppAction::NewTab),
            MenuAction::DuplicateTab => Some(MenuAppAction::DuplicateTab),
            MenuAction::RenameTab => {
                self.start_rename();
                None
            }
            MenuAction::CloseTab => Some(MenuAppAction::CloseTab),
            MenuAction::DetachTab => Some(MenuAppAction::DetachTab),
            MenuAction::Copy => {
                self.copy_selection();
                None
            }
            MenuAction::Paste => {
                self.paste_from_clipboard();
                None
            }
            MenuAction::ClearScrollback => {
                self.clear_scrollback();
                None
            }
            MenuAction::ThemeLight => Some(MenuAppAction::Theme(ThemeMode::Light)),
            MenuAction::ThemeDark => Some(MenuAppAction::Theme(ThemeMode::Dark)),
            MenuAction::ThemeAuto => Some(MenuAppAction::Theme(ThemeMode::Auto)),
            MenuAction::ViewSingle => {
                self.set_view_mode(config, ViewMode::Single, event_loop);
                None
            }
            MenuAction::ViewGrid => {
                self.set_view_mode(config, ViewMode::Grid, event_loop);
                None
            }
            MenuAction::TogglePerfOverlay => {
                self.perf_overlay_enabled = !self.perf_overlay_enabled;
                self.mark_chrome_dirty();
                self.needs_redraw = true;
                self.request_redraw();
                None
            }
            MenuAction::ZoomIn => {
                self.apply_font_zoom(config, self.font_zoom + FONT_ZOOM_STEP, event_loop);
                None
            }
            MenuAction::ZoomOut => {
                self.apply_font_zoom(config, self.font_zoom - FONT_ZOOM_STEP, event_loop);
                None
            }
            MenuAction::ZoomReset => {
                self.apply_font_zoom(config, 1.0, event_loop);
                None
            }
            MenuAction::Quit => Some(MenuAppAction::Quit),
        };

        if self.menu_open {
            self.refresh_menu_entries(theme_mode);
            self.needs_redraw = true;
            self.request_redraw();
        }

        app_action
    }

    pub fn resize_hit_at(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        let size = self.window.inner_size();
        let border = border_size(self.window.scale_factor());
        resize_direction_at(x, y, size.width as f64, size.height as f64, border)
    }

    pub fn update_resize_cursor(&self, x: f64, y: f64) {
        let icon = if self.sidebar_dragging || self.sidebar_divider_hit(x, y) {
            CursorIcon::ColResize
        } else {
            self.resize_hit_at(x, y)
                .map(cursor_for_direction)
                .unwrap_or(CursorIcon::Default)
        };
        self.window.set_cursor(icon);
    }

    pub fn start_resize_drag(&self, x: f64, y: f64) -> bool {
        let Some(direction) = self.resize_hit_at(x, y) else {
            return false;
        };
        self.window.drag_resize_window(direction).is_ok()
    }

    pub fn handle_key_action(
        &mut self,
        config: &Config,
        action: KeyAction,
        event_loop: &ActiveEventLoop,
    ) -> Option<MenuAppAction> {
        match action {
            KeyAction::NewTab => Some(MenuAppAction::NewTab),
            KeyAction::DuplicateTab => Some(MenuAppAction::DuplicateTab),
            KeyAction::RenameTab => {
                self.start_rename();
                None
            }
            KeyAction::CloseTab => Some(MenuAppAction::CloseTab),
            KeyAction::DetachTab => Some(MenuAppAction::DetachTab),
            KeyAction::NextTab => {
                self.tabs.next_tab();
                if let Some(id) = self.tabs.active_id() {
                    self.focus_tab(id, config);
                }
                None
            }
            KeyAction::PrevTab => {
                self.tabs.prev_tab();
                if let Some(id) = self.tabs.active_id() {
                    self.focus_tab(id, config);
                }
                None
            }
            KeyAction::SelectTab(number) => {
                self.tabs.select_tab_number(number);
                if let Some(id) = self.tabs.active_id() {
                    self.focus_tab(id, config);
                }
                None
            }
            KeyAction::SendToTerminal(bytes) => {
                if let Some(tab) = self.tabs.active_tab() {
                    tab.session.notifier.notify(Cow::Owned(bytes));
                }
                None
            }
            KeyAction::Copy => {
                self.copy_selection();
                None
            }
            KeyAction::Paste => {
                self.paste_from_clipboard();
                None
            }
            KeyAction::ZoomIn => {
                self.apply_font_zoom(config, self.font_zoom + FONT_ZOOM_STEP, event_loop);
                None
            }
            KeyAction::ZoomOut => {
                self.apply_font_zoom(config, self.font_zoom - FONT_ZOOM_STEP, event_loop);
                None
            }
            KeyAction::ZoomReset => {
                self.apply_font_zoom(config, 1.0, event_loop);
                None
            }
            KeyAction::ClearScrollback => {
                self.clear_scrollback();
                None
            }
        }
    }

    pub fn redraw(&mut self, config: &Config, theme_mode: ThemeMode) {
        let frame_start = Instant::now();
        self.ensure_renderer(config);

        if self.menu_open {
            self.refresh_menu_entries(theme_mode);
        }

        let chrome_changed = self.chrome_dirty;
        if self.chrome_dirty {
            self.cached_tab_entries = self.tab_bar_entries();
            self.cached_window_title = self.window_title(config);
            self.chrome_dirty = false;
        }

        let chrome_metrics = self.chrome_metrics;
        let menu_open = self.menu_open;
        let tab_scroll_offset = self.tab_scroll_offset;
        let window_height = self.window_height();
        let sidebar_collapsed = self.sidebar_collapsed;
        let bell_flash = self.bell_flash_active();
        let terminal_opacity = self.terminal_opacity;
        let force_full = self.force_full_capture;
        self.force_full_capture = false;

        if self.tabs.is_empty() {
            return;
        }
        if self.panes.is_empty() {
            self.recompute_panes(config);
        }
        if self.panes.is_empty() {
            return;
        }

        self.ensure_pane_buffers(self.panes.len());

        let focused_pane = self.focused_pane_slot();
        let capture_start = Instant::now();
        let mut full_panes = 0;
        let mut dirty_rows = 0;
        let search_query = (self.search.active && !self.search.query.is_empty())
            .then(|| self.search.query.clone());
        let mut search_match_count = 0;

        for (slot, pane) in self.panes.iter().enumerate() {
            let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
                continue;
            };
            let row_count = pane.layout.rows as usize;
            let col_count = pane.layout.cols as usize;
            let damage = {
                let mut term = tab.session.terminal.lock();
                capture_terminal_frame(
                    &mut self.pane_frame_bufs[slot],
                    &mut self.pane_row_scratch[slot],
                    &mut self.pane_frames[slot],
                    &mut term,
                    row_count,
                    col_count,
                    force_full,
                )
            };
            match &damage {
                FrameDamage::Full => {
                    full_panes += 1;
                    dirty_rows += row_count;
                }
                FrameDamage::Partial(lines) => {
                    dirty_rows += lines.len();
                }
                FrameDamage::None => {}
            }
            if let Some(query) = search_query.as_deref() {
                search_match_count +=
                    populate_search_matches(&mut self.pane_frames[slot], query, col_count);
            } else {
                self.pane_frames[slot].search_matches.clear();
            }

            let active_match =
                if slot == focused_pane && self.search.active && search_match_count > 0 {
                    Some(
                        self.search
                            .current_match
                            .min(search_match_count.saturating_sub(1)),
                    )
                } else {
                    None
                };
            self.pane_frames[slot].search_active_match = active_match;

            if slot == focused_pane && self.modifiers.control_key() {
                populate_link_hover(
                    &mut self.pane_frames[slot],
                    pane.layout.cols as usize,
                    self.cursor_position.0,
                    self.cursor_position.1,
                    pane.layout.content_offset_x,
                    pane.layout.content_offset_y,
                    pane.layout.cell_width,
                    pane.layout.cell_height,
                );
            } else {
                self.pane_frames[slot].link_hovers.clear();
            }

            self.pane_damage[slot] = damage;
        }
        self.search.match_count = search_match_count;
        if self.search.match_count == 0 || self.search.current_match >= self.search.match_count {
            self.search.current_match = 0;
        }
        let capture_elapsed = capture_start.elapsed();

        let terminal_changed =
            force_full || self.pane_damage.iter().any(|damage| !damage.is_unchanged());
        let bell_active = self.bell_flash_active();
        let link_hover_active = self.modifiers.control_key()
            && self.is_in_terminal(self.cursor_position.0, self.cursor_position.1);
        let overlays_active = menu_open
            || self.search.active
            || self.rename.active
            || self.command_palette.active
            || self.perf_overlay_enabled;
        let skip_render = !terminal_changed
            && !chrome_changed
            && !bell_active
            && !self.bell_flash_was_active
            && !overlays_active
            && !link_hover_active;

        let pane_count = self.panes.len();
        if skip_render {
            self.perf_stats.record(
                capture_elapsed,
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                frame_start.elapsed(),
                pane_count,
                full_panes,
                dirty_rows,
                true,
            );
            self.needs_redraw = false;
            return;
        }

        let pane_renders: Vec<PaneRender<'_>> = self
            .panes
            .iter()
            .enumerate()
            .filter_map(|(slot, pane)| {
                self.pane_frames.get(slot).map(|frame| PaneRender {
                    frame,
                    layout: pane.layout,
                    damage: &self.pane_damage[slot],
                })
            })
            .collect();

        let perf_stats = self.perf_stats;
        let perf_overlay = self.perf_overlay_enabled.then_some(&perf_stats);
        let search_overlay_snapshot = self.search.active.then(|| {
            (
                self.search.query.clone(),
                self.search.match_count,
                self.search.current_match,
            )
        });
        let rename_overlay_snapshot = self.rename.active.then(|| self.rename.draft.clone());
        let command_palette_snapshot = self.command_palette.active.then(|| {
            (
                self.command_palette.query.clone(),
                self.command_palette_labels(),
            )
        });
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let render_start = Instant::now();
        let search_overlay =
            search_overlay_snapshot
                .as_ref()
                .map(|(query, match_count, current_match)| SearchOverlayFrame {
                    query,
                    match_count: *match_count,
                    current_match: *current_match,
                });
        let rename_overlay = rename_overlay_snapshot
            .as_ref()
            .map(|draft| RenameOverlayFrame { draft });
        let command_palette =
            command_palette_snapshot
                .as_ref()
                .map(|(query, entries)| CommandPaletteFrame {
                    query,
                    entries: entries.as_slice(),
                });
        match renderer.render(
            &pane_renders,
            focused_pane,
            ChromeFrame {
                metrics: chrome_metrics,
                tab_entries: &self.cached_tab_entries,
                window_title: &self.cached_window_title,
                menu_open,
                menu_entries: if menu_open { &self.menu_entries } else { &[] },
                tab_scroll_offset,
                window_height,
                sidebar_collapsed,
                bell_flash,
                terminal_opacity,
                chrome_dirty: chrome_changed,
                perf_overlay,
                search_overlay,
                rename_overlay,
                command_palette,
            },
        ) {
            Ok(timings) => {
                self.perf_stats.record(
                    capture_elapsed,
                    timings.scene,
                    timings.gpu,
                    render_start.elapsed(),
                    pane_renders.len(),
                    full_panes,
                    dirty_rows,
                    false,
                );
            }
            Err(error) => tracing::error!(%error, "render failed"),
        }
        self.bell_flash_was_active = bell_active;

        self.needs_redraw = false;
    }

    pub fn invalidate_terminal_capture(&mut self) {
        self.request_full_capture();
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn sync_theme(&mut self, config: &Config) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_config(config.clone());
        }
        self.request_full_capture();
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn after_attach(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        self.ensure_renderer(config);
        self.sync_terminal_layout(config, event_loop);
        self.update_window_title(config);
    }

    pub fn write_preferences_to_config(&self, config: &mut Config) {
        config.appearance.opacity = self.terminal_opacity;
        config.ui.font_zoom = self.font_zoom;
        config.ui.sidebar_width = self.sidebar_width;
        config.ui.sidebar_collapsed = self.sidebar_collapsed;
        config.ui.view_mode = self.view_mode.into();
        config.session.cwd = self
            .tabs
            .infos()
            .into_iter()
            .filter_map(|info| info.cwd)
            .collect();
    }
}

#[derive(Clone, Copy)]
struct PaletteCommand {
    label: &'static str,
    action: MenuAction,
}

#[derive(Clone)]
enum PaletteItem {
    Command {
        label: &'static str,
        action: MenuAction,
    },
    Tab {
        id: TabId,
        title: String,
    },
}

fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "New Tab",
            action: MenuAction::NewTab,
        },
        PaletteCommand {
            label: "Duplicate Tab",
            action: MenuAction::DuplicateTab,
        },
        PaletteCommand {
            label: "Rename Tab",
            action: MenuAction::RenameTab,
        },
        PaletteCommand {
            label: "Close Tab",
            action: MenuAction::CloseTab,
        },
        PaletteCommand {
            label: "Move Tab to New Window",
            action: MenuAction::DetachTab,
        },
        PaletteCommand {
            label: "Copy",
            action: MenuAction::Copy,
        },
        PaletteCommand {
            label: "Paste",
            action: MenuAction::Paste,
        },
        PaletteCommand {
            label: "Clear Scrollback",
            action: MenuAction::ClearScrollback,
        },
        PaletteCommand {
            label: "Single Terminal View",
            action: MenuAction::ViewSingle,
        },
        PaletteCommand {
            label: "Grid View",
            action: MenuAction::ViewGrid,
        },
        PaletteCommand {
            label: "Toggle Perf Overlay",
            action: MenuAction::TogglePerfOverlay,
        },
        PaletteCommand {
            label: "Theme Light",
            action: MenuAction::ThemeLight,
        },
        PaletteCommand {
            label: "Theme Dark",
            action: MenuAction::ThemeDark,
        },
        PaletteCommand {
            label: "Theme Auto",
            action: MenuAction::ThemeAuto,
        },
        PaletteCommand {
            label: "Zoom In",
            action: MenuAction::ZoomIn,
        },
        PaletteCommand {
            label: "Zoom Out",
            action: MenuAction::ZoomOut,
        },
        PaletteCommand {
            label: "Reset Zoom",
            action: MenuAction::ZoomReset,
        },
        PaletteCommand {
            label: "Quit",
            action: MenuAction::Quit,
        },
    ]
}

fn short_path(path: &std::path::Path) -> String {
    let display = if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        path.strip_prefix(&home)
            .ok()
            .map(|rest| format!("~/{}", rest.display()))
            .unwrap_or_else(|| path.display().to_string())
    } else {
        path.display().to_string()
    };

    let chars: Vec<char> = display.chars().collect();
    if chars.len() <= 32 {
        display
    } else {
        format!(
            "...{}",
            chars[chars.len() - 29..].iter().collect::<String>()
        )
    }
}

fn url_span_in_row(
    cells: &[(Point, Cell)],
    col_count: usize,
    target_col: usize,
) -> Option<SearchMatch> {
    let mut chars = vec![' '; col_count];
    for (point, cell) in cells {
        let col = point.column.0;
        if col < col_count {
            chars[col] = cell.c;
        }
    }

    let row: String = chars.iter().collect();
    for scheme in ["https://", "http://", "file://"] {
        let mut search_from = 0;
        while let Some(offset) = row[search_from..].find(scheme) {
            let start = search_from + offset;
            let rest = &row[start..];
            let raw_len = rest
                .char_indices()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(idx, _)| idx)
                .unwrap_or(rest.len());
            let raw = &rest[..raw_len];
            let trimmed = raw.trim_end_matches(['.', ',', ')', ']']);
            let start_col = row[..start].chars().count();
            let end_col = start_col + trimmed.chars().count();
            if target_col >= start_col && target_col < end_col {
                return Some(SearchMatch {
                    row: 0,
                    start_col,
                    end_col,
                });
            }
            search_from = start + raw_len.max(1);
            if search_from >= row.len() {
                break;
            }
        }
    }

    None
}

fn populate_link_hover(
    frame: &mut TerminalFrame,
    col_count: usize,
    x: f64,
    y: f64,
    content_offset_x: f32,
    content_offset_y: f32,
    cell_width: f32,
    cell_height: f32,
) {
    frame.link_hovers.clear();
    let row = ((y - f64::from(content_offset_y)) / f64::from(cell_height))
        .floor()
        .max(0.0) as usize;
    let col = ((x - f64::from(content_offset_x)) / f64::from(cell_width))
        .floor()
        .max(0.0) as usize;

    let Some(cells) = frame.rows.get(row) else {
        return;
    };
    let Some(mut span) = url_span_in_row(cells, col_count, col) else {
        return;
    };
    span.row = row;
    frame.link_hovers.push(span);
}

fn url_in_row(cells: &[(Point, Cell)], col_count: usize, target_col: usize) -> Option<String> {
    let mut chars = vec![' '; col_count];
    for (point, cell) in cells {
        let col = point.column.0;
        if col < col_count {
            chars[col] = cell.c;
        }
    }

    let row: String = chars.iter().collect();
    for scheme in ["https://", "http://", "file://"] {
        let mut search_from = 0;
        while let Some(offset) = row[search_from..].find(scheme) {
            let start = search_from + offset;
            let rest = &row[start..];
            let raw_len = rest
                .char_indices()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(idx, _)| idx)
                .unwrap_or(rest.len());
            let raw = &rest[..raw_len];
            let trimmed = raw.trim_end_matches(['.', ',', ')', ']']);
            let start_col = row[..start].chars().count();
            let end_col = start_col + trimmed.chars().count();
            if target_col >= start_col && target_col < end_col {
                return Some(trimmed.to_owned());
            }
            search_from = start + raw_len.max(1);
            if search_from >= row.len() {
                break;
            }
        }
    }

    None
}

fn populate_search_matches(frame: &mut TerminalFrame, query: &str, col_count: usize) -> usize {
    frame.search_matches.clear();
    if query.is_empty() || col_count == 0 {
        return 0;
    }

    let query_chars: Vec<char> = query.chars().map(|ch| ch.to_ascii_lowercase()).collect();
    if query_chars.is_empty() {
        return 0;
    }

    let mut row_text = vec![' '; col_count];
    for (row, cells) in frame.rows.iter().enumerate() {
        row_text.fill(' ');
        for (point, cell) in cells {
            let col = point.column.0;
            if col < col_count {
                row_text[col] = cell.c.to_ascii_lowercase();
            }
        }

        if query_chars.len() > row_text.len() {
            continue;
        }

        for start in 0..=row_text.len() - query_chars.len() {
            if row_text[start..start + query_chars.len()] == query_chars {
                frame.search_matches.push(SearchMatch {
                    row,
                    start_col: start,
                    end_col: start + query_chars.len(),
                });
            }
        }
    }

    frame.search_matches.len()
}

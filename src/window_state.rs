use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TerminalEvent, EventListener, Notify, OnResize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{point_to_viewport, viewport_to_point, ClipboardType, Term, TermMode};
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::clipboard::{copy_primary, copy_text, paste_primary, paste_text, sanitize_paste};
use crate::config::{Config, ThemeMode, ViewModeConfig};
use crate::input::KeyAction;
use crate::keybindings::KeybindingsConfig;
use crate::links;
use crate::mouse;
use crate::pty::PtySession;
use crate::render::frame::{
    capture_terminal_frame, frame_has_blink_cells, merge_blink_damage, FrameDamage, SearchMatch,
    TerminalFrame, TerminalRowBuffer,
};
use crate::render::{
    border_size, collapsed_sidebar_width, cursor_for_direction, pane_rects, resize_direction_at,
    sidebar_width_bounds, split_dividers, ChromeFrame, ChromeMetrics, CommandPaletteFrame,
    ContentRect, MenuAction, MenuEntry, PaneRender, PerfStatsSnapshot, RenameOverlayFrame,
    Renderer, SearchOverlayFrame, SearchOverlayHit, SplitDividerHit, TabBarEntry, TabStripHit,
    TabStripLayout, TerminalLayout, TitleBarHit, MAX_GRID_PANES,
};
use crate::split::{spawn_leaf_session, SplitDirection};
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
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);
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
    pub leaf_id: usize,
    pub layout: TerminalLayout,
}

#[derive(Clone, Copy, Debug)]
struct GlobalSearchMatch {
    grid_line: i32,
    start_col: usize,
    end_col: usize,
}

#[derive(Default)]
struct SearchState {
    active: bool,
    query: String,
    match_count: usize,
    current_match: usize,
    global_matches: Vec<GlobalSearchMatch>,
    case_sensitive: bool,
    use_regex: bool,
    whole_word: bool,
    regex_error: bool,
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
    selected_index: usize,
}

struct SplitDragState {
    divider_index: usize,
    direction: SplitDirection,
    axis_span: f32,
    last_pos: (f64, f64),
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
    pub focused_pane: usize,
    split_drag: Option<SplitDragState>,
    pub modifiers: ModifiersState,
    pub pending_pty_layouts: Vec<(usize, usize, TerminalLayout)>,
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
    pub follow_output: bool,
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
    mouse_pressed: Option<MouseButton>,
    tab_drag_id: Option<TabId>,
    cursor_blink_visible: bool,
    cursor_blink_deadline: Option<Instant>,
    blink_phase_changed: bool,
    ime_composing: bool,
    ime_preedit: String,
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
    ToggleAlwaysOnTop,
    SwitchProfile(String),
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
        if config.window.decorations == crate::config::WindowDecorations::Native {
            chrome_metrics.title_bar.height = 0.0;
        }
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
            focused_pane: 0,
            split_drag: None,
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
            follow_output: config.ui.follow_output,
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
            mouse_pressed: None,
            tab_drag_id: None,
            cursor_blink_visible: true,
            cursor_blink_deadline: None,
            blink_phase_changed: false,
            ime_composing: false,
            ime_preedit: String::new(),
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
        if config.window.decorations == crate::config::WindowDecorations::Native {
            chrome_metrics.title_bar.height = 0.0;
        }
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
                leaf_id: 0,
                layout,
            }],
            focused_pane: 0,
            split_drag: None,
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
            follow_output: config.ui.follow_output,
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
            mouse_pressed: None,
            tab_drag_id: None,
            cursor_blink_visible: true,
            cursor_blink_deadline: None,
            blink_phase_changed: false,
            ime_composing: false,
            ime_preedit: String::new(),
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
        if config.window.decorations == crate::config::WindowDecorations::Native {
            metrics.title_bar.height = 0.0;
        }
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
        if self.panes.is_empty() {
            return 0;
        }
        self.focused_pane.min(self.panes.len() - 1)
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
                let content = ContentRect {
                    x0: content_x,
                    y0: content_y,
                    x1: content_x + content_w,
                    y1: content_y + content_h,
                };
                let mut leaf_counter = 0;
                let entries = self
                    .tabs
                    .tab_at_index(active)
                    .map(|tab| tab.root.layout_entries(content, &mut leaf_counter))
                    .unwrap_or_default();
                self.panes = entries
                    .iter()
                    .map(|entry| PaneSlot {
                        tab_index: active,
                        leaf_id: entry.leaf_id,
                        layout: TerminalLayout::from_rect(
                            entry.rect.x0,
                            entry.rect.y0,
                            entry.rect.width(),
                            entry.rect.height(),
                            scale,
                            font_size,
                        ),
                    })
                    .collect();
                if let Some(tab) = self.tabs.tab_at_index_mut(active) {
                    if tab.focused_leaf >= tab.leaf_count() {
                        tab.focused_leaf = 0;
                    }
                }
                self.layout = self
                    .panes
                    .iter()
                    .find(|pane| {
                        pane.tab_index == active
                            && self
                                .tabs
                                .tab_at_index(active)
                                .map(|tab| pane.leaf_id == tab.focused_leaf)
                                .unwrap_or(false)
                    })
                    .map(|pane| pane.layout)
                    .or_else(|| self.panes.first().map(|pane| pane.layout))
                    .unwrap_or(base);
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
                        leaf_id: 0,
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

        if self.focused_pane >= self.panes.len() {
            self.focused_pane = self.panes.len().saturating_sub(1);
        }

        self.ensure_pane_buffers(self.panes.len());

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(self.layout);
            renderer.invalidate_text_cache();
            renderer.sync_pane_count(self.panes.len());
        }
    }

    fn resize_all_pane_terminals(&mut self, config: &Config) -> bool {
        let _ = config;
        let mut any_grid_changed = false;

        for pane in &self.panes {
            let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) else {
                continue;
            };
            let Some(session) = tab.leaf_session_mut(pane.leaf_id) else {
                continue;
            };
            let mut term = session.terminal.lock();
            if term.columns() != pane.layout.cols as usize
                || term.screen_lines() != pane.layout.rows as usize
            {
                any_grid_changed = true;
                term.resize(pane.layout);
            }
        }

        if any_grid_changed {
            self.request_full_capture();
        }
        any_grid_changed
    }

    fn notify_all_pane_ptys(&mut self, config: &Config) {
        let _ = config;
        for pane in &self.panes {
            if let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) {
                if let Some(session) = tab.leaf_session_mut(pane.leaf_id) {
                    session.notifier.on_resize(pane.layout.window_size());
                }
            }
        }
    }

    pub fn apply_pane_layouts(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        if !self.resize_all_pane_terminals(config) {
            return;
        }

        self.pending_pty_layouts = self
            .panes
            .iter()
            .map(|pane| (pane.tab_index, pane.leaf_id, pane.layout))
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
        let leaf_id = pane.leaf_id;
        let layout = pane.layout;
        let Some(tab) = self.tabs.tab_at_index(tab_index) else {
            return;
        };
        let id = tab.id;
        if !self.tabs.set_active(id) {
            return;
        }
        if let Some(tab) = self.tabs.tab_at_index_mut(tab_index) {
            tab.focused_leaf = leaf_id;
        }
        self.focused_pane = slot;
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

    pub fn try_scrollbar_click(&mut self, x: f64, y: f64) -> bool {
        const TRACK_W: f64 = 5.0;
        let slot = match self.pane_at(x, y) {
            Some(slot) => slot,
            None => return false,
        };
        let pane = match self.panes.get(slot) {
            Some(pane) => pane,
            None => return false,
        };
        let layout = pane.layout;
        let width = f64::from(layout.cell_width) * f64::from(layout.cols);
        let height = f64::from(layout.cell_height) * f64::from(layout.rows);
        let x_off = f64::from(layout.content_offset_x);
        let y_off = f64::from(layout.content_offset_y);
        let track_x = x_off + width - TRACK_W - 1.0;
        if x < track_x || y < y_off || y >= y_off + height {
            return false;
        }

        let tab_index = pane.tab_index;
        let leaf_id = pane.leaf_id;
        let tab = match self.tabs.tab_at_index_mut(tab_index) {
            Some(tab) => tab,
            None => return false,
        };
        let session = match tab.leaf_session_mut(leaf_id) {
            Some(session) => session,
            None => return false,
        };
        let mut term = session.terminal.lock();
        let scroll_history = term
            .grid()
            .total_lines()
            .saturating_sub(term.grid().screen_lines());
        if scroll_history == 0 {
            return false;
        }

        let visible = layout.rows as f64;
        let total = visible + scroll_history as f64;
        let thumb_h = (height * (visible / total)).max(12.0);
        let track_h = (height - thumb_h).max(1.0);
        let frac = ((y - y_off) / track_h).clamp(0.0, 1.0);
        let target_offset = ((1.0 - frac) * scroll_history as f64).round() as i32;
        let current = term.grid().display_offset() as i32;
        let delta = target_offset - current;
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta));
        }
        drop(term);
        self.needs_redraw = true;
        self.request_redraw();
        true
    }

    pub fn tab_index_for_scroll(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        if self.view_mode == ViewMode::Grid {
            self.pane_at(x, y).and_then(|slot| {
                self.panes
                    .get(slot)
                    .map(|pane| (pane.tab_index, pane.leaf_id))
            })
        } else if self.tabs.is_empty() {
            None
        } else {
            let slot = self.pane_at(x, y).unwrap_or(self.focused_pane_slot());
            self.panes
                .get(slot)
                .map(|pane| (pane.tab_index, pane.leaf_id))
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

        links::url_at_position(&frame.rows, pane.layout.cols as usize, row, col)
    }

    pub fn open_url_at_position(&self, x: f64, y: f64) -> bool {
        let Some(url) = self.url_at_position(x, y) else {
            return false;
        };
        crate::platform::open_url(&url)
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
        tracing::info!(
            cols = self.layout.cols,
            rows = self.layout.rows,
            "spawning initial tab"
        );
        match spawn_leaf_session(config, id, 0, self.layout, &self.proxy_factory, first_cwd) {
            Ok(session) => {
                let tab = Tab::new(id, format!("Tab {}", id.0), session);
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
            .and_then(|tab| tab.current_working_directory());
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

    pub fn notify_pty_resize_pane(
        &mut self,
        tab_index: usize,
        leaf_id: usize,
        layout: TerminalLayout,
    ) {
        if let Some(tab) = self.tabs.tab_at_index_mut(tab_index) {
            if let Some(session) = tab.leaf_session_mut(leaf_id) {
                session.notifier.on_resize(layout.window_size());
            }
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
        for (tab_index, leaf_id, layout) in layouts {
            self.notify_pty_resize_pane(tab_index, leaf_id, layout);
        }

        self.needs_redraw = true;
        self.request_redraw();
        self.pty_resize_deadline = None;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    }

    pub fn handle_terminal_event(
        &mut self,
        config: &Config,
        tab_id: TabId,
        leaf_id: usize,
        event: TerminalEvent,
    ) {
        match event {
            TerminalEvent::Wakeup => {
                if self.follow_output {
                    if let Some(tab) = self.tabs.tab_by_id(tab_id) {
                        if let Some(session) = tab.leaf_session(leaf_id) {
                            let mut term = session.terminal.lock();
                            if term.grid().display_offset() == 0 {
                                term.scroll_display(Scroll::Bottom);
                            }
                        }
                    }
                }
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
                    if let Some(session) = tab.leaf_session(leaf_id) {
                        session.notifier.notify(Cow::Owned(data.into_bytes()));
                    }
                }
            }
            TerminalEvent::Bell => {
                if config.bell.visual {
                    self.bell_flash_until = Some(Instant::now() + BELL_FLASH_DURATION);
                    self.needs_redraw = true;
                    self.request_redraw();
                }
                if config.bell.audible {
                    crate::platform::play_audible_bell();
                }
                if config.bell.urgency {
                    self.window
                        .request_user_attention(Some(winit::window::UserAttentionType::Critical));
                }
            }
            TerminalEvent::CursorBlinkingChange => {
                self.reset_cursor_blink_timer();
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::ClipboardStore(clipboard_type, text) => {
                match clipboard_type {
                    ClipboardType::Clipboard => {
                        copy_text(&text);
                    }
                    ClipboardType::Selection => {
                        copy_primary(&text);
                    }
                }
            }
            TerminalEvent::ClipboardLoad(clipboard_type, respond) => {
                if let Some(tab) = self.tabs.tab_by_id(tab_id) {
                    if let Some(session) = tab.leaf_session(leaf_id) {
                        let mut text = match clipboard_type {
                            ClipboardType::Clipboard => paste_text(),
                            ClipboardType::Selection => paste_primary(),
                        }
                        .unwrap_or_default();
                        if config.terminal.sanitize_paste {
                            text = sanitize_paste(&text);
                        }
                        let response = respond(&text);
                        session
                            .notifier
                            .notify(Cow::Owned(response.into_bytes()));
                    }
                }
            }
            TerminalEvent::ColorRequest(index, respond) => {
                if let Some(tab) = self.tabs.tab_by_id(tab_id) {
                    if let Some(session) = tab.leaf_session(leaf_id) {
                        let term = session.terminal.lock();
                        let color =
                            crate::terminal_setup::resolve_color_at_index(&term, config, index);
                        drop(term);
                        let response = respond(color);
                        session
                            .notifier
                            .notify(Cow::Owned(response.into_bytes()));
                    }
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
        let Some(session) = tab.focused_session() else {
            return;
        };
        let term = session.terminal.lock();
        if let Some(text) = term.selection_to_string() {
            if !text.is_empty() {
                copy_text(&text);
                copy_primary(&text);
            }
        }
    }

    pub fn paste_from_clipboard(&mut self, config: &Config) {
        self.paste_text(config, paste_text());
    }

    pub fn paste_primary(&mut self, config: &Config) {
        self.paste_text(config, paste_primary());
    }

    fn paste_text(&mut self, config: &Config, text: Option<String>) {
        let Some(mut text) = text else {
            return;
        };
        if config.terminal.sanitize_paste {
            text = sanitize_paste(&text);
        }
        if text.is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.active_tab() {
            let payload = {
                let Some(session) = tab.focused_session() else {
                    return;
                };
                let term = session.terminal.lock();
                if term.mode().contains(TermMode::BRACKETED_PASTE) {
                    format!("\x1b[200~{text}\x1b[201~")
                } else {
                    text
                }
            };
            if let Some(session) = tab.focused_session() {
                session.notifier.notify(Cow::Owned(payload.into_bytes()));
            }
        }
    }

    pub fn terminal_input_state(&self) -> (TermMode, vte::ansi::ModifyOtherKeys, bool) {
        let (mode, modify_other_keys) = self
            .tabs
            .active_tab()
            .and_then(|tab| tab.focused_session())
            .map(|session| {
                (
                    *session.terminal.lock().mode(),
                    session.modify_other_keys(),
                )
            })
            .unwrap_or((TermMode::empty(), vte::ansi::ModifyOtherKeys::Reset));
        (mode, modify_other_keys, self.ime_composing)
    }

    pub fn send_bytes_to_pane(&self, tab_index: usize, leaf_id: usize, bytes: Vec<u8>) {
        if let Some(tab) = self.tabs.tab_at_index(tab_index) {
            if let Some(session) = tab.leaf_session(leaf_id) {
                session.notifier.notify(Cow::Owned(bytes));
            }
        }
    }

    fn layout_at_terminal(&self, x: f64, y: f64) -> Option<TerminalLayout> {
        if self.view_mode == ViewMode::Grid {
            self.pane_at(x, y)
                .and_then(|slot| self.panes.get(slot).map(|pane| pane.layout))
        } else {
            Some(self.layout)
        }
    }

    fn term_mode_for_pane(&self, tab_index: usize, leaf_id: usize) -> Option<TermMode> {
        let tab = self.tabs.tab_at_index(tab_index)?;
        let session = tab.leaf_session(leaf_id)?;
        Some(*session.terminal.lock().mode())
    }

    fn pane_at_cursor(&self, x: f64, y: f64) -> Option<(usize, usize, usize)> {
        let slot = self.pane_at(x, y)?;
        let pane = self.panes.get(slot)?;
        Some((slot, pane.tab_index, pane.leaf_id))
    }

    pub fn mouse_should_report(&self, x: f64, y: f64) -> bool {
        if self.modifiers.shift_key() {
            return false;
        }
        self.pane_at_cursor(x, y)
            .and_then(|(_, tab_index, leaf_id)| self.term_mode_for_pane(tab_index, leaf_id))
            .map(mouse::mouse_mode_active)
            .unwrap_or(false)
    }

    pub fn grid_point_at_terminal(&self, x: f64, y: f64) -> Option<(usize, usize, Point)> {
        let (_, tab_index, leaf_id) = self.pane_at_cursor(x, y)?;
        let layout = self.layout_at_terminal(x, y)?;
        let tab = self.tabs.tab_at_index(tab_index)?;
        let session = tab.leaf_session(leaf_id)?;
        let display_offset = session.terminal.lock().grid().display_offset();
        let point = mouse::grid_point_from_layout(&layout, display_offset, x, y);
        Some((tab_index, leaf_id, point))
    }

    pub fn try_report_mouse_wheel(&mut self, lines: i32, x: f64, y: f64) -> bool {
        if lines == 0 || self.modifiers.control_key() || !self.mouse_should_report(x, y) {
            return false;
        }
        let Some((tab_index, leaf_id, point)) = self.grid_point_at_terminal(x, y) else {
            return false;
        };
        let mode = self
            .term_mode_for_pane(tab_index, leaf_id)
            .unwrap_or(TermMode::empty());
        let button = mouse::wheel_button(lines);
        let bytes =
            mouse::encode_mouse_report(point, button, ElementState::Pressed, self.modifiers, mode);
        self.send_bytes_to_pane(tab_index, leaf_id, bytes);
        true
    }

    pub fn try_report_mouse_motion(&mut self, x: f64, y: f64) -> bool {
        if !self.mouse_should_report(x, y) {
            return false;
        }
        let Some((tab_index, leaf_id, point)) = self.grid_point_at_terminal(x, y) else {
            return false;
        };
        let mode = self
            .term_mode_for_pane(tab_index, leaf_id)
            .unwrap_or(TermMode::empty());
        let report_motion = mode.contains(TermMode::MOUSE_MOTION)
            || (mode.contains(TermMode::MOUSE_DRAG) && self.mouse_pressed.is_some());
        if !report_motion {
            return false;
        }
        let button = mouse::motion_button(self.mouse_pressed);
        let bytes =
            mouse::encode_mouse_report(point, button, ElementState::Pressed, self.modifiers, mode);
        self.send_bytes_to_pane(tab_index, leaf_id, bytes);
        true
    }

    pub fn try_report_mouse_button(
        &mut self,
        button: MouseButton,
        state: ElementState,
        x: f64,
        y: f64,
    ) -> bool {
        if !self.mouse_should_report(x, y) {
            return false;
        }
        let Some((tab_index, leaf_id, point)) = self.grid_point_at_terminal(x, y) else {
            return false;
        };
        let mode = self
            .term_mode_for_pane(tab_index, leaf_id)
            .unwrap_or(TermMode::empty());
        let code = mouse::button_code(button);
        let bytes = mouse::encode_mouse_report(point, code, state, self.modifiers, mode);
        self.send_bytes_to_pane(tab_index, leaf_id, bytes);
        match state {
            ElementState::Pressed => self.mouse_pressed = Some(button),
            ElementState::Released if self.mouse_pressed == Some(button) => {
                self.mouse_pressed = None;
            }
            _ => {}
        }
        true
    }

    pub fn handle_window_focus(&self, focused: bool) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let Some(session) = tab.focused_session() else {
            return;
        };
        let should_report = session
            .terminal
            .lock()
            .mode()
            .contains(TermMode::FOCUS_IN_OUT);
        if !should_report {
            return;
        }
        let bytes = if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        };
        session.notifier.notify(Cow::Owned(bytes));
    }

    pub fn handle_ime(&mut self, ime: winit::event::Ime) {
        use winit::event::Ime;

        match ime {
            Ime::Enabled => {
                self.ime_composing = true;
                self.ime_preedit.clear();
            }
            Ime::Preedit(text, _) => {
                self.ime_composing = true;
                self.ime_preedit = text;
                self.needs_redraw = true;
                self.request_redraw();
            }
            Ime::Commit(text) => {
                self.ime_composing = false;
                self.ime_preedit.clear();
                if let Some(tab) = self.tabs.active_tab() {
                    if let Some(session) = tab.focused_session() {
                        session.notifier.notify(Cow::Owned(text.into_bytes()));
                    }
                }
                self.needs_redraw = true;
                self.request_redraw();
            }
            Ime::Disabled => {
                self.ime_composing = false;
                self.ime_preedit.clear();
                self.needs_redraw = true;
                self.request_redraw();
            }
        }
    }

    pub fn clear_scrollback(&mut self) {
        if let Some(tab) = self.tabs.active_tab() {
            if let Some(session) = tab.focused_session() {
                let mut term = session.terminal.lock();
                term.grid_mut().clear_history();
                term.scroll_display(Scroll::Bottom);
            }
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

    pub fn build_menu_entries(&self, config: &Config, theme_mode: ThemeMode) -> Vec<MenuEntry> {
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
                action: MenuAction::ToggleTabPin,
                label: "Pin / Unpin Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::CycleTabColor,
                label: "Set Tab Color".to_owned(),
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
            MenuEntry::Action {
                action: MenuAction::ToggleFollowOutput,
                label: format!(
                    "Scroll on Output: {}",
                    if self.follow_output { "On" } else { "Off" }
                ),
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
            MenuEntry::Action {
                action: MenuAction::ToggleFullscreen,
                label: "Toggle Fullscreen".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::ToggleAlwaysOnTop,
                label: format!(
                    "Always on Top: {}",
                    if config.window.always_on_top {
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
    pub fn apply_slider_opacity(&mut self, config: &Config, theme_mode: ThemeMode, value: f32) {
        self.apply_terminal_opacity(value);
        if self.menu_open {
            self.refresh_menu_entries(config, theme_mode);
        }
    }

    pub fn refresh_menu_entries(&mut self, config: &Config, theme_mode: ThemeMode) {
        self.menu_entries = self.build_menu_entries(config, theme_mode);
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
                self.command_palette.selected_index = 0;
                self.clamp_palette_selection(config);
            }
            Key::Named(NamedKey::ArrowUp) => {
                let count = self.filtered_palette_items(config).len();
                if count > 0 {
                    self.command_palette.selected_index =
                        if self.command_palette.selected_index == 0 {
                            count - 1
                        } else {
                            self.command_palette.selected_index - 1
                        };
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                let count = self.filtered_palette_items(config).len();
                if count > 0 {
                    self.command_palette.selected_index =
                        (self.command_palette.selected_index + 1) % count;
                }
            }
            Key::Named(NamedKey::Enter) => {
                match self
                    .filtered_palette_items(config)
                    .get(self.command_palette.selected_index)
                {
                    Some(PaletteItem::Command { action, .. }) => {
                        self.command_palette.active = false;
                        app_action =
                            self.handle_menu_action(config, theme_mode, *action, event_loop);
                    }
                    Some(PaletteItem::Tab { id, .. }) => {
                        self.command_palette.active = false;
                        self.focus_tab(*id, config);
                    }
                    Some(PaletteItem::Profile { name }) => {
                        self.command_palette.active = false;
                        app_action = Some(MenuAppAction::SwitchProfile(name.clone()));
                    }
                    None => {}
                }
            }
            Key::Character(text) if !modifiers.control_key() && !modifiers.alt_key() => {
                self.command_palette.query.push_str(text);
                self.command_palette.selected_index = 0;
                self.clamp_palette_selection(config);
            }
            _ => {}
        }

        self.needs_redraw = true;
        self.request_redraw();
        (true, app_action)
    }

    fn clamp_palette_selection(&mut self, config: &Config) {
        let count = self.filtered_palette_items(config).len();
        if count == 0 {
            self.command_palette.selected_index = 0;
        } else if self.command_palette.selected_index >= count {
            self.command_palette.selected_index = count - 1;
        }
    }

    fn filtered_palette_items(&self, config: &Config) -> Vec<PaletteItem> {
        let query = self.command_palette.query.to_ascii_lowercase();
        let mut items = Vec::new();

        for name in &config.profile_names {
            let label = format!("Profile: {name}");
            if query.is_empty() || fuzzy_score(&query, &label).is_some() {
                items.push(PaletteItem::Profile {
                    name: name.clone(),
                });
            }
        }

        for info in self.tabs.infos() {
            let label = format!("Tab: {}", info.title);
            if query.is_empty() || fuzzy_score(&query, &label).is_some() {
                items.push(PaletteItem::Tab {
                    id: info.id,
                    title: info.title,
                });
            }
        }

        let mut commands: Vec<(i32, PaletteItem)> = Vec::new();
        for cmd in palette_commands() {
            if query.is_empty() {
                commands.push((
                    0,
                    PaletteItem::Command {
                        label: cmd.label,
                        action: cmd.action,
                    },
                ));
            } else if let Some(score) = fuzzy_score(&query, cmd.label) {
                commands.push((
                    score,
                    PaletteItem::Command {
                        label: cmd.label,
                        action: cmd.action,
                    },
                ));
            }
        }
        commands.sort_by(|a, b| b.0.cmp(&a.0));
        items.extend(commands.into_iter().map(|(_, item)| item));
        items
    }

    fn command_palette_labels(&self, config: &Config) -> Vec<String> {
        self.filtered_palette_items(config)
            .into_iter()
            .map(|item| match item {
                PaletteItem::Tab { title, .. } => format!("Tab: {title}"),
                PaletteItem::Profile { name } => format!("Profile: {name}"),
                PaletteItem::Command { label, action } => {
                    if let Some(hint) = menu_action_key_hint(action, &config.keybindings) {
                        format!("{label}    {hint}")
                    } else {
                        label.to_owned()
                    }
                }
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
                self.refresh_search_global_matches();
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
                self.scroll_to_current_search_match();
                self.request_full_capture();
            }
            Key::Named(NamedKey::Enter) => {}
            Key::Character(text)
                if (text == "c" || text == "C")
                    && ((modifiers.control_key() && modifiers.shift_key())
                        || (modifiers.alt_key() && !modifiers.control_key())) =>
            {
                self.search.case_sensitive = !self.search.case_sensitive;
                self.refresh_search_global_matches();
                self.request_full_capture();
            }
            Key::Character(text)
                if (text == "r" || text == "R")
                    && modifiers.alt_key()
                    && !modifiers.control_key() =>
            {
                self.search.use_regex = !self.search.use_regex;
                self.refresh_search_global_matches();
                self.request_full_capture();
            }
            Key::Character(text)
                if (text == "w" || text == "W")
                    && modifiers.alt_key()
                    && !modifiers.control_key() =>
            {
                self.search.whole_word = !self.search.whole_word;
                self.refresh_search_global_matches();
                self.request_full_capture();
            }
            Key::Character(text) if !modifiers.control_key() && !modifiers.alt_key() => {
                self.search.query.push_str(text);
                self.search.current_match = 0;
                self.refresh_search_global_matches();
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
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot).copied() else {
            return;
        };
        let layout = pane.layout;
        let (point, side) = {
            let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
                return;
            };
            let Some(session) = tab.leaf_session(pane.leaf_id) else {
                return;
            };
            let term = session.terminal.lock();
            let point = self.grid_point_from_layout(layout, term.grid().display_offset(), x, y);
            let side = self.side_from_layout(layout, x);
            (point, side)
        };
        if let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) {
            if let Some(session) = tab.leaf_session_mut(pane.leaf_id) {
                let mut term = session.terminal.lock();
                term.selection = Some(Selection::new(selection_type, point, side));
            }
        }
        if copy_immediately {
            self.copy_selection();
        }
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn update_selection_drag(&mut self, x: f64, y: f64) -> bool {
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot).copied() else {
            return false;
        };
        let layout = pane.layout;
        let (point, side) = {
            let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
                return false;
            };
            let Some(session) = tab.leaf_session(pane.leaf_id) else {
                return false;
            };
            let term = session.terminal.lock();
            let point = self.grid_point_from_layout(layout, term.grid().display_offset(), x, y);
            let side = self.side_from_layout(layout, x);
            (point, side)
        };
        if let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) {
            if let Some(session) = tab.leaf_session_mut(pane.leaf_id) {
                let mut term = session.terminal.lock();
                if let Some(selection) = term.selection.as_mut() {
                    selection.update(point, side);
                }
            }
        }
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
        true
    }

    pub fn begin_simple_selection(&mut self, x: f64, y: f64) {
        self.selecting = true;
        self.start_selection_at(x, y, SelectionType::Simple, false);
    }

    fn grid_point_from_layout(
        &self,
        layout: TerminalLayout,
        display_offset: usize,
        x: f64,
        y: f64,
    ) -> Point {
        let x = x - f64::from(layout.content_offset_x);
        let y = y - f64::from(layout.content_offset_y);
        let col = (x / f64::from(layout.cell_width)).floor().max(0.0) as usize;
        let row = (y / f64::from(layout.cell_height)).floor().max(0.0) as usize;
        let max_row = layout.rows.saturating_sub(1) as usize;
        let max_col = layout.cols.saturating_sub(1) as usize;
        let viewport = Point::new(row.min(max_row), Column(col.min(max_col)));
        viewport_to_point(display_offset, viewport)
    }

    fn side_from_layout(&self, layout: TerminalLayout, x: f64) -> Side {
        let col = (x - f64::from(layout.content_offset_x)) / f64::from(layout.cell_width);
        if col.fract() > 0.5 {
            Side::Right
        } else {
            Side::Left
        }
    }

    pub fn bell_flash_active(&self) -> bool {
        self.bell_flash_until
            .is_some_and(|until| Instant::now() < until)
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

    fn terminal_content_rect(&self, config: &Config) -> ContentRect {
        let base = self.terminal_layout_for_size(
            config,
            self.window.inner_size(),
            self.window.scale_factor(),
        );
        let size = self.window.inner_size();
        ContentRect {
            x0: base.content_offset_x,
            y0: base.content_offset_y,
            x1: size.width as f32,
            y1: size.height as f32,
        }
    }

    pub fn split_divider_hit(&self, config: &Config, x: f64, y: f64) -> Option<SplitDividerHit> {
        if self.view_mode != ViewMode::Single {
            return None;
        }
        let tab = self.tabs.active_tab()?;
        if tab.leaf_count() <= 1 {
            return None;
        }
        let rect = self.terminal_content_rect(config);
        let mut divider_index = 0;
        let dividers = split_dividers(rect, &tab.root, &mut divider_index);
        const HANDLE: f32 = 4.0;
        dividers.into_iter().find(|hit| {
            x as f32 >= hit.rect.x0 - HANDLE
                && x as f32 <= hit.rect.x1 + HANDLE
                && y as f32 >= hit.rect.y0 - HANDLE
                && y as f32 <= hit.rect.y1 + HANDLE
        })
    }

    pub fn start_split_divider_drag(&mut self, config: &Config, x: f64, y: f64) -> bool {
        let Some(hit) = self.split_divider_hit(config, x, y) else {
            return false;
        };
        self.split_drag = Some(SplitDragState {
            divider_index: hit.index,
            direction: hit.direction,
            axis_span: hit.axis_span,
            last_pos: (x, y),
        });
        true
    }

    pub fn apply_split_divider_drag(
        &mut self,
        config: &Config,
        x: f64,
        y: f64,
        event_loop: &ActiveEventLoop,
    ) {
        let Some(drag) = self.split_drag.as_mut() else {
            return;
        };
        let (dx, dy) = (x - drag.last_pos.0, y - drag.last_pos.1);
        drag.last_pos = (x, y);
        if drag.axis_span <= f32::EPSILON {
            return;
        }
        let delta_ratio = match drag.direction {
            SplitDirection::Horizontal => dx as f32 / drag.axis_span,
            SplitDirection::Vertical => dy as f32 / drag.axis_span,
        };
        if delta_ratio.abs() < f32::EPSILON {
            return;
        }
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.root
                .adjust_ratio_at_divider(drag.divider_index, delta_ratio);
            self.sync_terminal_layout(config, event_loop);
        }
    }

    pub fn end_split_divider_drag(&mut self) {
        self.split_drag = None;
    }

    pub fn split_dragging(&self) -> bool {
        self.split_drag.is_some()
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

    pub fn tab_bar_entries(&self) -> Vec<TabBarEntry> {
        let now = Instant::now();
        let mut entries: Vec<TabBarEntry> = self
            .tabs
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
                pinned: info.pinned,
                accent: info.accent,
            })
            .collect();
        entries.sort_by_key(|entry| (!entry.pinned, entry.id.0));
        entries
    }

    pub fn refresh_tab_activity(&mut self, heuristic_ms: u64) -> bool {
        let mut any_busy = false;
        let mut changed = false;

        for tab in self.tabs.iter_mut() {
            let before = tab.running_since();
            tab.refresh_running_since(heuristic_ms);
            let after = tab.running_since();
            if before != after {
                changed = true;
            }
            if after.is_some() {
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
                    self.tab_drag_id = Some(id);
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
        config: &Config,
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
            TitleBarHit::Minimize => {
                self.window.set_minimized(true);
            }
            TitleBarHit::Maximize => {
                self.window.set_maximized(!self.window.is_maximized());
            }
            TitleBarHit::Hamburger => {
                self.menu_open = !self.menu_open;
                if self.menu_open {
                    self.refresh_menu_entries(config, theme_mode);
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
                | MenuAction::ToggleFullscreen
                | MenuAction::ToggleFollowOutput
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
            MenuAction::ToggleTabPin => {
                if let Some(id) = self.tabs.active_id() {
                    self.toggle_tab_pin(id);
                }
                None
            }
            MenuAction::CycleTabColor => {
                if let Some(id) = self.tabs.active_id() {
                    self.cycle_tab_color(id);
                }
                None
            }
            MenuAction::ToggleAlwaysOnTop => Some(MenuAppAction::ToggleAlwaysOnTop),
            MenuAction::CloseTab => Some(MenuAppAction::CloseTab),
            MenuAction::DetachTab => Some(MenuAppAction::DetachTab),
            MenuAction::Copy => {
                self.copy_selection();
                None
            }
            MenuAction::Paste => {
                self.paste_from_clipboard(config);
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
            MenuAction::ToggleFullscreen => {
                self.toggle_fullscreen();
                None
            }
            MenuAction::ToggleFollowOutput => {
                self.follow_output = !self.follow_output;
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
            self.refresh_menu_entries(config, theme_mode);
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

    pub fn update_resize_cursor(&self, config: &Config, x: f64, y: f64) {
        let icon = if self.sidebar_dragging || self.sidebar_divider_hit(x, y) {
            CursorIcon::ColResize
        } else if let Some(direction) =
            self.split_drag
                .as_ref()
                .map(|drag| drag.direction)
                .or_else(|| {
                    self.split_divider_hit(config, x, y)
                        .map(|hit| hit.direction)
                })
        {
            match direction {
                SplitDirection::Horizontal => CursorIcon::ColResize,
                SplitDirection::Vertical => CursorIcon::RowResize,
            }
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
                let slot = self.focused_pane_slot();
                if let Some(pane) = self.panes.get(slot) {
                    self.send_bytes_to_pane(pane.tab_index, pane.leaf_id, bytes);
                }
                None
            }
            KeyAction::Copy => {
                self.copy_selection();
                None
            }
            KeyAction::Paste => {
                self.paste_from_clipboard(config);
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
            KeyAction::OpenSearch => {
                self.open_search();
                None
            }
            KeyAction::OpenCommandPalette => {
                self.open_command_palette();
                None
            }
            KeyAction::SplitVertical
            | KeyAction::SplitHorizontal
            | KeyAction::ClosePane
            | KeyAction::FocusPaneLeft
            | KeyAction::FocusPaneRight
            | KeyAction::FocusPaneUp
            | KeyAction::FocusPaneDown
            | KeyAction::ScrollPageUp
            | KeyAction::ScrollPageDown
            | KeyAction::JumpPromptPrev
            | KeyAction::JumpPromptNext
            | KeyAction::ToggleFullscreen => {
                self.handle_extended_key_action(config, action, event_loop);
                None
            }
        }
    }

    fn refresh_search_global_matches(&mut self) {
        self.search.global_matches.clear();
        self.search.match_count = 0;
        if !self.search.active || self.search.query.is_empty() {
            return;
        }
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot) else {
            return;
        };
        let col_count = pane.layout.cols as usize;
        let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
            return;
        };
        let Some(session) = tab.leaf_session(pane.leaf_id) else {
            return;
        };
        let term = session.terminal.lock();
        self.search.global_matches = search_scrollback(
            &term,
            &self.search.query,
            col_count,
            self.search.case_sensitive,
            self.search.use_regex,
            self.search.whole_word,
            &mut self.search.regex_error,
        );
        self.search.match_count = self.search.global_matches.len();
        if self.search.current_match >= self.search.match_count {
            self.search.current_match = 0;
        }
    }

    fn scroll_to_current_search_match(&mut self) {
        if self.search.match_count == 0 {
            return;
        }
        let grid_line = self.search.global_matches[self.search.current_match].grid_line;
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot) else {
            return;
        };
        let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) else {
            return;
        };
        let Some(session) = tab.leaf_session_mut(pane.leaf_id) else {
            return;
        };
        let mut term = session.terminal.lock();
        scroll_to_grid_line(&mut term, Line(grid_line));
    }

    pub fn open_search(&mut self) {
        self.search.active = true;
        self.search.current_match = 0;
        self.command_palette.active = false;
        self.dismiss_menu();
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn open_command_palette(&mut self) {
        self.command_palette.active = true;
        self.command_palette.query.clear();
        self.command_palette.selected_index = 0;
        self.search.active = false;
        self.dismiss_menu();
        self.needs_redraw = true;
        self.request_redraw();
    }

    /// Extended key actions for split panes, scroll, and fullscreen.
    pub fn handle_extended_key_action(
        &mut self,
        config: &Config,
        action: KeyAction,
        event_loop: &ActiveEventLoop,
    ) {
        match action {
            KeyAction::ScrollPageUp => self.scroll_terminal_page_up(),
            KeyAction::ScrollPageDown => self.scroll_terminal_page_down(),
            KeyAction::JumpPromptPrev => self.jump_to_previous_prompt(),
            KeyAction::JumpPromptNext => self.jump_to_next_prompt(),
            KeyAction::ToggleFullscreen => self.toggle_fullscreen(),
            KeyAction::SplitVertical => {
                self.split_focused_pane(config, SplitDirection::Horizontal, event_loop)
            }
            KeyAction::SplitHorizontal => {
                self.split_focused_pane(config, SplitDirection::Vertical, event_loop)
            }
            KeyAction::ClosePane => self.close_focused_pane(config, event_loop),
            KeyAction::FocusPaneLeft => self.focus_adjacent_pane(-1, 0, config),
            KeyAction::FocusPaneRight => self.focus_adjacent_pane(1, 0, config),
            KeyAction::FocusPaneUp => self.focus_adjacent_pane(0, -1, config),
            KeyAction::FocusPaneDown => self.focus_adjacent_pane(0, 1, config),
            _ => {}
        }
    }

    fn split_focused_pane(
        &mut self,
        config: &Config,
        direction: SplitDirection,
        event_loop: &ActiveEventLoop,
    ) {
        if self.view_mode == ViewMode::Grid {
            return;
        }
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot).copied() else {
            return;
        };
        let cwd = self
            .tabs
            .tab_at_index(pane.tab_index)
            .and_then(|tab| tab.leaf_session(pane.leaf_id))
            .and_then(|s| s.current_working_directory());
        let tab_id = self.tabs.tab_at_index(pane.tab_index).map(|t| t.id);
        let Some(tab_id) = tab_id else {
            return;
        };
        match spawn_leaf_session(
            config,
            tab_id,
            pane.leaf_id + 1,
            pane.layout,
            &self.proxy_factory,
            cwd,
        ) {
            Ok(session) => {
                if let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) {
                    if let Some(new_id) = tab.root.split_leaf(pane.leaf_id, direction, session) {
                        tab.focused_leaf = new_id;
                    }
                }
                self.sync_terminal_layout(config, event_loop);
            }
            Err(error) => tracing::error!(%error, "failed to spawn split pane"),
        }
    }

    fn close_focused_pane(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        if self.view_mode == ViewMode::Grid {
            return;
        }
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot).copied() else {
            return;
        };
        if let Some(tab) = self.tabs.tab_at_index(pane.tab_index) {
            if tab.leaf_count() <= 1 {
                if let Some(id) = self.tabs.tab_at_index(pane.tab_index).map(|t| t.id) {
                    let empty = self.close_tab(id, config, event_loop);
                    if empty {
                        return;
                    }
                }
            } else if let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) {
                tab.root.remove_leaf(pane.leaf_id);
                tab.focused_leaf = tab.focused_leaf.min(tab.leaf_count().saturating_sub(1));
            }
        }
        self.sync_terminal_layout(config, event_loop);
    }

    fn focus_adjacent_pane(&mut self, dx: i32, dy: i32, config: &Config) {
        let current = self.focused_pane_slot();
        let Some(current_pane) = self.panes.get(current) else {
            return;
        };
        let cur_bounds = current_pane.layout.pixel_bounds();
        let cur_cx = (cur_bounds.x0 + cur_bounds.x1) * 0.5;
        let cur_cy = (cur_bounds.y0 + cur_bounds.y1) * 0.5;
        let mut best: Option<(usize, f32)> = None;
        for (index, pane) in self.panes.iter().enumerate() {
            if index == current {
                continue;
            }
            let b = pane.layout.pixel_bounds();
            let cx = (b.x0 + b.x1) * 0.5;
            let cy = (b.y0 + b.y1) * 0.5;
            let ok = match (dx, dy) {
                (1, 0) => cx > cur_cx,
                (-1, 0) => cx < cur_cx,
                (0, 1) => cy > cur_cy,
                (0, -1) => cy < cur_cy,
                _ => false,
            };
            if !ok {
                continue;
            }
            let dist = (cx - cur_cx).powi(2) + (cy - cur_cy).powi(2);
            if best.map(|(_, d)| dist < d).unwrap_or(true) {
                best = Some((index, dist));
            }
        }
        if let Some((slot, _)) = best {
            self.focus_pane_slot(slot, config);
        }
    }

    pub fn scroll_terminal_page_up(&mut self) {
        let slot = self.focused_pane_slot();
        if let Some(pane) = self.panes.get(slot) {
            if let Some(tab) = self.tabs.tab_at_index(pane.tab_index) {
                if let Some(session) = tab.leaf_session(pane.leaf_id) {
                    let mut term = session.terminal.lock();
                    term.scroll_display(Scroll::PageUp);
                }
            }
        }
        self.invalidate_terminal_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn scroll_terminal_page_down(&mut self) {
        let slot = self.focused_pane_slot();
        if let Some(pane) = self.panes.get(slot) {
            if let Some(tab) = self.tabs.tab_at_index(pane.tab_index) {
                if let Some(session) = tab.leaf_session(pane.leaf_id) {
                    let mut term = session.terminal.lock();
                    term.scroll_display(Scroll::PageDown);
                }
            }
        }
        self.invalidate_terminal_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn jump_to_previous_prompt(&mut self) {
        self.jump_to_prompt(false);
    }

    pub fn jump_to_next_prompt(&mut self) {
        self.jump_to_prompt(true);
    }

    fn jump_to_prompt(&mut self, forward: bool) {
        let slot = self.focused_pane_slot();
        let Some(pane) = self.panes.get(slot) else {
            return;
        };
        let col_count = pane.layout.cols as usize;
        let Some(tab) = self.tabs.tab_at_index_mut(pane.tab_index) else {
            return;
        };
        let Some(session) = tab.leaf_session_mut(pane.leaf_id) else {
            return;
        };
        let mut term = session.terminal.lock();
        let grid = term.grid();
        let display_offset = grid.display_offset();
        let viewport_top = Line(-(display_offset as i32));
        let top = grid.topmost_line().0;
        let bottom = grid.bottommost_line().0;
        let lines: Vec<i32> = if forward {
            ((viewport_top.0 + 1)..=bottom).collect()
        } else {
            (top..viewport_top.0).rev().collect()
        };

        let mut row_text = vec![' '; col_count];
        for line in lines {
            row_text.fill(' ');
            for col in 0..col_count {
                row_text[col] = grid[Point::new(Line(line), Column(col))].c;
            }
            let text: String = row_text.iter().collect();
            if is_prompt_line(&text) {
                scroll_to_grid_line(&mut term, Line(line));
                break;
            }
        }
        drop(term);
        self.invalidate_terminal_capture();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn finish_tab_drag(&mut self, _x: f64, y: f64) {
        let Some(id) = self.tab_drag_id.take() else {
            return;
        };
        let layout = self.tab_strip_layout_snapshot();
        if y < layout.list_top || y >= layout.list_bottom() {
            return;
        }
        let content_y = (y - layout.list_top) + self.tab_scroll_offset;
        let row_h = self.chrome_metrics.tab_strip.row_height();
        if row_h <= 0.0 {
            return;
        }
        let index = (content_y / row_h).floor() as usize;
        if self.tabs.reorder_tab(id, index) {
            self.mark_chrome_dirty();
            self.needs_redraw = true;
            self.request_redraw();
        }
    }

    pub fn reset_cursor_blink_timer(&mut self) {
        self.cursor_blink_visible = true;
        self.cursor_blink_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
        self.blink_phase_changed = true;
    }

    pub fn tick_cursor_blink(&mut self) {
        let Some(deadline) = self.cursor_blink_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        self.cursor_blink_visible = !self.cursor_blink_visible;
        self.cursor_blink_deadline = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
        self.blink_phase_changed = true;
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn blink_wake_deadline(&self) -> Option<Instant> {
        self.cursor_blink_deadline
    }

    pub fn ensure_blink_timer(&mut self) {
        if self.cursor_blink_deadline.is_some() {
            return;
        }
        if self.any_blink_animation_needed() {
            self.reset_cursor_blink_timer();
        }
    }

    fn any_blink_animation_needed(&self) -> bool {
        for (slot, pane) in self.panes.iter().enumerate() {
            if self
                .pane_frames
                .get(slot)
                .is_some_and(frame_has_blink_cells)
            {
                return true;
            }
            let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
                continue;
            };
            let Some(session) = tab.leaf_session(pane.leaf_id) else {
                continue;
            };
            let term = session.terminal.lock();
            if term.cursor_style().blinking {
                return true;
            }
        }
        false
    }

    pub fn cursor_should_draw(&self, session: &PtySession) -> bool {
        let term = session.terminal.lock();
        if !term.mode().contains(TermMode::SHOW_CURSOR) {
            return false;
        }
        if term.cursor_style().blinking {
            self.cursor_blink_visible
        } else {
            true
        }
    }

    pub fn duplicate_tab(
        &mut self,
        config: &Config,
        next_tab_id: &mut usize,
        event_loop: &ActiveEventLoop,
    ) {
        let (title, snapshot, reported_cwd) = {
            let Some(tab) = self.tabs.active_tab() else {
                return;
            };
            let Some(session) = tab.focused_session() else {
                return;
            };
            let term = session.terminal.lock();
            let lines = crate::terminal_clone::capture_grid_lines(&term);
            let cwd = session.reported_cwd_handle().lock().clone();
            (format!("{} (copy)", tab.title), lines, cwd)
        };

        self.spawn_tab(config, next_tab_id, event_loop);

        if let Some(id) = self.tabs.active_id() {
            if let Some(tab) = self.tabs.tab_by_id_mut(id) {
                if let Some(session) = tab.focused_session_mut() {
                    if let Some(cwd) = reported_cwd {
                        *session.reported_cwd_handle().lock() = Some(cwd);
                    }
                    let mut term = session.terminal.lock();
                    crate::terminal_clone::replay_grid_lines(&mut term, &snapshot);
                }
            }
            self.tabs.set_title(id, title);
            self.update_window_title(config);
            self.mark_chrome_dirty();
            self.request_full_capture();
            self.needs_redraw = true;
            self.request_redraw();
        }
    }

    pub fn apply_config_reload(&mut self, config: &Config) {
        for tab in self.tabs.iter_mut() {
            for leaf in 0..tab.leaf_count() {
                if let Some(session) = tab.leaf_session_mut(leaf) {
                    let mut term = session.terminal.lock();
                    crate::terminal_setup::apply_config_to_term(config, &mut term);
                }
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.invalidate_text_cache();
            renderer.set_config(config.clone());
        }
        self.mark_chrome_dirty();
        self.request_full_capture();
        self.needs_redraw = true;
        self.request_redraw();
        if config.window.always_on_top {
            self.window
                .set_window_level(winit::window::WindowLevel::AlwaysOnTop);
        } else {
            self.window
                .set_window_level(winit::window::WindowLevel::Normal);
        }
    }

    pub fn toggle_always_on_top(&mut self, config: &mut Config) {
        config.window.always_on_top = !config.window.always_on_top;
        self.apply_config_reload(config);
    }

    pub fn toggle_tab_pin(&mut self, id: TabId) {
        self.tabs.toggle_pin(id);
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn cycle_tab_color(&mut self, id: TabId) {
        self.tabs.cycle_tab_color(id);
        self.mark_chrome_dirty();
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn toggle_fullscreen(&mut self) {
        use winit::window::Fullscreen;
        let entering = self.window.fullscreen().is_none();
        if entering {
            self.window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            self.window.set_fullscreen(None);
        }
        self.needs_redraw = true;
        self.request_redraw();
    }

    pub fn redraw(&mut self, config: &Config, theme_mode: ThemeMode) {
        self.tick_cursor_blink();
        let frame_start = Instant::now();
        self.ensure_renderer(config);

        if self.menu_open {
            self.refresh_menu_entries(config, theme_mode);
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
        if search_query.is_some() {
            self.refresh_search_global_matches();
        }
        let search_match_count = self.search.match_count;

        for (slot, pane) in self.panes.iter().enumerate() {
            let Some(tab) = self.tabs.tab_at_index(pane.tab_index) else {
                continue;
            };
            let row_count = pane.layout.rows as usize;
            let col_count = pane.layout.cols as usize;
            let damage = {
                let Some(session) = tab.leaf_session(pane.leaf_id) else {
                    continue;
                };
                let mut term = session.terminal.lock();
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
            let active_match =
                if slot == focused_pane && self.search.active && search_match_count > 0 {
                    let display_offset = self.pane_frames[slot].display_offset;
                    populate_search_matches_from_global(
                        &mut self.pane_frames[slot],
                        &self.search.global_matches,
                        self.search.current_match,
                        display_offset,
                        row_count,
                        col_count,
                    )
                } else {
                    self.pane_frames[slot].search_matches.clear();
                    None
                };
            self.pane_frames[slot].search_active_match = active_match;
            if let Some(session) = tab.leaf_session(pane.leaf_id) {
                self.pane_frames[slot].cursor_visible = self.cursor_should_draw(session);
            }
            self.pane_frames[slot].text_blink_visible = self.cursor_blink_visible;
            if self.blink_phase_changed {
                self.pane_damage[slot] = merge_blink_damage(
                    std::mem::take(&mut self.pane_damage[slot]),
                    &self.pane_frames[slot],
                    col_count,
                );
            }

            if slot == focused_pane && self.modifiers.control_key() {
                frame_link_hover_at_cursor(
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
        self.ensure_blink_timer();
        let blink_phase_changed = self.blink_phase_changed;
        self.blink_phase_changed = false;

        let terminal_changed = force_full
            || blink_phase_changed
            || self.pane_damage.iter().any(|damage| !damage.is_unchanged());
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

        let ime_preedit = (!self.ime_preedit.is_empty()).then(|| self.ime_preedit.as_str());
        let pane_renders: Vec<PaneRender<'_>> = self
            .panes
            .iter()
            .enumerate()
            .filter_map(|(slot, pane)| {
                self.pane_frames.get(slot).map(|frame| PaneRender {
                    frame,
                    layout: pane.layout,
                    damage: &self.pane_damage[slot],
                    ime_preedit: (slot == focused_pane).then_some(ime_preedit).flatten(),
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
                self.search.case_sensitive,
                self.search.use_regex,
                self.search.whole_word,
                self.search.regex_error,
            )
        });
        let rename_overlay_snapshot = self.rename.active.then(|| self.rename.draft.clone());
        let command_palette_snapshot = self.command_palette.active.then(|| {
            (
                self.command_palette.query.clone(),
                self.command_palette_labels(config),
                self.command_palette.selected_index,
            )
        });
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let render_start = Instant::now();
        let search_overlay = search_overlay_snapshot.as_ref().map(
            |(
                query,
                match_count,
                current_match,
                case_sensitive,
                use_regex,
                whole_word,
                regex_error,
            )| SearchOverlayFrame {
                query,
                match_count: *match_count,
                current_match: *current_match,
                case_sensitive: *case_sensitive,
                use_regex: *use_regex,
                whole_word: *whole_word,
                regex_error: *regex_error,
            },
        );
        let rename_overlay = rename_overlay_snapshot
            .as_ref()
            .map(|draft| RenameOverlayFrame { draft });
        let command_palette =
            command_palette_snapshot
                .as_ref()
                .map(|(query, entries, selected_index)| CommandPaletteFrame {
                    query,
                    entries: entries.as_slice(),
                    selected_index: *selected_index,
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
                if timings.presented {
                    self.needs_redraw = false;
                } else {
                    // The surface was occluded/outdated and the frame was
                    // dropped (e.g. another window covered ours). Keep the dirty
                    // flag and force a full recapture so the next paint restores
                    // the real content instead of leaving stale output. We do
                    // not call `request_redraw` here: the idle poll retries at a
                    // bounded rate, avoiding a tight repaint loop while hidden.
                    self.force_full_capture = true;
                    self.needs_redraw = true;
                }
            }
            Err(error) => {
                tracing::error!(%error, "render failed");
                self.needs_redraw = false;
            }
        }
        self.bell_flash_was_active = bell_active;
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
        config.ui.follow_output = self.follow_output;
        let scale = self.window.scale_factor();
        let size = self.window.inner_size();
        config.window.width = size.width as f64 / scale;
        config.window.height = size.height as f64 / scale;
        if let Ok(pos) = self.window.outer_position() {
            config.window.x = Some(pos.x);
            config.window.y = Some(pos.y);
        }
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
    Profile {
        name: String,
    },
}

fn fuzzy_score(query: &str, label: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().collect();
    let l: Vec<char> = label.to_ascii_lowercase().chars().collect();
    let mut qi = 0usize;
    let mut score = 0i32;
    let mut prev = 0usize;
    for (i, ch) in l.iter().enumerate() {
        if qi < q.len() && *ch == q[qi] {
            if qi > 0 {
                score += 10 - (i - prev).min(9) as i32;
            } else {
                score += 10 - i.min(9) as i32;
            }
            prev = i;
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

fn menu_action_key_hint(action: MenuAction, bindings: &KeybindingsConfig) -> Option<String> {
    menu_action_to_key_action(action)
        .and_then(|key_action| bindings.chord_label_for_action(key_action))
}

fn menu_action_to_key_action(action: MenuAction) -> Option<KeyAction> {
    match action {
        MenuAction::NewTab => Some(KeyAction::NewTab),
        MenuAction::DuplicateTab => Some(KeyAction::DuplicateTab),
        MenuAction::CloseTab => Some(KeyAction::CloseTab),
        MenuAction::DetachTab => Some(KeyAction::DetachTab),
        MenuAction::RenameTab => Some(KeyAction::RenameTab),
        MenuAction::Copy => Some(KeyAction::Copy),
        MenuAction::Paste => Some(KeyAction::Paste),
        MenuAction::ClearScrollback => Some(KeyAction::ClearScrollback),
        MenuAction::ZoomIn => Some(KeyAction::ZoomIn),
        MenuAction::ZoomOut => Some(KeyAction::ZoomOut),
        MenuAction::ZoomReset => Some(KeyAction::ZoomReset),
        MenuAction::ToggleFullscreen => Some(KeyAction::ToggleFullscreen),
        MenuAction::ToggleFollowOutput => Some(KeyAction::ScrollPageDown),
        MenuAction::TogglePerfOverlay => Some(KeyAction::OpenCommandPalette),
        MenuAction::ViewSingle | MenuAction::ViewGrid => Some(KeyAction::OpenCommandPalette),
        MenuAction::ThemeLight | MenuAction::ThemeDark | MenuAction::ThemeAuto => {
            Some(KeyAction::OpenCommandPalette)
        }
        MenuAction::ToggleTabPin | MenuAction::CycleTabColor | MenuAction::ToggleAlwaysOnTop => {
            None
        }
        MenuAction::Quit => Some(KeyAction::CloseTab),
    }
}

fn is_prompt_line(text: &str) -> bool {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.ends_with(['$', '#', '%', '❯', '➜', 'λ', '»'])
        || trimmed.ends_with('>')
        || trimmed.contains("$ ")
        || trimmed.contains("# ")
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
            label: "Toggle Scroll on Output",
            action: MenuAction::ToggleFollowOutput,
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
            label: "Toggle Fullscreen",
            action: MenuAction::ToggleFullscreen,
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
            label: "Pin / Unpin Tab",
            action: MenuAction::ToggleTabPin,
        },
        PaletteCommand {
            label: "Set Tab Color",
            action: MenuAction::CycleTabColor,
        },
        PaletteCommand {
            label: "Always on Top",
            action: MenuAction::ToggleAlwaysOnTop,
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

fn frame_link_hover_at_cursor(
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

    if frame.rows.get(row).is_none() {
        return;
    }
    let Some(link) = links::detect_link_at(&frame.rows, col_count, row, col) else {
        return;
    };
    frame.link_hovers.extend(link.spans);
}

fn search_scrollback<T: EventListener>(
    term: &Term<T>,
    query: &str,
    col_count: usize,
    case_sensitive: bool,
    use_regex: bool,
    whole_word: bool,
    regex_error: &mut bool,
) -> Vec<GlobalSearchMatch> {
    *regex_error = false;
    if query.is_empty() || col_count == 0 {
        return Vec::new();
    }

    let grid = term.grid();
    let top = grid.topmost_line().0;
    let bottom = grid.bottommost_line().0;
    let mut matches = Vec::new();
    let mut row_text = vec![' '; col_count];

    if use_regex {
        let pattern = if whole_word {
            format!(r"\b{query}\b")
        } else {
            query.to_owned()
        };
        let mut builder = regex::RegexBuilder::new(&pattern);
        if !case_sensitive {
            builder.case_insensitive(true);
        }
        let Ok(re) = builder.build() else {
            *regex_error = true;
            return Vec::new();
        };

        for line in top..=bottom {
            row_text.fill(' ');
            for col in 0..col_count {
                row_text[col] = grid[Point::new(Line(line), Column(col))].c;
            }
            let line_str: String = row_text.iter().collect();
            for m in re.find_iter(&line_str) {
                matches.push(GlobalSearchMatch {
                    grid_line: line,
                    start_col: m.start(),
                    end_col: m.end(),
                });
            }
        }
        return matches;
    }

    let query_chars: Vec<char> = if case_sensitive {
        query.chars().collect()
    } else {
        query.chars().map(|ch| ch.to_ascii_lowercase()).collect()
    };

    for line in top..=bottom {
        row_text.fill(' ');
        for col in 0..col_count {
            let cell = &grid[Point::new(Line(line), Column(col))];
            row_text[col] = if case_sensitive {
                cell.c
            } else {
                cell.c.to_ascii_lowercase()
            };
        }
        if query_chars.len() > row_text.len() {
            continue;
        }
        for start in 0..=row_text.len() - query_chars.len() {
            if row_text[start..start + query_chars.len()] != query_chars {
                continue;
            }
            if whole_word && !word_boundary_match(&row_text, start, query_chars.len()) {
                continue;
            }
            matches.push(GlobalSearchMatch {
                grid_line: line,
                start_col: start,
                end_col: start + query_chars.len(),
            });
        }
    }
    matches
}

fn word_boundary_match(row: &[char], start: usize, len: usize) -> bool {
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let before_ok = start == 0 || !is_word(row[start - 1]);
    let end = start + len;
    let after_ok = end >= row.len() || !is_word(row[end]);
    before_ok && after_ok
}

fn scroll_to_grid_line<T: EventListener>(term: &mut Term<T>, line: Line) {
    let grid = term.grid();
    let display_offset = grid.display_offset() as i32;
    let viewport_top = Line(-display_offset);
    let viewport_bottom = Line(viewport_top.0 + grid.screen_lines() as i32 - 1);
    if line < viewport_top {
        let delta: i32 = viewport_top.0 - line.0;
        term.scroll_display(Scroll::Delta(-delta));
    } else if line > viewport_bottom {
        let delta: i32 = line.0 - viewport_bottom.0;
        term.scroll_display(Scroll::Delta(delta));
    }
}

fn populate_search_matches_from_global(
    frame: &mut TerminalFrame,
    matches: &[GlobalSearchMatch],
    current_match: usize,
    display_offset: usize,
    row_count: usize,
    col_count: usize,
) -> Option<usize> {
    frame.search_matches.clear();
    let mut active_viewport = None;
    for (index, m) in matches.iter().enumerate() {
        let line = Line(m.grid_line);
        let Some(viewport) = point_to_viewport(display_offset, Point::new(line, Column(0))) else {
            continue;
        };
        let row = viewport.line;
        if row >= row_count {
            continue;
        }
        let match_index = frame.search_matches.len();
        frame.search_matches.push(SearchMatch {
            row,
            start_col: m.start_col.min(col_count),
            end_col: m.end_col.min(col_count),
        });
        if index == current_match {
            active_viewport = Some(match_index);
        }
    }
    active_viewport
}

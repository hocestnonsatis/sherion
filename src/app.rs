use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TerminalEvent, Notify, OnResize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::viewport_to_point;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowId};

use crate::clipboard::{copy_text, paste_text};
use crate::config::{Config, ThemeMode};
use crate::event::UserEvent;
use crate::input::{key_event_to_action, KeyAction};
use crate::pty::PtySession;
use crate::render::{
    border_size, cursor_for_direction, resize_direction_at, ChromeFrame, ChromeMetrics,
    MenuAction, MenuEntry, Renderer, TabBarEntry, TabStripHit, TabStripLayout, TerminalLayout,
    TitleBarHit, SIDEBAR_COLLAPSED_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH,
};
use crate::tabs::{EventLoopProxyFactory, Tab, TabId, TabManager};

const INITIAL_WIDTH: u32 = 800;
const INITIAL_HEIGHT: u32 = 600;
const PTY_RESIZE_DEBOUNCE: Duration = Duration::from_millis(150);
/// How long the window size must stay stable before we spawn the first tab. Tiling window
/// managers resize the window shortly after creation; spawning too early would give the PTY a
/// stale (small) column count and startup programs like fastfetch would truncate their output.
const STARTUP_SETTLE: Duration = Duration::from_millis(120);
const BELL_FLASH_DURATION: Duration = Duration::from_millis(150);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(300);
const DOUBLE_CLICK_SLOP: f64 = 5.0;
const FONT_ZOOM_MIN: f32 = 0.5;
const FONT_ZOOM_MAX: f32 = 3.0;
const FONT_ZOOM_STEP: f32 = 0.1;
const OPACITY_MIN: f32 = 0.2;
const OPACITY_MAX: f32 = 1.0;
const OPACITY_STEP: f32 = 0.05;

pub struct App {
    config: Config,
    proxy: Option<EventLoopProxy<UserEvent>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    tabs: Option<TabManager>,
    proxy_factory: Option<EventLoopProxyFactory>,
    chrome_metrics: ChromeMetrics,
    layout: TerminalLayout,
    modifiers: ModifiersState,
    pending_pty_layout: Option<TerminalLayout>,
    pty_resize_deadline: Option<Instant>,
    needs_redraw: bool,
    selecting: bool,
    menu_open: bool,
    tab_scroll_offset: f64,
    cursor_position: (f64, f64),
    initial_tab_pending: bool,
    startup_deadline: Option<Instant>,
    sidebar_width: f32,
    sidebar_dragging: bool,
    sidebar_collapsed: bool,
    font_zoom: f32,
    theme_mode: ThemeMode,
    terminal_opacity: f32,
    menu_entries: Vec<MenuEntry>,
    bell_flash_until: Option<Instant>,
    last_click_time: Instant,
    last_click_pos: (f64, f64),
    click_count: u32,
}

impl App {
    pub fn run(config: Config) -> Result<()> {
        let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let chrome_metrics = ChromeMetrics::from_font_size(config.font.size, 1.0);
        let sidebar_width = chrome_metrics.tab_strip.width;

        let layout = TerminalLayout::from_pixels(
            PhysicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT),
            1.0,
            config.font.size,
            chrome_metrics.content_offset_x(),
            chrome_metrics.content_offset_y(),
        );

        let theme_mode = config.appearance.theme;
        let terminal_opacity = config.appearance.opacity.clamp(OPACITY_MIN, OPACITY_MAX);

        let mut app = Self {
            config,
            proxy: Some(proxy),
            window: None,
            renderer: None,
            tabs: None,
            proxy_factory: None,
            chrome_metrics,
            layout,
            modifiers: ModifiersState::empty(),
            pending_pty_layout: None,
            pty_resize_deadline: None,
            needs_redraw: true,
            selecting: false,
            menu_open: false,
            tab_scroll_offset: 0.0,
            cursor_position: (0.0, 0.0),
            initial_tab_pending: true,
            startup_deadline: None,
            sidebar_width,
            sidebar_dragging: false,
            sidebar_collapsed: false,
            font_zoom: 1.0,
            theme_mode,
            terminal_opacity,
            menu_entries: Vec::new(),
            bell_flash_until: None,
            last_click_time: Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
        };

        event_loop.run_app(&mut app)?;
        Ok(())
    }

    fn effective_font_size(&self) -> f32 {
        self.config.font.size * self.font_zoom
    }

    fn chrome_metrics_for_window(&self, window: &Window) -> ChromeMetrics {
        let mut metrics = ChromeMetrics::from_font_size(
            self.effective_font_size(),
            window.scale_factor() as f32,
        );
        metrics.tab_strip.width = if self.sidebar_collapsed {
            SIDEBAR_COLLAPSED_WIDTH
        } else {
            self.sidebar_width
        };
        metrics
    }

    fn window_height(&self) -> f64 {
        self.window
            .as_ref()
            .map(|window| window.inner_size().height as f64)
            .unwrap_or(f64::from(INITIAL_HEIGHT))
    }

    fn terminal_layout_for_size(&self, size: PhysicalSize<u32>, scale: f64) -> TerminalLayout {
        TerminalLayout::from_pixels(
            size,
            scale,
            self.effective_font_size(),
            self.chrome_metrics.content_offset_x(),
            self.chrome_metrics.content_offset_y(),
        )
    }

    fn tab_strip_layout_snapshot(&self) -> TabStripLayout {
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

    fn clamp_tab_scroll(&mut self) {
        let max = self.tab_strip_layout_snapshot().max_scroll_offset();
        self.tab_scroll_offset = self.tab_scroll_offset.clamp(0.0, max);
    }

    fn scroll_tab_strip(&mut self, delta_lines: i32) {
        let delta = -f64::from(delta_lines) * self.chrome_metrics.tab_strip.row_height();
        self.tab_scroll_offset += delta;
        self.clamp_tab_scroll();
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn ensure_active_tab_visible(&mut self) {
        let layout = self.tab_strip_layout_snapshot();
        let Some(active_id) = self.tabs.as_ref().and_then(|tabs| tabs.active_id()) else {
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

    fn maybe_spawn_initial_tab(&mut self, event_loop: &ActiveEventLoop) {
        if !self.initial_tab_pending {
            return;
        }
        let Some(deadline) = self.startup_deadline else {
            return;
        };
        if self.window.is_none() || self.proxy_factory.is_none() {
            return;
        }
        if Instant::now() < deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }

        // Recompute layout from the now-settled window size before spawning.
        self.ensure_renderer();
        if let Some(window) = self.window.clone() {
            let size = window.inner_size();
            let scale = window.scale_factor();
            self.chrome_metrics = self.chrome_metrics_for_window(&window);
            self.layout = self.terminal_layout_for_size(size, scale);
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_layout(self.layout);
            }
        }

        self.initial_tab_pending = false;
        self.startup_deadline = None;
        event_loop.set_control_flow(ControlFlow::Wait);
        self.init_tabs();
        self.request_redraw();
    }

    fn init_tabs(&mut self) {
        if self.tabs.is_some() {
            return;
        }

        let Some(proxy_factory) = self.proxy_factory.clone() else {
            return;
        };

        let id = TabId(0);
        let event_proxy = proxy_factory.for_tab(id);
        tracing::info!(cols = self.layout.cols, rows = self.layout.rows, "spawning initial tab");
        match PtySession::spawn(&self.config, self.layout, event_proxy) {
            Ok(session) => {
                let tab = Tab {
                    id,
                    title: "Tab 0".to_owned(),
                    session,
                };
                self.tabs = Some(TabManager::with_initial(tab));
            }
            Err(error) => {
                tracing::error!(%error, "failed to spawn initial tab");
            }
        }
    }

    fn ensure_renderer(&mut self) {
        if self.renderer.is_some() {
            return;
        }

        let Some(window) = self.window.as_ref() else {
            return;
        };

        self.chrome_metrics = self.chrome_metrics_for_window(window);
        self.layout = self.terminal_layout_for_size(
            window.inner_size(),
            window.scale_factor(),
        );

        tracing::info!("initializing GPU renderer");
        match Renderer::new(Arc::clone(window), self.layout, self.config.clone()) {
            Ok(mut renderer) => {
                renderer.resize(window.inner_size());
                let _ = renderer.present_clear();
                self.renderer = Some(renderer);
                self.resize_terminal_grids(self.layout);
            }
            Err(error) => {
                tracing::error!(%error, "failed to initialize renderer");
            }
        }
    }

    fn spawn_tab(&mut self) {
        let Some(tabs) = self.tabs.as_mut() else {
            return;
        };
        let Some(proxy_factory) = self.proxy_factory.clone() else {
            return;
        };

        if tabs
            .spawn_tab(&self.config, self.layout, proxy_factory)
            .is_some()
        {
            self.ensure_active_tab_visible();
            self.needs_redraw = true;
            self.request_redraw();
        }
    }

    fn close_active_tab(&mut self, event_loop: &ActiveEventLoop) {
        let Some(active_id) = self.tabs.as_ref().and_then(|tabs| tabs.active_id()) else {
            return;
        };
        self.close_tab(active_id, event_loop);
    }

    fn close_tab(&mut self, id: TabId, event_loop: &ActiveEventLoop) {
        let Some(tabs) = self.tabs.as_mut() else {
            return;
        };

        tabs.close_tab(id);

        if tabs.is_empty() {
            self.tabs = None;
            event_loop.exit();
            return;
        }

        self.clamp_tab_scroll();
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn handle_tab_shell_exit(&mut self, tab_id: TabId, event_loop: &ActiveEventLoop) {
        tracing::info!(tab_id = tab_id.0, "shell exited");
        self.close_tab(tab_id, event_loop);
    }

    fn resize_terminal_grids(&mut self, layout: TerminalLayout) {
        let Some(tabs) = self.tabs.as_mut() else {
            return;
        };

        for tab in tabs.iter_mut() {
            let mut term = tab.session.terminal.lock();
            term.resize(layout);
            term.reset_damage();
        }
    }

    fn notify_pty_resize(&mut self, layout: TerminalLayout) {
        let Some(tabs) = self.tabs.as_mut() else {
            return;
        };

        let window_size = layout.window_size();
        for tab in tabs.iter_mut() {
            tab.session.notifier.on_resize(window_size);
        }
    }

    fn handle_window_resize(&mut self, size: PhysicalSize<u32>, event_loop: &ActiveEventLoop) {
        self.ensure_renderer();

        let snapshot = if let Some(window) = self.window.as_ref() {
            self.chrome_metrics = self.chrome_metrics_for_window(window);
            self.terminal_layout_for_size(size, window.scale_factor())
        } else {
            TerminalLayout::from_pixels(
                size,
                1.0,
                self.effective_font_size(),
                self.chrome_metrics.content_offset_x(),
                self.chrome_metrics.content_offset_y(),
            )
        };

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
            renderer.set_layout(snapshot);
        }

        if self.layout.grid_dims_changed(snapshot) {
            tracing::debug!(
                old_cols = self.layout.cols,
                new_cols = snapshot.cols,
                "grid dims changed"
            );
            // Keep the visible grid in sync immediately; debounce only SIGWINCH to the shell.
            self.resize_terminal_grids(snapshot);
            self.pending_pty_layout = Some(snapshot);
            self.schedule_pty_resize(event_loop);
        }

        self.layout = snapshot;
        self.clamp_tab_scroll();

        // While waiting to spawn the first tab, keep deferring until resizes stop so the PTY
        // gets the final, settled column count.
        if self.initial_tab_pending {
            let deadline = Instant::now() + STARTUP_SETTLE;
            self.startup_deadline = Some(deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }

        self.needs_redraw = true;
        self.request_redraw();
    }

    fn schedule_pty_resize(&mut self, event_loop: &ActiveEventLoop) {
        if self.pending_pty_layout.is_none() {
            return;
        }
        let deadline = Instant::now() + PTY_RESIZE_DEBOUNCE;
        self.pty_resize_deadline = Some(deadline);
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }

    fn flush_pty_resize(&mut self, event_loop: &ActiveEventLoop) {
        let Some(deadline) = self.pty_resize_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }

        if let Some(layout) = self.pending_pty_layout.take() {
            self.notify_pty_resize(layout);
            self.needs_redraw = true;
            self.request_redraw();
        }

        self.pty_resize_deadline = None;
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn handle_terminal_event(&mut self, tab_id: TabId, event: TerminalEvent) {
        match event {
            TerminalEvent::Wakeup => {
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::Title(title) => {
                if let Some(tabs) = self.tabs.as_mut() {
                    tabs.set_title(tab_id, title);
                }
                self.update_window_title();
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::ResetTitle => {
                if let Some(tabs) = self.tabs.as_mut() {
                    let fallback = format!("Tab {}", tab_id.0);
                    tabs.reset_title(tab_id, &fallback);
                }
                self.update_window_title();
                self.needs_redraw = true;
                self.request_redraw();
            }
            TerminalEvent::PtyWrite(data) => {
                if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.tab_by_id(tab_id)) {
                    tab.session.notifier.notify(Cow::Owned(data.into_bytes()));
                }
            }
            TerminalEvent::Bell => {
                if self.config.bell.visual {
                    self.bell_flash_until =
                        Some(Instant::now() + BELL_FLASH_DURATION);
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            TerminalEvent::Exit => {}
            _ => {}
        }
    }

    fn update_window_title(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let title = self
            .tabs
            .as_ref()
            .and_then(|tabs| tabs.active_tab())
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| self.config.window.title.clone());
        window.set_title(&title);
    }

    fn window_title(&self) -> String {
        self.tabs
            .as_ref()
            .and_then(|tabs| tabs.active_tab())
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| self.config.window.title.clone())
    }

    fn reflow_terminal_layout(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.chrome_metrics = self.chrome_metrics_for_window(&window);
        let snapshot =
            self.terminal_layout_for_size(window.inner_size(), window.scale_factor());

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(snapshot);
        }
        if self.layout.grid_dims_changed(snapshot) {
            self.resize_terminal_grids(snapshot);
            self.pending_pty_layout = Some(snapshot);
            self.schedule_pty_resize(event_loop);
        }
        self.layout = snapshot;
        self.clamp_tab_scroll();
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn copy_selection(&mut self) {
        let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) else {
            return;
        };
        let term = tab.session.terminal.lock();
        if let Some(text) = term.selection_to_string() {
            if !text.is_empty() {
                copy_text(&text);
            }
        }
    }

    fn paste_from_clipboard(&mut self) {
        let Some(text) = paste_text() else {
            return;
        };
        if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
            tab.session.notifier.notify(Cow::Owned(text.into_bytes()));
        }
    }

    fn clear_scrollback(&mut self) {
        if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
            let mut term = tab.session.terminal.lock();
            term.grid_mut().clear_history();
            term.scroll_display(Scroll::Bottom);
            term.reset_damage();
        }
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn apply_font_zoom(&mut self, new_zoom: f32, event_loop: &ActiveEventLoop) {
        let clamped = new_zoom.clamp(FONT_ZOOM_MIN, FONT_ZOOM_MAX);
        if (clamped - self.font_zoom).abs() < f32::EPSILON {
            return;
        }
        self.font_zoom = clamped;
        self.reflow_terminal_layout(event_loop);
    }

    fn apply_terminal_opacity(&mut self, new_opacity: f32) {
        let clamped = new_opacity.clamp(OPACITY_MIN, OPACITY_MAX);
        if (clamped - self.terminal_opacity).abs() < f32::EPSILON {
            return;
        }
        self.terminal_opacity = clamped;
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn resolved_system_theme(&self) -> Theme {
        match self.theme_mode {
            ThemeMode::Light => Theme::Light,
            ThemeMode::Dark => Theme::Dark,
            ThemeMode::Auto => self
                .window
                .as_ref()
                .and_then(|window| window.theme())
                .unwrap_or(Theme::Dark),
        }
    }

    fn sync_theme_colors(&mut self) {
        let theme = self.resolved_system_theme();
        self.config.apply_system_theme(theme);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_config(self.config.clone());
        }
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn apply_theme_mode(&mut self, mode: ThemeMode) {
        self.theme_mode = mode;
        if let Some(window) = self.window.as_ref() {
            let winit_theme = match mode {
                ThemeMode::Light => Some(Theme::Light),
                ThemeMode::Dark => Some(Theme::Dark),
                ThemeMode::Auto => None,
            };
            window.set_theme(winit_theme);
        }
        self.sync_theme_colors();
    }

    fn theme_menu_label(&self, mode: ThemeMode) -> String {
        let name = match mode {
            ThemeMode::Light => "Theme: Light",
            ThemeMode::Dark => "Theme: Dark",
            ThemeMode::Auto => "Theme: Auto",
        };
        if self.theme_mode == mode {
            format!("{name} ✓")
        } else {
            name.to_owned()
        }
    }

    fn build_menu_entries(&self) -> Vec<MenuEntry> {
        vec![
            MenuEntry::Action {
                action: MenuAction::NewTab,
                label: "New Tab".to_owned(),
            },
            MenuEntry::Action {
                action: MenuAction::CloseTab,
                label: "Close Tab".to_owned(),
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
            MenuEntry::Action {
                action: MenuAction::ThemeLight,
                label: self.theme_menu_label(ThemeMode::Light),
            },
            MenuEntry::Action {
                action: MenuAction::ThemeDark,
                label: self.theme_menu_label(ThemeMode::Dark),
            },
            MenuEntry::Action {
                action: MenuAction::ThemeAuto,
                label: self.theme_menu_label(ThemeMode::Auto),
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
            MenuEntry::Action {
                action: MenuAction::OpacityIncrease,
                label: format!(
                    "Opacity + ({:.0}%)",
                    self.terminal_opacity * 100.0
                ),
            },
            MenuEntry::Action {
                action: MenuAction::OpacityDecrease,
                label: "Opacity -".to_owned(),
            },
            MenuEntry::Separator,
            MenuEntry::Action {
                action: MenuAction::Quit,
                label: "Quit".to_owned(),
            },
        ]
    }

    fn refresh_menu_entries(&mut self) {
        self.menu_entries = self.build_menu_entries();
    }

    fn register_click(&mut self, x: f64, y: f64) -> u32 {
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

    fn start_selection_at(
        &mut self,
        x: f64,
        y: f64,
        selection_type: SelectionType,
        copy_immediately: bool,
    ) {
        if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
            let mut term = tab.session.terminal.lock();
            let point = self.grid_point_from_position(
                term.grid().display_offset(),
                x,
                y,
            );
            let side = self.side_from_position(x);
            term.selection = Some(Selection::new(selection_type, point, side));
            term.reset_damage();
        }
        if copy_immediately {
            self.copy_selection();
        }
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn bell_flash_active(&self) -> bool {
        self.bell_flash_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn terminal_content_x(&self, x: f64) -> f64 {
        x - f64::from(self.layout.content_offset_x)
    }

    fn terminal_content_y(&self, y: f64) -> f64 {
        y - f64::from(self.layout.content_offset_y)
    }

    fn is_in_tab_strip(&self, x: f64, y: f64) -> bool {
        x < f64::from(self.chrome_metrics.tab_strip.width)
            && y >= f64::from(self.chrome_metrics.title_bar.height)
    }

    fn is_in_title_bar(&self, _x: f64, y: f64) -> bool {
        y < f64::from(self.chrome_metrics.title_bar.height)
    }

    fn sidebar_divider_hit(&self, x: f64, y: f64) -> bool {
        const HANDLE: f64 = 4.0;
        if self.sidebar_collapsed {
            return false;
        }
        let divider = f64::from(self.chrome_metrics.tab_strip.width);
        y >= f64::from(self.chrome_metrics.title_bar.height)
            && (x - divider).abs() <= HANDLE
    }

    fn toggle_sidebar_collapsed(&mut self, event_loop: &ActiveEventLoop) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.sidebar_dragging = false;

        let Some(window) = self.window.clone() else {
            return;
        };
        self.chrome_metrics = self.chrome_metrics_for_window(&window);
        let snapshot =
            self.terminal_layout_for_size(window.inner_size(), window.scale_factor());

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(snapshot);
        }
        if self.layout.grid_dims_changed(snapshot) {
            self.resize_terminal_grids(snapshot);
            self.pending_pty_layout = Some(snapshot);
            self.schedule_pty_resize(event_loop);
        }
        self.layout = snapshot;
        self.clamp_tab_scroll();
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn apply_sidebar_width(&mut self, new_width: f32, event_loop: &ActiveEventLoop) {
        let clamped = new_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        if (clamped - self.sidebar_width).abs() < 0.5 {
            return;
        }
        self.sidebar_width = clamped;

        let Some(window) = self.window.clone() else {
            return;
        };
        self.chrome_metrics = self.chrome_metrics_for_window(&window);
        let snapshot =
            self.terminal_layout_for_size(window.inner_size(), window.scale_factor());

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_layout(snapshot);
        }
        if self.layout.grid_dims_changed(snapshot) {
            self.resize_terminal_grids(snapshot);
            self.pending_pty_layout = Some(snapshot);
            self.schedule_pty_resize(event_loop);
        }
        self.layout = snapshot;
        self.clamp_tab_scroll();
        self.needs_redraw = true;
        self.request_redraw();
    }

    fn is_in_terminal(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.layout.content_offset_x)
            && y >= f64::from(self.layout.content_offset_y)
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

    fn tab_bar_entries(&self) -> Vec<TabBarEntry> {
        self.tabs
            .as_ref()
            .map(|tabs| {
                tabs.titles()
                    .into_iter()
                    .map(|(id, title, active)| TabBarEntry { id, title, active })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn handle_tab_strip_click(
        &mut self,
        x: f64,
        y: f64,
        button: MouseButton,
        event_loop: &ActiveEventLoop,
    ) {
        let layout = self.tab_strip_layout_snapshot();
        match layout.hit_test(x, y, self.tab_scroll_offset) {
            TabStripHit::Tab(id) => {
                if button == MouseButton::Left {
                    if let Some(tabs) = self.tabs.as_mut() {
                        tabs.set_active(id);
                        self.update_window_title();
                        self.ensure_active_tab_visible();
                        self.needs_redraw = true;
                        self.request_redraw();
                    }
                } else if button == MouseButton::Middle {
                    self.close_tab(id, event_loop);
                }
            }
            TabStripHit::Close(id) => {
                if button == MouseButton::Left || button == MouseButton::Middle {
                    self.close_tab(id, event_loop);
                }
            }
            TabStripHit::NewTab => {
                if button == MouseButton::Left {
                    self.spawn_tab();
                }
            }
            TabStripHit::ToggleCollapse => {
                if button == MouseButton::Left {
                    self.toggle_sidebar_collapsed(event_loop);
                }
            }
            TabStripHit::None => {}
        }
    }

    fn handle_title_bar_click(
        &mut self,
        x: f64,
        y: f64,
        button: MouseButton,
        event_loop: &ActiveEventLoop,
    ) {
        if button != MouseButton::Left {
            return;
        }

        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let Some(layout) = renderer.title_bar_layout() else {
            return;
        };

        match layout.hit_test(x, y) {
            TitleBarHit::Close => {
                self.tabs = None;
                event_loop.exit();
            }
            TitleBarHit::Hamburger => {
                self.menu_open = !self.menu_open;
                if self.menu_open {
                    self.refresh_menu_entries();
                }
                self.needs_redraw = true;
                self.request_redraw();
            }
            TitleBarHit::Drag => {
                if let Some(window) = self.window.as_ref() {
                    let _ = window.drag_window();
                }
            }
            TitleBarHit::None => {}
        }
    }

    fn handle_menu_action(&mut self, action: MenuAction, event_loop: &ActiveEventLoop) {
        let keep_open = matches!(
            action,
            MenuAction::ThemeLight
                | MenuAction::ThemeDark
                | MenuAction::ThemeAuto
                | MenuAction::ZoomIn
                | MenuAction::ZoomOut
                | MenuAction::ZoomReset
                | MenuAction::OpacityIncrease
                | MenuAction::OpacityDecrease
        );
        if !keep_open {
            self.menu_open = false;
        }

        match action {
            MenuAction::NewTab => self.spawn_tab(),
            MenuAction::CloseTab => self.close_active_tab(event_loop),
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Paste => self.paste_from_clipboard(),
            MenuAction::ClearScrollback => self.clear_scrollback(),
            MenuAction::ThemeLight => self.apply_theme_mode(ThemeMode::Light),
            MenuAction::ThemeDark => self.apply_theme_mode(ThemeMode::Dark),
            MenuAction::ThemeAuto => self.apply_theme_mode(ThemeMode::Auto),
            MenuAction::ZoomIn => {
                self.apply_font_zoom(self.font_zoom + FONT_ZOOM_STEP, event_loop);
            }
            MenuAction::ZoomOut => {
                self.apply_font_zoom(self.font_zoom - FONT_ZOOM_STEP, event_loop);
            }
            MenuAction::ZoomReset => self.apply_font_zoom(1.0, event_loop),
            MenuAction::OpacityIncrease => {
                self.apply_terminal_opacity(self.terminal_opacity + OPACITY_STEP);
            }
            MenuAction::OpacityDecrease => {
                self.apply_terminal_opacity(self.terminal_opacity - OPACITY_STEP);
            }
            MenuAction::Quit => {
                self.tabs = None;
                event_loop.exit();
            }
        }

        if self.menu_open {
            self.refresh_menu_entries();
            self.needs_redraw = true;
            self.request_redraw();
        }
    }

    fn resize_hit_at(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        let border = border_size(window.scale_factor());
        resize_direction_at(
            x,
            y,
            size.width as f64,
            size.height as f64,
            border,
        )
    }

    fn update_resize_cursor(&self, x: f64, y: f64) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let icon = if self.sidebar_dragging || self.sidebar_divider_hit(x, y) {
            CursorIcon::ColResize
        } else {
            self.resize_hit_at(x, y)
                .map(cursor_for_direction)
                .unwrap_or(CursorIcon::Default)
        };
        window.set_cursor(icon);
    }

    fn start_resize_drag(&self, x: f64, y: f64) -> bool {
        let Some(window) = self.window.as_ref() else {
            return false;
        };
        let Some(direction) = self.resize_hit_at(x, y) else {
            return false;
        };
        window.drag_resize_window(direction).is_ok()
    }

    fn handle_chrome_click(
        &mut self,
        x: f64,
        y: f64,
        button: MouseButton,
        event_loop: &ActiveEventLoop,
    ) {
        if self.menu_open {
            if let Some(renderer) = self.renderer.as_ref() {
                if let Some(menu) = renderer.menu_layout() {
                    if let Some(action) = menu.hit_test(x, y) {
                        if button == MouseButton::Left {
                            self.handle_menu_action(action, event_loop);
                        }
                        return;
                    }
                }
            }
            self.menu_open = false;
            self.needs_redraw = true;
            self.request_redraw();
        }

        if self.is_in_title_bar(x, y) {
            self.handle_title_bar_click(x, y, button, event_loop);
            return;
        }

        if self.is_in_tab_strip(x, y) {
            if button == MouseButton::Left || button == MouseButton::Middle {
                self.handle_tab_strip_click(x, y, button, event_loop);
            }
        }
    }

    fn handle_key_action(&mut self, action: KeyAction, event_loop: &ActiveEventLoop) {
        match action {
            KeyAction::NewTab => self.spawn_tab(),
            KeyAction::CloseTab => self.close_active_tab(event_loop),
            KeyAction::NextTab => {
                if let Some(tabs) = self.tabs.as_mut() {
                    tabs.next_tab();
                    self.update_window_title();
                    self.ensure_active_tab_visible();
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            KeyAction::PrevTab => {
                if let Some(tabs) = self.tabs.as_mut() {
                    tabs.prev_tab();
                    self.update_window_title();
                    self.ensure_active_tab_visible();
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            KeyAction::SelectTab(number) => {
                if let Some(tabs) = self.tabs.as_mut() {
                    tabs.select_tab_number(number);
                    self.update_window_title();
                    self.ensure_active_tab_visible();
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            KeyAction::SendToTerminal(bytes) => {
                if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
                    tab.session.notifier.notify(Cow::Owned(bytes));
                }
            }
            KeyAction::Copy => self.copy_selection(),
            KeyAction::Paste => self.paste_from_clipboard(),
            KeyAction::ZoomIn => {
                self.apply_font_zoom(self.font_zoom + FONT_ZOOM_STEP, event_loop);
            }
            KeyAction::ZoomOut => {
                self.apply_font_zoom(self.font_zoom - FONT_ZOOM_STEP, event_loop);
            }
            KeyAction::ZoomReset => {
                self.apply_font_zoom(1.0, event_loop);
            }
            KeyAction::ClearScrollback => self.clear_scrollback(),
        }
    }

    fn redraw(&mut self) {
        self.ensure_renderer();

        if self.menu_open {
            self.refresh_menu_entries();
        }

        let tab_entries = self.tab_bar_entries();
        let chrome_metrics = self.chrome_metrics;
        let window_title = self.window_title();
        let menu_open = self.menu_open;
        let menu_entries = self.menu_entries.clone();
        let tab_scroll_offset = self.tab_scroll_offset;
        let window_height = self.window_height();
        let sidebar_collapsed = self.sidebar_collapsed;
        let bell_flash = self.bell_flash_active();
        let terminal_opacity = self.terminal_opacity;

        let tabs = self.tabs.as_ref();
        let renderer = self.renderer.as_mut();
        let (Some(renderer), Some(tabs)) = (renderer, tabs) else {
            return;
        };

        let Some(active) = tabs.active_tab() else {
            return;
        };

        let mut term = active.session.terminal.lock();
        let content = term.renderable_content();
        if let Err(error) = renderer.render(
            content,
            ChromeFrame {
                metrics: chrome_metrics,
                tab_entries: &tab_entries,
                window_title: &window_title,
                menu_open,
                menu_entries: &menu_entries,
                tab_scroll_offset,
                window_height,
                sidebar_collapsed,
                bell_flash,
                terminal_opacity,
            },
        ) {
            tracing::error!(%error, "render failed");
        }
        term.reset_damage();
        self.needs_redraw = false;
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.config.window.title.clone())
                        .with_decorations(false)
                        .with_transparent(true)
                        .with_visible(true)
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            INITIAL_WIDTH,
                            INITIAL_HEIGHT,
                        )),
                )
                .expect("create window"),
        );

        match self.theme_mode {
            ThemeMode::Light => window.set_theme(Some(Theme::Light)),
            ThemeMode::Dark => window.set_theme(Some(Theme::Dark)),
            ThemeMode::Auto => window.set_theme(None),
        }
        let effective_theme = match self.theme_mode {
            ThemeMode::Light => Theme::Light,
            ThemeMode::Dark => Theme::Dark,
            ThemeMode::Auto => window.theme().unwrap_or(Theme::Dark),
        };
        self.config.apply_system_theme(effective_theme);

        self.chrome_metrics = self.chrome_metrics_for_window(&window);
        self.layout = self.terminal_layout_for_size(
            window.inner_size(),
            window.scale_factor(),
        );
        self.window = Some(Arc::clone(&window));

        if let Some(proxy) = self.proxy.clone() {
            self.proxy_factory = Some(EventLoopProxyFactory::new(proxy, window.id()));
        }

        self.startup_deadline = Some(Instant::now() + STARTUP_SETTLE);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            self.startup_deadline.unwrap(),
        ));
        self.update_window_title();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.tabs = None;
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.handle_window_resize(size, event_loop);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = self.window.as_ref() {
                    self.handle_window_resize(window.inner_size(), event_loop);
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                if self.theme_mode == ThemeMode::Auto {
                    self.config.apply_system_theme(theme);
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.set_config(self.config.clone());
                    }
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.menu_open
                    && event.state == ElementState::Pressed
                    && matches!(event.logical_key, winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape))
                {
                    self.menu_open = false;
                    self.needs_redraw = true;
                    self.request_redraw();
                    return;
                }
                if let Some(action) = key_event_to_action(&event, self.modifiers) {
                    self.handle_key_action(action, event_loop);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 20.0).round() as i32,
                };
                if lines == 0 {
                    return;
                }

                if self.is_in_tab_strip(self.cursor_position.0, self.cursor_position.1) {
                    self.scroll_tab_strip(lines);
                    return;
                }

                if !self.is_in_terminal(self.cursor_position.0, self.cursor_position.1) {
                    return;
                }

                if self.modifiers.control_key() {
                    if lines > 0 {
                        self.apply_font_zoom(self.font_zoom + FONT_ZOOM_STEP, event_loop);
                    } else {
                        self.apply_font_zoom(self.font_zoom - FONT_ZOOM_STEP, event_loop);
                    }
                    return;
                }

                if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
                    let mut term = tab.session.terminal.lock();
                    term.scroll_display(Scroll::Delta(lines));
                    term.reset_damage();
                }
                self.needs_redraw = true;
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);

                if self.sidebar_dragging {
                    self.update_resize_cursor(position.x, position.y);
                    self.apply_sidebar_width(position.x as f32, event_loop);
                    return;
                }

                self.update_resize_cursor(position.x, position.y);
                if self.selecting
                    && self.is_in_terminal(position.x, position.y)
                {
                    if let Some(tab) = self.tabs.as_ref().and_then(|tabs| tabs.active_tab()) {
                        let mut term = tab.session.terminal.lock();
                        let point = self.grid_point_from_position(
                            term.grid().display_offset(),
                            position.x,
                            position.y,
                        );
                        let side = self.side_from_position(position.x);
                        if let Some(selection) = term.selection.as_mut() {
                            selection.update(point, side);
                        }
                        term.reset_damage();
                    }
                    self.needs_redraw = true;
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let position = self.cursor_position;

                if button == MouseButton::Left && state == ElementState::Released {
                    self.sidebar_dragging = false;
                }

                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && self.start_resize_drag(position.0, position.1)
                {
                    return;
                }

                if self.menu_open {
                    if state == ElementState::Pressed {
                        self.handle_chrome_click(position.0, position.1, button, event_loop);
                    }
                    return;
                }

                // Begin dragging the sidebar divider to resize it.
                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && self.sidebar_divider_hit(position.0, position.1)
                {
                    self.sidebar_dragging = true;
                    return;
                }

                if self.is_in_tab_strip(position.0, position.1)
                    || self.is_in_title_bar(position.0, position.1)
                {
                    if state == ElementState::Pressed {
                        self.handle_chrome_click(position.0, position.1, button, event_loop);
                    }
                    return;
                }

                if !self.is_in_terminal(position.0, position.1) {
                    return;
                }

                if button == MouseButton::Right && state == ElementState::Pressed {
                    self.paste_from_clipboard();
                    return;
                }

                if button != MouseButton::Left {
                    return;
                }

                match state {
                    ElementState::Pressed => {
                        let clicks = self.register_click(position.0, position.1);
                        match clicks {
                            2 => {
                                self.selecting = false;
                                self.start_selection_at(
                                    position.0,
                                    position.1,
                                    SelectionType::Semantic,
                                    true,
                                );
                            }
                            n if n >= 3 => {
                                self.selecting = false;
                                self.start_selection_at(
                                    position.0,
                                    position.1,
                                    SelectionType::Lines,
                                    true,
                                );
                            }
                            _ => {
                                self.selecting = true;
                                if let Some(tab) =
                                    self.tabs.as_ref().and_then(|tabs| tabs.active_tab())
                                {
                                    let mut term = tab.session.terminal.lock();
                                    let point = self.grid_point_from_position(
                                        term.grid().display_offset(),
                                        position.0,
                                        position.1,
                                    );
                                    let side = self.side_from_position(position.0);
                                    term.selection = Some(Selection::new(
                                        SelectionType::Simple,
                                        point,
                                        side,
                                    ));
                                    term.reset_damage();
                                }
                                self.needs_redraw = true;
                                self.request_redraw();
                            }
                        }
                    }
                    ElementState::Released => {
                        if self.selecting {
                            self.copy_selection();
                        }
                        self.selecting = false;
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.needs_redraw {
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Terminal {
                tab_id,
                event: TerminalEvent::Exit,
            } => self.handle_tab_shell_exit(tab_id, event_loop),
            UserEvent::Terminal { tab_id, event } => self.handle_terminal_event(tab_id, event),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.maybe_spawn_initial_tab(event_loop);
        self.flush_pty_resize(event_loop);
        if let Some(until) = self.bell_flash_until {
            if Instant::now() < until {
                event_loop.set_control_flow(ControlFlow::WaitUntil(until));
                self.needs_redraw = true;
            } else {
                self.bell_flash_until = None;
                self.needs_redraw = true;
            }
        }
        if self.needs_redraw {
            self.request_redraw();
        }
    }
}

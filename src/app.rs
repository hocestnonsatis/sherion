use std::collections::HashMap;
use std::sync::Arc;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::viewport_to_point;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Theme, Window, WindowId};

use crate::config::{Config, ThemeMode};
use crate::event::UserEvent;
use crate::input::key_event_to_action;
use crate::render::MenuHit;
use crate::tabs::{EventLoopProxyFactory, Tab, TabId};
use crate::window_state::{
    MenuAppAction, TabStripAction, WindowState, BUSY_POLL_INTERVAL, IDLE_POLL_INTERVAL,
    INITIAL_HEIGHT, INITIAL_WIDTH, OPACITY_MAX, OPACITY_MIN, STARTUP_SETTLE,
};

pub struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    theme_mode: ThemeMode,
    next_tab_id: usize,
    windows: HashMap<WindowId, WindowState>,
}

impl App {
    pub fn run(config: Config) -> Result<()> {
        let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let theme_mode = config.appearance.theme;
        let terminal_opacity = config.appearance.opacity.clamp(OPACITY_MIN, OPACITY_MAX);

        let mut app = Self {
            config,
            proxy,
            theme_mode,
            next_tab_id: 0,
            windows: HashMap::new(),
        };

        event_loop.run_app(&mut app)?;
        let _ = terminal_opacity;
        Ok(())
    }

    fn terminal_opacity(&self) -> f32 {
        self.config
            .appearance
            .opacity
            .clamp(OPACITY_MIN, OPACITY_MAX)
    }

    fn find_window_for_tab(&self, tab_id: TabId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, ws)| ws.tabs.tab_by_id(tab_id).is_some())
            .map(|(id, _)| *id)
    }

    fn resolved_system_theme(&self) -> Theme {
        match self.theme_mode {
            ThemeMode::Light => Theme::Light,
            ThemeMode::Dark => Theme::Dark,
            ThemeMode::Auto => self
                .windows
                .values()
                .next()
                .and_then(|ws| ws.window.theme())
                .unwrap_or(Theme::Dark),
        }
    }

    fn sync_theme_colors(&mut self) {
        let theme = self.resolved_system_theme();
        self.config.apply_system_theme(theme);
        for ws in self.windows.values_mut() {
            ws.sync_theme(&self.config);
        }
    }

    fn apply_theme_mode(&mut self, mode: ThemeMode) {
        self.theme_mode = mode;
        self.config.appearance.theme = mode;
        let winit_theme = match mode {
            ThemeMode::Light => Some(Theme::Light),
            ThemeMode::Dark => Some(Theme::Dark),
            ThemeMode::Auto => None,
        };
        for ws in self.windows.values() {
            ws.window.set_theme(winit_theme);
        }
        self.sync_theme_colors();
        self.save_config_quietly();
    }

    fn save_config_quietly(&self) {
        if let Err(error) = self.config.save() {
            tracing::warn!(%error, "failed to save config");
        }
    }

    fn capture_window_preferences(&mut self, window_id: WindowId) {
        if let Some(ws) = self.windows.get(&window_id) {
            ws.write_preferences_to_config(&mut self.config);
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
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

        let window_id = window.id();
        let proxy_factory = EventLoopProxyFactory::new(self.proxy.clone(), window_id);
        let ws =
            WindowState::new_pending(window, proxy_factory, &self.config, self.terminal_opacity());
        self.windows.insert(window_id, ws);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + STARTUP_SETTLE,
        ));
    }

    fn open_window_with_tab(
        &mut self,
        event_loop: &ActiveEventLoop,
        tab: Tab,
        font_zoom: f32,
        sidebar_width: f32,
        sidebar_collapsed: bool,
        terminal_opacity: f32,
        outer_pos: Option<PhysicalPosition<i32>>,
    ) -> Option<WindowId> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(tab.title.clone())
                        .with_decorations(false)
                        .with_transparent(true)
                        .with_visible(true)
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            INITIAL_WIDTH,
                            INITIAL_HEIGHT,
                        )),
                )
                .expect("create detached window"),
        );

        if let Some(pos) = outer_pos {
            window.set_outer_position(PhysicalPosition::new(pos.x + 40, pos.y + 40));
        }

        match self.theme_mode {
            ThemeMode::Light => window.set_theme(Some(Theme::Light)),
            ThemeMode::Dark => window.set_theme(Some(Theme::Dark)),
            ThemeMode::Auto => window.set_theme(None),
        }

        let window_id = window.id();
        let proxy_factory = EventLoopProxyFactory::new(self.proxy.clone(), window_id);
        let sidebar_width = if sidebar_width > 0.0 {
            sidebar_width
        } else {
            crate::render::ChromeMetrics::from_font_size(
                self.config.font.size * font_zoom,
                window.scale_factor() as f32,
            )
            .tab_strip
            .width
        };

        let mut ws = WindowState::new_with_tab(
            window,
            tab,
            proxy_factory,
            &self.config,
            terminal_opacity,
            font_zoom,
            sidebar_width,
            sidebar_collapsed,
        );
        ws.after_attach(&self.config, event_loop);
        self.windows.insert(window_id, ws);
        Some(window_id)
    }

    fn close_window(&mut self, window_id: WindowId, event_loop: &ActiveEventLoop) {
        self.capture_window_preferences(window_id);
        self.save_config_quietly();
        self.windows.remove(&window_id);
        if self.windows.is_empty() {
            event_loop.exit();
        }
    }

    fn spawn_tab(&mut self, window_id: WindowId, event_loop: &ActiveEventLoop) {
        let Some(ws) = self.windows.get_mut(&window_id) else {
            return;
        };
        ws.spawn_tab(&self.config, &mut self.next_tab_id, event_loop);
        ws.update_window_title(&self.config);
    }

    fn close_active_tab(&mut self, window_id: WindowId, event_loop: &ActiveEventLoop) {
        let Some(active_id) = self
            .windows
            .get(&window_id)
            .and_then(|ws| ws.tabs.active_id())
        else {
            return;
        };
        self.close_tab(window_id, active_id, event_loop);
    }

    fn close_tab(&mut self, window_id: WindowId, tab_id: TabId, event_loop: &ActiveEventLoop) {
        let empty = {
            let Some(ws) = self.windows.get_mut(&window_id) else {
                return;
            };
            ws.close_tab(tab_id, &self.config, event_loop)
        };
        if empty {
            self.close_window(window_id, event_loop);
        } else if let Some(ws) = self.windows.get_mut(&window_id) {
            ws.update_window_title(&self.config);
        }
    }

    fn detach_tab(&mut self, source_id: WindowId, tab_id: TabId, event_loop: &ActiveEventLoop) {
        let (font_zoom, sidebar_width, sidebar_collapsed, terminal_opacity, outer_pos) = self
            .windows
            .get(&source_id)
            .map(|ws| {
                (
                    ws.font_zoom,
                    ws.sidebar_width,
                    ws.sidebar_collapsed,
                    ws.terminal_opacity,
                    ws.window.outer_position().ok(),
                )
            })
            .unwrap_or((1.0, 0.0, false, self.terminal_opacity(), None));

        let tab = {
            let Some(ws) = self.windows.get_mut(&source_id) else {
                return;
            };
            ws.tabs.take_tab(tab_id)
        };
        let Some(tab) = tab else {
            return;
        };

        self.open_window_with_tab(
            event_loop,
            tab,
            font_zoom,
            sidebar_width,
            sidebar_collapsed,
            terminal_opacity,
            outer_pos,
        );

        let source_empty = self
            .windows
            .get(&source_id)
            .is_some_and(|ws| ws.tabs.is_empty());
        if source_empty {
            self.close_window(source_id, event_loop);
        } else if let Some(ws) = self.windows.get_mut(&source_id) {
            ws.sync_terminal_layout(&self.config, event_loop);
            ws.update_window_title(&self.config);
        }
    }

    fn detach_active_tab(&mut self, window_id: WindowId, event_loop: &ActiveEventLoop) {
        let Some(active_id) = self
            .windows
            .get(&window_id)
            .and_then(|ws| ws.tabs.active_id())
        else {
            return;
        };
        self.detach_tab(window_id, active_id, event_loop);
    }

    fn handle_menu_app_action(
        &mut self,
        window_id: WindowId,
        action: MenuAppAction,
        event_loop: &ActiveEventLoop,
    ) {
        match action {
            MenuAppAction::NewTab => self.spawn_tab(window_id, event_loop),
            MenuAppAction::DuplicateTab => self.spawn_tab(window_id, event_loop),
            MenuAppAction::CloseTab => self.close_active_tab(window_id, event_loop),
            MenuAppAction::DetachTab => self.detach_active_tab(window_id, event_loop),
            MenuAppAction::Quit => {
                if let Some(window_id) = self.windows.keys().copied().next() {
                    self.capture_window_preferences(window_id);
                    self.save_config_quietly();
                }
                self.windows.clear();
                event_loop.exit();
            }
            MenuAppAction::Theme(mode) => self.apply_theme_mode(mode),
        }
    }

    fn handle_terminal_event(
        &mut self,
        window_id: WindowId,
        tab_id: TabId,
        event: TerminalEvent,
        event_loop: &ActiveEventLoop,
    ) {
        match event {
            TerminalEvent::Exit => {
                self.close_tab(window_id, tab_id, event_loop);
            }
            other => {
                if let Some(ws) = self.windows.get_mut(&window_id) {
                    ws.handle_terminal_event(&self.config, tab_id, other);
                }
            }
        }
    }

    fn dispatch_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(ws) = self.windows.get_mut(&window_id) else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                self.close_window(window_id, event_loop);
            }
            WindowEvent::Resized(size) => {
                ws.handle_window_resize(&self.config, size, event_loop);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = ws.window.inner_size();
                ws.handle_window_resize(&self.config, size, event_loop);
            }
            WindowEvent::ThemeChanged(theme) => {
                if self.theme_mode == ThemeMode::Auto {
                    self.config.apply_system_theme(theme);
                    ws.sync_theme(&self.config);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                ws.modifiers = modifiers.state();
                if ws.is_in_terminal(ws.cursor_position.0, ws.cursor_position.1) {
                    ws.needs_redraw = true;
                    ws.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let (palette_consumed, palette_action) = ws.handle_command_palette_key(
                    &self.config,
                    self.theme_mode,
                    event_loop,
                    &event,
                    ws.modifiers,
                );
                if palette_consumed {
                    if let Some(app_action) = palette_action {
                        self.handle_menu_app_action(window_id, app_action, event_loop);
                    }
                    return;
                }
                if ws.handle_rename_key(&self.config, &event) {
                    return;
                }
                if ws.handle_search_key(&event, ws.modifiers) {
                    return;
                }
                if ws.menu_open
                    && event.state == ElementState::Pressed
                    && matches!(
                        event.logical_key,
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
                    )
                {
                    ws.dismiss_menu();
                    ws.needs_redraw = true;
                    ws.request_redraw();
                    return;
                }
                if let Some(action) = key_event_to_action(&event, ws.modifiers) {
                    if let Some(app_action) = ws.handle_key_action(&self.config, action, event_loop)
                    {
                        self.handle_menu_app_action(window_id, app_action, event_loop);
                    }
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

                if ws.is_in_tab_strip(ws.cursor_position.0, ws.cursor_position.1) {
                    ws.scroll_tab_strip(lines);
                    return;
                }

                if !ws.is_in_terminal(ws.cursor_position.0, ws.cursor_position.1) {
                    return;
                }

                if ws.modifiers.control_key() {
                    let zoom_step = crate::window_state::FONT_ZOOM_STEP;
                    if lines > 0 {
                        ws.apply_font_zoom(&self.config, ws.font_zoom + zoom_step, event_loop);
                    } else {
                        ws.apply_font_zoom(&self.config, ws.font_zoom - zoom_step, event_loop);
                    }
                    return;
                }

                if let Some(tab_index) =
                    ws.tab_index_for_scroll(ws.cursor_position.0, ws.cursor_position.1)
                {
                    if let Some(tab) = ws.tabs.tab_at_index(tab_index) {
                        let mut term = tab.session.terminal.lock();
                        term.scroll_display(Scroll::Delta(lines));
                    }
                }
                ws.invalidate_terminal_capture();
                ws.needs_redraw = true;
                ws.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                ws.cursor_position = (position.x, position.y);

                if ws.menu_slider_dragging {
                    let value = ws
                        .renderer
                        .as_ref()
                        .and_then(|renderer| renderer.menu_layout())
                        .and_then(|menu| menu.opacity_from_x(position.x));
                    if let Some(value) = value {
                        ws.apply_slider_opacity(self.theme_mode, value);
                    }
                    return;
                }

                if ws.sidebar_dragging {
                    ws.update_resize_cursor(position.x, position.y);
                    ws.apply_sidebar_width(&self.config, position.x as f32, event_loop);
                    return;
                }

                ws.update_resize_cursor(position.x, position.y);
                let on_link = ws.modifiers.control_key()
                    && ws.url_at_position(position.x, position.y).is_some();
                if on_link {
                    ws.window.set_cursor(CursorIcon::Pointer);
                }
                if ws.modifiers.control_key() && ws.is_in_terminal(position.x, position.y) {
                    ws.needs_redraw = true;
                    ws.request_redraw();
                }
                if ws.selecting && ws.is_in_terminal(position.x, position.y) {
                    if let Some(tab) = ws.tabs.active_tab() {
                        let mut term = tab.session.terminal.lock();
                        let point = grid_point_from_position(
                            &ws.layout,
                            term.grid().display_offset(),
                            position.x,
                            position.y,
                        );
                        let side = side_from_position(&ws.layout, position.x);
                        if let Some(selection) = term.selection.as_mut() {
                            selection.update(point, side);
                        }
                    }
                    // Selection changes don't always surface as grid damage for
                    // every affected row, so force a full recapture/redraw to keep
                    // the highlight consistent while dragging.
                    ws.request_full_capture();
                    ws.needs_redraw = true;
                    ws.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let position = ws.cursor_position;

                if button == MouseButton::Left && state == ElementState::Released {
                    ws.sidebar_dragging = false;
                    ws.menu_slider_dragging = false;
                }

                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && ws.start_resize_drag(position.0, position.1)
                {
                    return;
                }

                if ws.menu_open {
                    if state == ElementState::Pressed {
                        self.handle_chrome_click(
                            window_id, position.0, position.1, button, event_loop,
                        );
                    }
                    return;
                }

                if ws.search_active()
                    && state == ElementState::Pressed
                    && button == MouseButton::Left
                    && ws.handle_search_overlay_click(position.0, position.1)
                {
                    return;
                }

                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && ws.sidebar_divider_hit(position.0, position.1)
                {
                    ws.sidebar_dragging = true;
                    return;
                }

                if ws.is_in_tab_strip(position.0, position.1)
                    || ws.is_in_title_bar(position.0, position.1)
                {
                    if state == ElementState::Pressed {
                        self.handle_chrome_click(
                            window_id, position.0, position.1, button, event_loop,
                        );
                    }
                    return;
                }

                if !ws.is_in_terminal(position.0, position.1) {
                    return;
                }

                if button == MouseButton::Right && state == ElementState::Pressed {
                    ws.paste_from_clipboard();
                    return;
                }

                if button != MouseButton::Left {
                    return;
                }

                match state {
                    ElementState::Pressed => {
                        if ws.modifiers.control_key()
                            && ws.open_url_at_position(position.0, position.1)
                        {
                            return;
                        }
                        if let Some(slot) = ws.pane_at(position.0, position.1) {
                            ws.focus_pane_slot(slot, &self.config);
                        }
                        let clicks = ws.register_click(position.0, position.1);
                        match clicks {
                            2 => {
                                ws.selecting = false;
                                ws.start_selection_at(
                                    position.0,
                                    position.1,
                                    SelectionType::Semantic,
                                    true,
                                );
                            }
                            n if n >= 3 => {
                                ws.selecting = false;
                                ws.start_selection_at(
                                    position.0,
                                    position.1,
                                    SelectionType::Lines,
                                    true,
                                );
                            }
                            _ => {
                                ws.selecting = true;
                                if let Some(tab) = ws.tabs.active_tab() {
                                    let mut term = tab.session.terminal.lock();
                                    let point = grid_point_from_position(
                                        &ws.layout,
                                        term.grid().display_offset(),
                                        position.0,
                                        position.1,
                                    );
                                    let side = side_from_position(&ws.layout, position.0);
                                    term.selection =
                                        Some(Selection::new(SelectionType::Simple, point, side));
                                }
                                ws.needs_redraw = true;
                                ws.request_redraw();
                            }
                        }
                    }
                    ElementState::Released => {
                        if ws.selecting {
                            ws.copy_selection();
                        }
                        ws.selecting = false;
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                ws.redraw(&self.config, self.theme_mode);
                if ws.needs_redraw {
                    ws.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn handle_chrome_click(
        &mut self,
        window_id: WindowId,
        x: f64,
        y: f64,
        button: MouseButton,
        event_loop: &ActiveEventLoop,
    ) {
        if let Some(ws) = self.windows.get_mut(&window_id) {
            if ws.menu_open {
                let hit = ws
                    .renderer
                    .as_ref()
                    .and_then(|renderer| renderer.menu_layout())
                    .and_then(|menu| menu.hit_test(x, y));
                if let Some(hit) = hit {
                    if button == MouseButton::Left {
                        match hit {
                            MenuHit::Action(action) => {
                                if let Some(app_action) = ws.handle_menu_action(
                                    &self.config,
                                    self.theme_mode,
                                    action,
                                    event_loop,
                                ) {
                                    self.handle_menu_app_action(window_id, app_action, event_loop);
                                }
                            }
                            MenuHit::Opacity(value) => {
                                ws.menu_slider_dragging = true;
                                ws.apply_slider_opacity(self.theme_mode, value);
                            }
                            MenuHit::Keep => {}
                        }
                    }
                    return;
                }
                ws.dismiss_menu();
                ws.needs_redraw = true;
                ws.request_redraw();
            }
        }

        let theme_mode = self.theme_mode;
        let config = self.config.clone();

        if let Some(ws) = self.windows.get_mut(&window_id) {
            if ws.is_in_title_bar(x, y) {
                if ws.handle_title_bar_click(&config, theme_mode, x, y, button) {
                    self.close_window(window_id, event_loop);
                }
                return;
            }
        }

        if let Some(ws) = self.windows.get_mut(&window_id) {
            if ws.is_in_tab_strip(x, y)
                && (button == MouseButton::Left || button == MouseButton::Middle)
            {
                match ws.handle_tab_strip_click(&config, x, y, button, event_loop) {
                    TabStripAction::NewTab => self.spawn_tab(window_id, event_loop),
                    TabStripAction::Detach(id) => self.detach_tab(window_id, id, event_loop),
                    TabStripAction::WindowEmpty => self.close_window(window_id, event_loop),
                    TabStripAction::None => {}
                }
            }
        }
    }
}

fn grid_point_from_position(
    layout: &crate::render::TerminalLayout,
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

fn side_from_position(layout: &crate::render::TerminalLayout, x: f64) -> Side {
    let col = (x - f64::from(layout.content_offset_x)) / f64::from(layout.cell_width);
    if col.fract() > 0.5 {
        Side::Right
    } else {
        Side::Left
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            self.create_window(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.dispatch_window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Terminal {
                window_id,
                tab_id,
                event,
            } => {
                let resolved = self.find_window_for_tab(tab_id).unwrap_or(window_id);
                self.handle_terminal_event(resolved, tab_id, event, event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let window_ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let mut next_tab_id = self.next_tab_id;

        let mut any_initial_pending = false;
        let mut next_wake: Option<std::time::Instant> = None;
        let now = std::time::Instant::now();

        for window_id in &window_ids {
            if let Some(ws) = self.windows.get_mut(window_id) {
                if ws.initial_tab_pending {
                    any_initial_pending = true;
                    ws.maybe_spawn_initial_tab(&self.config, &mut next_tab_id, event_loop);
                    continue;
                }

                ws.flush_pty_resize(event_loop);

                let busy = ws.refresh_tab_activity();
                let interval = if busy {
                    BUSY_POLL_INTERVAL
                } else {
                    IDLE_POLL_INTERVAL
                };
                next_wake = Some(next_wake.map_or(now + interval, |w| w.min(now + interval)));

                // Quiet foreground commands (e.g. `sleep`) need a timer tick on the
                // active tab, but only when the displayed second changes.
                if busy && ws.tick_busy_timer() {
                    ws.needs_redraw = true;
                }

                if let Some(until) = ws.bell_flash_until {
                    if now < until {
                        next_wake = Some(next_wake.map_or(until, |w| w.min(until)));
                        ws.needs_redraw = true;
                    } else {
                        ws.bell_flash_until = None;
                        ws.needs_redraw = true;
                    }
                }

                if ws.needs_redraw {
                    ws.request_redraw();
                }
            }
        }

        self.next_tab_id = next_tab_id;

        if !any_initial_pending {
            if let Some(deadline) = next_wake {
                let any_pending_resize = self
                    .windows
                    .values()
                    .any(|ws| !ws.pending_pty_layouts.is_empty());
                if !any_pending_resize {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
                }
            }
        }
    }
}

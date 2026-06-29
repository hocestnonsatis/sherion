use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::selection::SelectionType;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Theme, Window, WindowId};

use crate::config::{Config, ThemeMode, WindowDecorations};
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
    config_reload_rx: Option<Receiver<()>>,
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
            config_reload_rx: Self::watch_config_file(),
        };

        event_loop.run_app(&mut app)?;
        let _ = terminal_opacity;
        Ok(())
    }

    fn watch_config_file() -> Option<Receiver<()>> {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel();
        let path = crate::config::config_path();
        let mut watcher = RecommendedWatcher::new(
            move |_| {
                let _ = tx.send(());
            },
            notify::Config::default(),
        )
        .ok()?;
        if path.exists() {
            let _ = watcher.watch(&path, RecursiveMode::NonRecursive);
        }
        std::mem::forget(watcher);
        Some(rx)
    }

    fn reload_config_if_changed(&mut self) {
        let Some(rx) = &self.config_reload_rx else {
            return;
        };
        while rx.try_recv().is_ok() {
            if let Ok(config) = Config::load_profile(self.config.active_profile.as_deref()) {
                self.config = config;
                for ws in self.windows.values_mut() {
                    ws.apply_config_reload(&self.config);
                }
            }
        }
    }

    pub fn switch_profile(&mut self, name: &str) {
        if let Ok(config) = Config::load_profile(Some(name)) {
            self.config = config;
            for ws in self.windows.values_mut() {
                ws.apply_config_reload(&self.config);
            }
        }
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

    fn window_inner_size(config: &Config) -> (u32, u32) {
        let width = if config.window.width > 0.0 {
            config.window.width.round() as u32
        } else {
            INITIAL_WIDTH
        };
        let height = if config.window.height > 0.0 {
            config.window.height.round() as u32
        } else {
            INITIAL_HEIGHT
        };
        (width.max(200), height.max(150))
    }

    fn apply_window_position(window: &Window, config: &Config) {
        if let (Some(x), Some(y)) = (config.window.x, config.window.y) {
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
    }

    fn window_attributes(
        &self,
        title: &str,
        width: u32,
        height: u32,
    ) -> winit::window::WindowAttributes {
        let native = self.config.window.decorations == WindowDecorations::Native;
        Window::default_attributes()
            .with_title(title)
            .with_decorations(native)
            .with_transparent(!native)
            .with_visible(true)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let (width, height) = Self::window_inner_size(&self.config);
        let window = Arc::new(
            event_loop
                .create_window(self.window_attributes(&self.config.window.title, width, height))
                .expect("create window"),
        );

        Self::apply_window_position(&window, &self.config);

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
        let (width, height) = Self::window_inner_size(&self.config);
        let window = Arc::new(
            event_loop
                .create_window(self.window_attributes(&tab.title, width, height))
                .expect("create detached window"),
        );

        if let Some(pos) = outer_pos {
            window.set_outer_position(PhysicalPosition::new(pos.x + 40, pos.y + 40));
        } else {
            Self::apply_window_position(&window, &self.config);
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
            MenuAppAction::DuplicateTab => {
                if let Some(ws) = self.windows.get_mut(&window_id) {
                    ws.duplicate_tab(&self.config, &mut self.next_tab_id, event_loop);
                }
            }
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
            MenuAppAction::ToggleAlwaysOnTop => {
                if let Some(ws) = self.windows.get_mut(&window_id) {
                    ws.toggle_always_on_top(&mut self.config);
                }
            }
            MenuAppAction::SwitchProfile(name) => {
                self.switch_profile(&name);
            }
        }
    }

    fn handle_terminal_event(
        &mut self,
        window_id: WindowId,
        tab_id: TabId,
        leaf_id: usize,
        event: TerminalEvent,
        event_loop: &ActiveEventLoop,
    ) {
        match event {
            TerminalEvent::Exit => {
                if let Some(ws) = self.windows.get_mut(&window_id) {
                    if let Some(tab) = ws.tabs.tab_by_id(tab_id) {
                        if tab.leaf_count() <= 1 {
                            self.close_tab(window_id, tab_id, event_loop);
                            return;
                        }
                    }
                    if let Some(tab) = ws.tabs.tab_by_id_mut(tab_id) {
                        tab.root.remove_leaf(leaf_id);
                        tab.focused_leaf = tab.focused_leaf.min(tab.leaf_count().saturating_sub(1));
                        ws.sync_terminal_layout(&self.config, event_loop);
                    }
                }
            }
            other => {
                if let Some(ws) = self.windows.get_mut(&window_id) {
                    ws.handle_terminal_event(&self.config, tab_id, leaf_id, other);
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
            WindowEvent::Occluded(false) => {
                // The window became visible again (e.g. a browser launched from
                // the shell was closed). Force a fresh paint so any frame that
                // was dropped while we were covered is restored immediately.
                ws.request_full_capture();
                ws.needs_redraw = true;
                ws.request_redraw();
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
                if ws.handle_rename_key(&self.config, &event) {
                    return;
                }
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
                let (mode, modify_other_keys, ime_composing) = ws.terminal_input_state();
                if let Some(action) = key_event_to_action(
                    &event,
                    ws.modifiers,
                    mode,
                    modify_other_keys,
                    ime_composing,
                    &self.config.keybindings,
                ) {
                    if let Some(app_action) = ws.handle_key_action(&self.config, action, event_loop)
                    {
                        self.handle_menu_app_action(window_id, app_action, event_loop);
                    }
                }
            }
            WindowEvent::Ime(ime) => {
                ws.handle_ime(ime);
            }
            WindowEvent::Focused(focused) => {
                ws.handle_window_focus(focused);
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

                if ws.try_report_mouse_wheel(lines, ws.cursor_position.0, ws.cursor_position.1) {
                    return;
                }

                if let Some((tab_index, leaf_id)) =
                    ws.tab_index_for_scroll(ws.cursor_position.0, ws.cursor_position.1)
                {
                    if let Some(tab) = ws.tabs.tab_at_index(tab_index) {
                        if let Some(session) = tab.leaf_session(leaf_id) {
                            let mut term = session.terminal.lock();
                            term.scroll_display(Scroll::Delta(lines));
                        }
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
                        ws.apply_slider_opacity(&self.config, self.theme_mode, value);
                    }
                    return;
                }

                if ws.sidebar_dragging {
                    ws.update_resize_cursor(&self.config, position.x, position.y);
                    ws.apply_sidebar_width(&self.config, position.x as f32, event_loop);
                    return;
                }

                if ws.split_dragging() {
                    ws.update_resize_cursor(&self.config, position.x, position.y);
                    ws.apply_split_divider_drag(&self.config, position.x, position.y, event_loop);
                    return;
                }

                ws.update_resize_cursor(&self.config, position.x, position.y);
                let on_link = ws.modifiers.control_key()
                    && ws.url_at_position(position.x, position.y).is_some();
                if on_link {
                    ws.window.set_cursor(CursorIcon::Pointer);
                }
                if ws.modifiers.control_key() && ws.is_in_terminal(position.x, position.y) {
                    ws.needs_redraw = true;
                    ws.request_redraw();
                }
                let mouse_motion_reported = ws.try_report_mouse_motion(position.x, position.y);
                if !mouse_motion_reported
                    && ws.selecting
                    && ws.is_in_terminal(position.x, position.y)
                {
                    let _ = ws.update_selection_drag(position.x, position.y);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let position = ws.cursor_position;

                if button == MouseButton::Left && state == ElementState::Released {
                    ws.sidebar_dragging = false;
                    ws.menu_slider_dragging = false;
                    ws.end_split_divider_drag();
                    if ws.is_in_tab_strip(position.0, position.1) {
                        ws.finish_tab_drag(position.0, position.1);
                    }
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

                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && ws.start_split_divider_drag(&self.config, position.0, position.1)
                {
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

                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && ws.try_scrollbar_click(position.0, position.1)
                {
                    return;
                }

                if ws.try_report_mouse_button(button, state, position.0, position.1) {
                    return;
                }

                if button == MouseButton::Right && state == ElementState::Pressed {
                    ws.paste_from_clipboard(&self.config);
                    return;
                }

                if button == MouseButton::Middle && state == ElementState::Pressed {
                    ws.paste_primary(&self.config);
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
                        if ws.modifiers.alt_key() {
                            ws.selecting = true;
                            ws.start_selection_at(
                                position.0,
                                position.1,
                                SelectionType::Block,
                                false,
                            );
                            return;
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
                                ws.begin_simple_selection(position.0, position.1);
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
                                ws.apply_slider_opacity(&self.config, self.theme_mode, value);
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
                leaf_id,
                event,
            } => {
                let resolved = self.find_window_for_tab(tab_id).unwrap_or(window_id);
                self.handle_terminal_event(resolved, tab_id, leaf_id, event, event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.reload_config_if_changed();
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

                let busy = ws.refresh_tab_activity(self.config.terminal.busy_heuristic_ms);
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

                if let Some(until) = ws.blink_wake_deadline() {
                    if now < until {
                        next_wake = Some(next_wake.map_or(until, |w| w.min(until)));
                    } else {
                        ws.tick_cursor_blink();
                    }
                } else {
                    ws.ensure_blink_timer();
                }
                if let Some(until) = ws.blink_wake_deadline() {
                    next_wake = Some(next_wake.map_or(until, |w| w.min(until)));
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

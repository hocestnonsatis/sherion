use std::sync::Arc;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;

use crate::config::Config;
use crate::event::EventProxy;
use crate::pty::PtySession;
use crate::render::TerminalLayout;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub usize);

pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub session: PtySession,
}

pub struct TabManager {
    tabs: Vec<Tab>,
    active: usize,
    next_id: usize,
}

impl TabManager {
    pub fn with_initial(tab: Tab) -> Self {
        Self {
            tabs: vec![tab],
            active: 0,
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.tabs.get(self.active).map(|tab| tab.id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let active = self.active;
        self.tabs.get_mut(active)
    }

    pub fn tab_by_id(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_by_id_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn titles(&self) -> Vec<(TabId, String, bool)> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| (tab.id, tab.title.clone(), index == self.active))
            .collect()
    }

    pub fn spawn_tab(
        &mut self,
        config: &Config,
        layout: TerminalLayout,
        proxy: EventLoopProxyFactory,
    ) -> Option<TabId> {
        let id = TabId(self.next_id);
        self.next_id += 1;

        let event_proxy = proxy.for_tab(id);
        let title = format!("Tab {}", id.0);
        tracing::info!(cols = layout.cols, rows = layout.rows, "spawning new tab");
        match PtySession::spawn(config, layout, event_proxy) {
            Ok(session) => {
                let tab = Tab { id, title, session };
                self.tabs.push(tab);
                let index = self.tabs.len() - 1;
                self.active = index;
                Some(id)
            }
            Err(error) => {
                tracing::error!(%error, tab_id = id.0, "failed to spawn tab");
                None
            }
        }
    }

    pub fn close_tab(&mut self, id: TabId) -> bool {
        let Some(index) = self.tab_index(id) else {
            return false;
        };

        self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active = 0;
            return true;
        }

        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }

        true
    }

    pub fn set_active(&mut self, id: TabId) -> bool {
        let Some(index) = self.tab_index(id) else {
            return false;
        };
        self.active = index;
        true
    }

    pub fn next_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.active = (self.active + 1) % self.tabs.len();
    }

    pub fn prev_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.active = if self.active == 0 {
            self.tabs.len() - 1
        } else {
            self.active - 1
        };
    }

    pub fn select_tab_number(&mut self, number: usize) {
        if number == 0 || number > self.tabs.len() {
            return;
        }
        self.active = number - 1;
    }

    pub fn set_title(&mut self, id: TabId, title: String) {
        if let Some(tab) = self.tab_by_id_mut(id) {
            tab.title = title;
        }
    }

    pub fn reset_title(&mut self, id: TabId, fallback: &str) {
        if let Some(tab) = self.tab_by_id_mut(id) {
            tab.title = fallback.to_owned();
        }
    }

    pub fn active_terminal(&self) -> Option<Arc<FairMutex<Term<EventProxy>>>> {
        self.active_tab()
            .map(|tab| Arc::clone(&tab.session.terminal))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut()
    }
}

#[derive(Clone)]
pub struct EventLoopProxyFactory {
    proxy: winit::event_loop::EventLoopProxy<crate::event::UserEvent>,
    window_id: winit::window::WindowId,
}

impl EventLoopProxyFactory {
    pub fn new(
        proxy: winit::event_loop::EventLoopProxy<crate::event::UserEvent>,
        window_id: winit::window::WindowId,
    ) -> Self {
        Self { proxy, window_id }
    }

    pub fn for_tab(&self, tab_id: TabId) -> EventProxy {
        EventProxy::new(self.proxy.clone(), self.window_id, tab_id)
    }
}

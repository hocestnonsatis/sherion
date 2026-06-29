use std::path::PathBuf;
use std::time::Instant;

use crate::config::Config;
use crate::event::EventProxy;
use crate::pty::PtySession;
use crate::render::TerminalLayout;
use crate::split::{spawn_leaf_session, SplitNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub usize);

pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub root: SplitNode,
    pub focused_leaf: usize,
    pub pinned: bool,
    pub accent: Option<[u8; 3]>,
}

/// A lightweight snapshot of a tab used to build the sidebar entries.
pub struct TabInfo {
    pub id: TabId,
    pub title: String,
    pub cwd: Option<PathBuf>,
    pub active: bool,
    pub running_since: Option<Instant>,
    pub pinned: bool,
    pub accent: Option<[u8; 3]>,
}

pub struct TabManager {
    tabs: Vec<Tab>,
    active: usize,
}

impl Tab {
    pub fn new(id: TabId, title: String, session: PtySession) -> Self {
        Self {
            id,
            title,
            root: SplitNode::leaf(session),
            focused_leaf: 0,
            pinned: false,
            accent: None,
        }
    }

    pub fn leaf_count(&self) -> usize {
        self.root.leaf_count()
    }

    pub fn focused_session(&self) -> Option<&PtySession> {
        self.root.leaf_session(self.focused_leaf)
    }

    pub fn focused_session_mut(&mut self) -> Option<&mut PtySession> {
        self.root.leaf_session_mut(self.focused_leaf)
    }

    pub fn leaf_session(&self, leaf_id: usize) -> Option<&PtySession> {
        self.root.leaf_session(leaf_id)
    }

    pub fn leaf_session_mut(&mut self, leaf_id: usize) -> Option<&mut PtySession> {
        self.root.leaf_session_mut(leaf_id)
    }

    pub fn running_since(&self) -> Option<Instant> {
        (0..self.leaf_count())
            .filter_map(|i| self.root.leaf_running_since(i).and_then(|o| o))
            .min()
    }

    pub fn refresh_running_since(&mut self, heuristic_ms: u64) {
        let now = Instant::now();
        let count = self.leaf_count();
        for leaf in 0..count {
            let busy = self
                .root
                .leaf_session(leaf)
                .map(|session| session.is_busy(heuristic_ms))
                .unwrap_or(false);
            if let Some(running) = self.root.leaf_running_since_mut(leaf) {
                if busy {
                    if running.is_none() {
                        *running = Some(now);
                    }
                } else {
                    *running = None;
                }
            }
        }
    }

    pub fn current_working_directory(&self) -> Option<PathBuf> {
        self.root.focused_cwd(self.focused_leaf)
    }
}

impl TabManager {
    pub fn empty() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
        }
    }

    pub fn with_initial(tab: Tab) -> Self {
        Self {
            tabs: vec![tab],
            active: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.tabs.get(self.active).map(|tab| tab.id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
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

    pub fn infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabInfo {
                id: tab.id,
                title: tab.title.clone(),
                cwd: tab.current_working_directory(),
                active: index == self.active,
                running_since: tab.running_since(),
                pinned: tab.pinned,
                accent: tab.accent,
            })
            .collect()
    }

    pub fn spawn_tab(
        &mut self,
        config: &Config,
        layout: TerminalLayout,
        proxy: EventLoopProxyFactory,
        next_tab_id: &mut usize,
        working_directory: Option<PathBuf>,
    ) -> Option<TabId> {
        let id = TabId(*next_tab_id);
        *next_tab_id += 1;

        let title = format!("Tab {}", id.0);
        tracing::info!(
            cols = layout.cols,
            rows = layout.rows,
            cwd = ?working_directory,
            "spawning new tab"
        );
        match spawn_leaf_session(config, id, 0, layout, &proxy, working_directory) {
            Ok(session) => {
                let tab = Tab::new(id, title, session);
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

    pub fn take_tab(&mut self, id: TabId) -> Option<Tab> {
        let index = self.tab_index(id)?;
        let tab = self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }

        Some(tab)
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn tab_at_index(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index)
    }

    pub fn tab_at_index_mut(&mut self, index: usize) -> Option<&mut Tab> {
        self.tabs.get_mut(index)
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

    pub fn reorder_tab(&mut self, from_id: TabId, to_index: usize) -> bool {
        let Some(from_index) = self.tab_index(from_id) else {
            return false;
        };
        let active_id = self.active_id();
        let tab = self.tabs.remove(from_index);
        let mut insert_at = to_index.min(self.tabs.len());
        if from_index < insert_at {
            insert_at = insert_at.saturating_sub(1);
        }
        self.tabs.insert(insert_at, tab);
        if let Some(id) = active_id {
            if let Some(idx) = self.tab_index(id) {
                self.active = idx;
            }
        }
        true
    }

    pub fn toggle_pin(&mut self, id: TabId) {
        if let Some(tab) = self.tab_by_id_mut(id) {
            tab.pinned = !tab.pinned;
        }
    }

    pub fn cycle_tab_color(&mut self, id: TabId) {
        const PRESETS: [[u8; 3]; 6] = [
            [231, 76, 60],
            [46, 204, 113],
            [52, 152, 219],
            [155, 89, 182],
            [241, 196, 15],
            [230, 126, 34],
        ];
        if let Some(tab) = self.tab_by_id_mut(id) {
            tab.accent = match tab.accent {
                None => Some(PRESETS[0]),
                Some(current) => {
                    let idx = PRESETS.iter().position(|c| *c == current).unwrap_or(0);
                    PRESETS.get((idx + 1) % (PRESETS.len() + 1)).copied()
                }
            };
        }
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

    pub fn for_tab(&self, tab_id: TabId, leaf_id: usize) -> EventProxy {
        EventProxy::new(self.proxy.clone(), self.window_id, tab_id, leaf_id)
    }
}

use std::time::Duration;

use crate::tabs::TabId;

#[derive(Clone, Debug)]
pub struct TabBarEntry {
    pub id: TabId,
    pub title: String,
    pub cwd: Option<String>,
    pub active: bool,
    /// A foreground command is currently running in this tab.
    pub busy: bool,
    /// How long the current command has been running (when `busy`).
    pub elapsed: Option<Duration>,
    pub pinned: bool,
    pub accent: Option<[u8; 3]>,
}

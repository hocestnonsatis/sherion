mod chrome;
pub mod frame;
mod glyph_cache;
mod layout;
mod menu;
mod perf;
mod renderer;
mod resize;
mod scene;
mod tab_bar;
mod tab_strip;
mod text;
mod theme;
mod title_bar;

pub use chrome::ChromeMetrics;
pub use layout::{grid_dims, pane_rects, ContentRect, TerminalLayout, MAX_GRID_PANES};
pub use menu::{MenuAction, MenuEntry, MenuHit, MenuLayout};
pub use perf::{PerfStatsSnapshot, RenderTimings};
pub use renderer::{
    ChromeFrame, CommandPaletteFrame, PaneRender, RenameOverlayFrame, Renderer, SearchOverlayFrame,
    SearchOverlayHit,
};
pub use resize::{border_size, cursor_for_direction, resize_direction_at};
pub use tab_bar::TabBarEntry;
pub use tab_strip::{
    collapsed_sidebar_width, sidebar_width_bounds, TabStripHit, TabStripLayout,
    SIDEBAR_COLLAPSED_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH,
};
pub use title_bar::{TitleBarHit, TitleBarLayout, TitleBarMetrics};

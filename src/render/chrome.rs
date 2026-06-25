use crate::render::tab_strip::TabStripMetrics;
use crate::render::title_bar::TitleBarMetrics;

#[derive(Clone, Copy, Debug)]
pub struct ChromeMetrics {
    pub title_bar: TitleBarMetrics,
    pub tab_strip: TabStripMetrics,
}

impl ChromeMetrics {
    pub fn from_font_size(font_size: f32, scale: f32) -> Self {
        Self {
            title_bar: TitleBarMetrics::from_font_size(font_size, scale),
            tab_strip: TabStripMetrics::from_font_size(font_size, scale),
        }
    }

    pub fn content_offset_x(&self) -> f32 {
        self.tab_strip.width
    }

    pub fn content_offset_y(&self) -> f32 {
        self.title_bar.height
    }
}

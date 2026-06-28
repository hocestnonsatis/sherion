use alacritty_terminal::event::{Event as TerminalEvent, EventListener};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::tabs::TabId;

#[derive(Debug, Clone)]
pub enum UserEvent {
    Terminal {
        window_id: WindowId,
        tab_id: TabId,
        event: TerminalEvent,
    },
}

#[derive(Debug, Clone)]
pub struct EventProxy {
    proxy: EventLoopProxy<UserEvent>,
    #[allow(dead_code)]
    window_id: WindowId,
    tab_id: TabId,
}

impl EventProxy {
    pub fn new(proxy: EventLoopProxy<UserEvent>, window_id: WindowId, tab_id: TabId) -> Self {
        Self {
            proxy,
            window_id,
            tab_id,
        }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: TerminalEvent) {
        let _ = self.proxy.send_event(UserEvent::Terminal {
            window_id: self.window_id,
            tab_id: self.tab_id,
            event,
        });
    }
}

use std::sync::Arc;
use std::thread::JoinHandle;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{self, Options as PtyOptions, Shell};
use anyhow::{Context, Result};
use parking_lot::Mutex;

use crate::config::Config;
use crate::event::EventProxy;
use crate::render::TerminalLayout;

pub struct PtySession {
    pub terminal: Arc<FairMutex<Term<EventProxy>>>,
    pub notifier: Notifier,
    io_thread: Mutex<Option<JoinHandle<()>>>,
}

impl PtySession {
    pub fn spawn(
        config: &Config,
        layout: TerminalLayout,
        event_proxy: EventProxy,
    ) -> Result<Self> {
        let term_config = config.term_config();
        let terminal = Term::new(term_config, &layout, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        let mut pty_options = PtyOptions::default();
        pty_options.env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        pty_options.env.insert("COLORTERM".to_owned(), "truecolor".to_owned());

        if !config.terminal.shell.is_empty() {
            pty_options.shell = Some(Shell::new(config.terminal.shell.clone(), Vec::new()));
        }

        let window_size = layout.window_size();
        let pty = tty::new(&pty_options, window_size, 0).context("failed to create PTY")?;

        let event_loop = EventLoop::new(
            Arc::clone(&terminal),
            event_proxy,
            pty,
            pty_options.drain_on_exit,
            false,
        )
        .context("failed to create PTY event loop")?;

        let notifier = Notifier(event_loop.channel());
        let io_thread = event_loop.spawn();

        let handle = std::thread::spawn(move || {
            let _ = io_thread.join();
        });

        Ok(Self {
            terminal,
            notifier,
            io_thread: Mutex::new(Some(handle)),
        })
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
        if let Some(handle) = self.io_thread.lock().take() {
            let _ = handle.join();
        }
    }
}

impl TerminalLayout {
    pub fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: self.cell_width.round().max(1.0) as u16,
            cell_height: self.cell_height.round().max(1.0) as u16,
        }
    }
}

impl Dimensions for TerminalLayout {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

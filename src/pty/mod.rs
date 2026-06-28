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
    #[cfg(unix)]
    process: Option<ProcessHandle>,
}

/// Captured handles used to tell whether a foreground command is running in the
/// tab. The shell creates its own process group (it calls `setsid`), so its
/// pgid equals its pid. When the user launches a command, the shell puts it in
/// a new foreground process group; comparing the terminal's foreground pgid
/// against the shell's pid tells us if something is actively running.
#[cfg(unix)]
struct ProcessHandle {
    shell_pid: i32,
    /// A private dup of the PTY master fd, kept open for the lifetime of the
    /// session so `tcgetpgrp` stays valid independent of the IO thread.
    master_fd: std::os::fd::OwnedFd,
}

impl PtySession {
    pub fn spawn(config: &Config, layout: TerminalLayout, event_proxy: EventProxy) -> Result<Self> {
        Self::spawn_with_working_directory(config, layout, event_proxy, None)
    }

    pub fn spawn_with_working_directory(
        config: &Config,
        layout: TerminalLayout,
        event_proxy: EventProxy,
        working_directory: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let term_config = config.term_config();
        let terminal = Term::new(term_config, &layout, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        let mut pty_options = PtyOptions::default();
        pty_options
            .env
            .insert("TERM".to_owned(), "xterm-256color".to_owned());
        pty_options
            .env
            .insert("COLORTERM".to_owned(), "truecolor".to_owned());
        pty_options.working_directory = working_directory;

        if !config.terminal.shell.is_empty() {
            pty_options.shell = Some(Shell::new(config.terminal.shell.clone(), Vec::new()));
        }

        let window_size = layout.window_size();
        let pty = tty::new(&pty_options, window_size, 0).context("failed to create PTY")?;

        // Capture the shell pid and a private dup of the master fd before the PTY
        // is moved into the IO event loop, so we can poll the foreground process
        // group later for the "tab is busy" indicator.
        #[cfg(unix)]
        let process = {
            use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
            let shell_pid = pty.child().id() as i32;
            let master_fd = pty.file().as_raw_fd();
            let dup_fd = unsafe { libc::dup(master_fd) };
            if dup_fd >= 0 {
                Some(ProcessHandle {
                    shell_pid,
                    master_fd: unsafe { OwnedFd::from_raw_fd(dup_fd) },
                })
            } else {
                None
            }
        };

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
            #[cfg(unix)]
            process,
        })
    }

    /// Best-effort current directory for the shell process backing this tab.
    pub fn current_working_directory(&self) -> Option<std::path::PathBuf> {
        #[cfg(unix)]
        {
            let process = self.process.as_ref()?;
            let path = std::fs::read_link(format!("/proc/{}/cwd", process.shell_pid)).ok()?;
            path.is_dir().then_some(path)
        }

        #[cfg(not(unix))]
        {
            None
        }
    }

    /// Whether a foreground command (i.e. not the idle shell prompt) is running.
    pub fn is_busy(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if let Some(process) = self.process.as_ref() {
                let fg = unsafe { libc::tcgetpgrp(process.master_fd.as_raw_fd()) };
                return fg > 0 && fg != process.shell_pid;
            }
        }
        false
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

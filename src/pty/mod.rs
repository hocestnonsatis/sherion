use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::thread::JoinHandle;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{self, Options as PtyOptions, Shell};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use vte::ansi::ModifyOtherKeys;

use crate::config::Config;
use crate::event::EventProxy;
use crate::render::TerminalLayout;
use crate::security::validate_spawn_cwd;
use crate::terminal_setup;

#[cfg(windows)]
mod windows;

mod busy;
mod output_tap;
mod tap;
use tap::TappingPty;

pub struct PtySession {
    pub terminal: Arc<FairMutex<Term<EventProxy>>>,
    pub notifier: Notifier,
    io_thread: Mutex<Option<JoinHandle<()>>>,
    reported_cwd: Arc<Mutex<Option<PathBuf>>>,
    modify_other_keys: Arc<Mutex<ModifyOtherKeys>>,
    #[cfg(unix)]
    process: Option<ProcessHandle>,
    /// Updated on PTY output; used for non-Unix busy heuristics.
    #[cfg_attr(unix, allow(dead_code))]
    last_output_at: Arc<Mutex<Option<Instant>>>,
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
        let mut term = Term::new(term_config, &layout, event_proxy.clone());
        terminal_setup::apply_config_to_term(config, &mut term);
        let terminal = Arc::new(FairMutex::new(term));

        let mut pty_options = PtyOptions::default();
        pty_options
            .env
            .insert("TERM".to_owned(), "xterm-256color".to_owned());
        pty_options
            .env
            .insert("COLORTERM".to_owned(), "truecolor".to_owned());
        pty_options.working_directory = working_directory.and_then(|path| {
            validate_spawn_cwd(&path).or_else(|| {
                tracing::warn!(path = %path.display(), "ignoring invalid spawn working directory");
                None
            })
        });

        if !config.terminal.shell.is_empty() {
            pty_options.shell = Some(Shell::new(
                config.terminal.shell.clone(),
                config.terminal.shell_args.clone(),
            ));
        }

        #[cfg(windows)]
        if !config.terminal.shell_args.is_empty() {
            pty_options.escape_args = true;
        }

        let window_size = layout.window_size();
        let reported_cwd = Arc::new(Mutex::new(None));
        let modify_other_keys = Arc::new(Mutex::new(ModifyOtherKeys::Reset));
        let last_output_at = Arc::new(Mutex::new(None));
        let pty_notifier = Arc::new(Mutex::new(None::<Notifier>));

        let inner = tty::new(&pty_options, window_size, 0).context("failed to create PTY")?;
        #[cfg(windows)]
        windows::on_pty_created();

        let pty = TappingPty::new(
            inner,
            Arc::clone(&reported_cwd),
            Arc::clone(&modify_other_keys),
            Arc::clone(&pty_notifier),
            Arc::clone(&last_output_at),
        )
        .context("failed to wrap PTY")?;

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
        *pty_notifier.lock() = Some(Notifier(event_loop.channel()));
        let io_thread = event_loop.spawn();

        let handle = std::thread::spawn(move || {
            let _ = io_thread.join();
        });

        Ok(Self {
            terminal,
            notifier,
            io_thread: Mutex::new(Some(handle)),
            reported_cwd,
            modify_other_keys,
            #[cfg(unix)]
            process,
            last_output_at,
        })
    }

    pub fn modify_other_keys(&self) -> ModifyOtherKeys {
        *self.modify_other_keys.lock()
    }

    /// Best-effort current directory for the shell process backing this tab.
    pub fn current_working_directory(&self) -> Option<std::path::PathBuf> {
        if let Some(cwd) = self.reported_cwd.lock().clone() {
            return Some(cwd);
        }
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

    pub fn reported_cwd_handle(&self) -> Arc<Mutex<Option<PathBuf>>> {
        Arc::clone(&self.reported_cwd)
    }

    /// Whether a foreground command (i.e. not the idle shell prompt) is running.
    ///
    /// On Unix this compares the PTY foreground process group against the shell.
    /// On other platforms this uses a best-effort recent-output activity heuristic.
    pub fn is_busy(&self, heuristic_ms: u64) -> bool {
        #[cfg(unix)]
        {
            let _ = heuristic_ms;
            use std::os::fd::AsRawFd;
            if let Some(process) = self.process.as_ref() {
                let fg = unsafe { libc::tcgetpgrp(process.master_fd.as_raw_fd()) };
                return fg > 0 && fg != process.shell_pid;
            }
            return false;
        }

        #[cfg(not(unix))]
        {
            let last = *self.last_output_at.lock();
            busy::recent_output_is_busy(last, Instant::now(), heuristic_ms)
        }
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

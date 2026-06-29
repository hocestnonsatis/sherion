use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::OnResize;
use alacritty_terminal::event_loop::{Msg, Notifier};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use parking_lot::Mutex;
use polling::{Event, PollMode, Poller};
use vte::ansi::ModifyOtherKeys;

use crate::pty::output_tap::{PtyOutputTap, TapEvent};

/// PTY wrapper that scans output for OSC 7 and modifyOtherKeys sequences.
pub struct TappingPty {
    inner: tty::Pty,
    io: TapFile,
}

pub struct TapFile {
    file: std::fs::File,
    tap: PtyOutputTap,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    modify_other_keys: Arc<Mutex<ModifyOtherKeys>>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    last_output_at: Arc<Mutex<Option<Instant>>>,
}

impl TappingPty {
    pub fn new(
        inner: tty::Pty,
        cwd: Arc<Mutex<Option<PathBuf>>>,
        modify_other_keys: Arc<Mutex<ModifyOtherKeys>>,
        notifier: Arc<Mutex<Option<Notifier>>>,
        last_output_at: Arc<Mutex<Option<Instant>>>,
    ) -> io::Result<Self> {
        let file = inner.file().try_clone()?;
        Ok(Self {
            inner,
            io: TapFile {
                file,
                tap: PtyOutputTap::default(),
                cwd,
                modify_other_keys,
                notifier,
                last_output_at,
            },
        })
    }

    pub fn child(&self) -> &std::process::Child {
        self.inner.child()
    }

    pub fn file(&self) -> &std::fs::File {
        self.inner.file()
    }
}

impl TapFile {
    fn handle_tap_events(&self, events: Vec<TapEvent>) {
        for event in events {
            match event {
                TapEvent::Cwd(path) => {
                    *self.cwd.lock() = Some(path);
                }
                TapEvent::ModifyOtherKeys(mode) => {
                    *self.modify_other_keys.lock() = mode;
                }
                TapEvent::QueryModifyOtherKeys => {
                    let mode = *self.modify_other_keys.lock();
                    let pv = match mode {
                        ModifyOtherKeys::Reset => 0,
                        ModifyOtherKeys::EnableExceptWellDefined => 1,
                        ModifyOtherKeys::EnableAll => 2,
                    };
                    let response = format!("\x1b[>4;{pv}m");
                    if let Some(notifier) = self.notifier.lock().as_ref() {
                        let _ = notifier.0.send(Msg::Input(
                            std::borrow::Cow::Owned(response.into_bytes()),
                        ));
                    }
                }
            }
        }
    }
}

impl Read for TapFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        if n > 0 {
            *self.last_output_at.lock() = Some(Instant::now());
            let events = self.tap.feed(&buf[..n]);
            self.handle_tap_events(events);
        }
        Ok(n)
    }
}

impl Write for TapFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl EventedReadWrite for TappingPty {
    type Reader = TapFile;
    type Writer = TapFile;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        unsafe { self.inner.register(poll, interest, poll_opts) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poll, interest, poll_opts)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.io
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.io
    }
}

impl EventedPty for TappingPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for TappingPty {
    fn on_resize(&mut self, window_size: alacritty_terminal::event::WindowSize) {
        self.inner.on_resize(window_size);
    }
}

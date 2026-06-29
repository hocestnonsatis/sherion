use std::cell::RefCell;

use arboard::Clipboard;

#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

thread_local! {
    /// A persistent clipboard handle kept alive for the lifetime of the UI
    /// thread. On Linux the owning process must stay alive to serve clipboard
    /// contents to other applications; dropping a fresh `Clipboard` right after
    /// `set_text` loses the data and triggers arboard's "dropped very quickly"
    /// warning. Reusing one instance keeps the selection available.
    static CLIPBOARD: RefCell<Option<Clipboard>> = RefCell::new(Clipboard::new().ok());
}

fn with_clipboard<T>(f: impl FnOnce(&mut Clipboard) -> T) -> Option<T> {
    CLIPBOARD.with(|cell| {
        let mut guard = cell.borrow_mut();
        if guard.is_none() {
            *guard = Clipboard::new().ok();
        }
        guard.as_mut().map(f)
    })
}

pub fn copy_text(text: &str) -> bool {
    with_clipboard(|cb| cb.set_text(text.to_owned()).is_ok()).unwrap_or(false)
}

pub fn paste_text() -> Option<String> {
    with_clipboard(|cb| cb.get_text().ok()).flatten()
}

/// Copy to the X11/Wayland primary selection (middle-click paste source).
pub fn copy_primary(text: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        return with_clipboard(|cb| {
            cb.set()
                .clipboard(LinuxClipboardKind::Primary)
                .text(text.to_owned())
                .is_ok()
        })
        .unwrap_or(false);
    }

    #[cfg(not(target_os = "linux"))]
    {
        copy_text(text)
    }
}

/// Read from the primary selection; falls back to the regular clipboard.
pub fn paste_primary() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(text) =
            with_clipboard(|cb| cb.get().clipboard(LinuxClipboardKind::Primary).text().ok())
                .flatten()
        {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }

    paste_text()
}

/// Strip dangerous control characters from pasted text while preserving newlines and tabs.
pub fn sanitize_paste(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect()
}

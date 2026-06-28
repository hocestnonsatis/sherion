use std::cell::RefCell;

use arboard::Clipboard;

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

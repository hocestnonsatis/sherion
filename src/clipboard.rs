use arboard::Clipboard;

pub fn copy_text(text: &str) -> bool {
    Clipboard::new()
        .and_then(|mut cb| cb.set_text(text.to_owned()))
        .is_ok()
}

pub fn paste_text() -> Option<String> {
    Clipboard::new()
        .ok()?
        .get_text()
        .ok()
}

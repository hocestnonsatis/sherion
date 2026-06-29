//! Windows ConPTY integration hooks.
//!
//! Sherion uses `alacritty_terminal`'s portable PTY backend, which wraps
//! `CreatePseudoConsole` with passthrough and Win32 input mode on Windows.
//! This module centralizes any Windows-specific PTY lifecycle notes.

/// Called after the ConPTY instance is created via the portable PTY backend.
pub fn on_pty_created() {
    tracing::debug!("ConPTY session created via portable-pty backend");
}

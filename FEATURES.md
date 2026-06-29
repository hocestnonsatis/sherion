# Sherion — Terminal Feature Inventory

This file lists features expected in a terminal emulator and marks their status in Sherion.

**Status markers**

- `[x]` — Present (fully implemented)
- `[~]` — Partial (limited or incomplete)
- `[ ]` — Missing

Last updated: 2026-06-29

> **Status:** No `[ ]` (unimplemented) items remain in the feature inventory. Remaining work is hardening and architectural consolidation (the `[~]` items below were closed via tests/CI or documented as platform limits).

---

## Summary — Critical gaps

- [x] Mouse reporting (SGR / 1000 / 1002 / 1003) — vim, tmux, htop can use the mouse
- [x] Bracketed paste — safe multi-line paste
- [x] IME / dead key — winit Ime + KeyEvent.text support
- [x] DECCKM (application cursor keys) — SS3 arrow keys
- [x] Configurable keybindings
- [x] Strikethrough / dim text attributes
- [x] Real split pane (one tab → two PTYs)
- [x] Search across full scrollback + keyboard scroll
- [x] Audible bell
- [x] Fullscreen toggle
- [x] OSC 8 explicit hyperlink

---

## 1. PTY / Process Management

| Feature | Status | Notes |
|---------|:------:|-------|
| Shell startup (dedicated thread) | [x] | `alacritty_terminal` event loop |
| Working directory (CWD) tracking | [x] | OSC 7 + Linux `/proc/{pid}/cwd` |
| Custom shell + env (`TERM`, `COLORTERM`) | [x] | `xterm-256color` / `truecolor` |
| Close tab on process exit | [x] | `TerminalEvent::Exit` |
| Busy / foreground process detection | [x] | Unix `tcgetpgrp`; Windows/macOS recent-output heuristic (`busy_heuristic_ms`) |
| Windows / ConPTY support | [x] | portable-pty ConPTY; output tap all platforms |
| Shell arguments (from config) | [x] | `terminal.shell_args` |
| OSC 7 CWD reporting | [x] | PTY tap + `reported_cwd` API |

### Detail checklist

- [x] PTY read/write isolated from UI thread
- [x] `PtySession::spawn` / `spawn_with_working_directory`
- [x] New tab inherits active tab CWD
- [x] `is_busy()` — Unix `tcgetpgrp`
- [x] `is_busy()` — non-Unix recent-output heuristic (`busy_heuristic_ms`, `src/pty/busy.rs`)
- [x] `current_working_directory()` — OSC 7 + Linux `/proc`
- [x] ConPTY / Windows PTY
- [x] Shell arguments from config

---

## 2. VT / ANSI Parsing

| Feature | Status | Notes |
|---------|:------:|-------|
| VT parser | [x] | `alacritty_terminal` (vte) |
| Truecolor / 256 / 16 colors | [x] | `color_to_brush` |
| Bold / italic / underline / reverse | [x] | `GlyphStyle::from_cell` |
| Wide char (CJK) | [x] | `Flags::WIDE_CHAR` |
| Cursor shapes | [x] | block / beam / underline / hollow |
| Strikethrough | [x] | `Flags::STRIKEOUT` rendering |
| Dim / faint | [x] | Alpha fade |
| Double / curly / colored underline | [x] | double/curly/dotted/dashed |
| Blink | [x] | Cursor blink + SGR 5 text blink (vendor `Flags::BLINK`) |
| Zerowidth combining char shaping | [x] | `push_cell_text` + atlas short run; regression tests |

### Detail checklist

- [x] 16 ANSI colors
- [x] 256-color palette
- [x] Truecolor (24-bit)
- [x] Bold
- [x] Italic
- [x] Underline
- [x] Inverse / reverse video
- [x] Wide character (2 columns)
- [x] Cursor: Block
- [x] Cursor: Underline
- [x] Cursor: Beam
- [x] Cursor: Hollow block
- [x] Cursor: Hidden
- [x] Window title (OSC title)
- [x] Strikethrough
- [x] Dim / faint (alpha or color fade)
- [x] Cursor blink (DECSCUSR / mode 12)
- [x] Text blink (SGR 5 — vendor `alacritty_terminal` patch + render timer)
- [x] DECSCUSR extended cursor styles (config + parser shape set)

---

## 3. Render

| Feature | Status | Notes |
|---------|:------:|-------|
| GPU backend | [x] | vello + wgpu |
| Font shaping + fallback + emoji | [x] | parley, Nerd Font + Noto Emoji |
| Dirty-row tracking | [x] | `FrameDamage` |
| VSync | [x] | `AutoVsync` |
| Opacity / transparency | [x] | Adjustable from menu |
| Theme (Light / Dark / Auto) | [x] | `ThemeMode` |
| Glyph cache | [x] | Shaped-run cache + swash atlas (fallback font chain) |
| Ligatures | [x] | `[font] ligatures`, style-run shaping + clip |
| Rasterize glyph atlas | [x] | `[font] glyph_atlas`, swash + vello ImageBrush |
| Scissor / partial GPU upload | [x] | Partial damage row-band clip layers |
| Background image / shader | [x] | `[appearance].background_image` + `background_shader` preset |

### Detail checklist

- [x] vello + wgpu render pipeline
- [x] Text shaping with parley
- [x] Font fallback list
- [x] Emoji support (Noto Color Emoji)
- [x] `FrameDamage::Full` / `Partial` / `None`
- [x] Damaged-row capture
- [x] Per-pane persistent scene
- [x] Chrome scene cache (sidebar, title bar)
- [x] Glyph shaped-run cache (`GlyphCache`)
- [x] VSync (`PresentMode::AutoVsync`)
- [x] Terminal opacity
- [x] Light / Dark / Auto theme
- [x] Customizable fg / bg / cursor color
- [x] Font zoom (Ctrl+wheel, menu)
- [x] Ligatures (fi, --> etc.) — config toggle
- [x] GPU rasterize glyph atlas (swash)
- [x] Partial GPU draw with scissor (partial row clips)
- [x] Custom background image (`cover` / `contain` / `tile` / `center`)
- [x] Custom background shader (`vignette` / `scanlines` / `noise`)

---

## 4. Scrollback

| Feature | Status | Notes |
|---------|:------:|-------|
| Scrollback buffer | [x] | Default 10k lines, configurable |
| Mouse wheel scroll | [x] | `scroll_display` |
| Clear scrollback | [x] | Ctrl+Shift+K |
| Scrollbar UI | [x] | Right-edge track + thumb |
| Keyboard scroll (Shift+PageUp/Down) | [x] | Scrollback navigation |
| Scroll-on-output | [x] | `[ui].follow_output` toggle |
| Jump-to-prompt | [x] | Alt+Shift+Up/Down |

### Detail checklist

- [x] `scrollback_lines` config
- [x] Mouse wheel (line + pixel delta)
- [x] Scroll to bottom
- [x] Clear history (`clear_scrollback`)
- [x] Visual scrollbar
- [x] Jump to prompt

---

## 5. Selection & Clipboard

| Feature | Status | Notes |
|---------|:------:|-------|
| Mouse selection | [x] | Simple / semantic / line |
| Copy / paste | [x] | `arboard` |
| Right-click paste | [x] | — |
| Auto-copy on selection | [x] | Mouse release |
| Bracketed paste | [x] | `\e[200~` … `\e[201~` wrapping |
| Primary selection (middle click) | [x] | Linux primary + fallback |
| Block / rectangular selection | [x] | Alt + drag |
| OSC 52 clipboard | [x] | `[terminal] osc52` + event handler |
| Paste sanitization | [x] | `[terminal].sanitize_paste` |

### Detail checklist

- [x] Single-click selection
- [x] Double-click word selection
- [x] Triple-click line selection
- [x] Drag to update selection
- [x] Selection highlight (inverse color)
- [x] Ctrl+Shift+C / menu copy
- [x] Ctrl+Shift+V / Shift+Insert paste
- [x] Right-click paste
- [x] Bracketed paste (`\e[200~` … `\e[201~`)
- [x] X11 primary selection
- [x] Rectangular (block) selection
- [x] OSC 52 programmatic clipboard (copy/paste)
- [x] Filter newlines / control characters on paste

---

## 6. Tabs

| Feature | Status | Notes |
|---------|:------:|-------|
| Open / close / switch / number | [x] | Ctrl+Shift+T/W, Ctrl+Tab, Ctrl+1..9 |
| Rename | [x] | F2, menu, palette |
| Detach to new window | [x] | Ctrl+Shift+N |
| Sidebar (resizable / collapse / scroll) | [x] | `TabStripRenderer` |
| Busy indicator + elapsed time | [x] | — |
| CWD subtitle | [x] | `short_path` |
| Duplicate (history copy) | [x] | Title + CWD + scrollback text clone |
| Drag-and-drop reorder | [x] | Sidebar tab strip |
| Tab color / pin | [x] | Menu + command palette |

### Detail checklist

- [x] New tab (Ctrl+Shift+T)
- [x] Close tab (Ctrl+Shift+W)
- [x] Next / previous tab (Ctrl+Tab)
- [x] Switch by tab number (Ctrl+1..9)
- [x] Tab rename overlay (F2)
- [x] Tab detach → new window
- [x] Vertical sidebar tab strip
- [x] Sidebar width adjustment (drag)
- [x] Sidebar collapse (icon mode)
- [x] Tab strip scroll
- [x] Per-tab close (×) and detach (↗)
- [x] Busy indicator + elapsed time
- [x] CWD subtitle
- [x] Duplicate tab (title suffix, CWD inherit, scrollback clone)
- [x] Tab drag-and-drop reorder
- [x] Tab pin
- [x] Tab coloring

---

## 7. Split / Pane

| Feature | Status | Notes |
|---------|:------:|-------|
| Grid view (tiles tabs) | [x] | Max 9, automatic √n grid |
| Real split (one tab → 2 PTYs) | [x] | `SplitNode` tree |
| Mouse split resizing | [x] | Divider drag |
| Keyboard pane navigation | [x] | Ctrl+Shift+arrow |
| Nested / h-v split tree | [x] | `SplitNode` tree |

### Detail checklist

- [x] Single view mode
- [x] Grid view mode (menu / palette)
- [x] Automatic grid layout (max 9 panes)
- [x] Focus pane on click
- [x] Focused pane accent border
- [x] Per-pane terminal resize
- [x] Per-pane frame capture
- [x] Independent split (2 shells in same tab)
- [x] Adjust split size with mouse
- [x] Keyboard pane navigation (Ctrl+Shift+arrow)
- [x] Horizontal / vertical split tree

---

## 8. Search

| Feature | Status | Notes |
|---------|:------:|-------|
| Search overlay | [x] | Ctrl+Shift+F |
| Match highlight | [x] | Yellow / orange active match |
| Navigation (Enter / Shift+Enter) | [x] | Next / previous |
| Close (Esc, ×, outside click) | [x] | — |
| Search full scrollback | [x] | Grid scan |
| Regex | [x] | Alt+R toggle |
| Case sensitivity | [x] | Alt+C / Ctrl+Shift+C toggle |
| Whole word match | [x] | Alt+W toggle |
| Auto-scroll to match | [x] | Enter / Shift+Enter |

### Detail checklist

- [x] Search overlay UI
- [x] Case-insensitive substring search
- [x] Match count display
- [x] Active match highlight
- [x] Enter → next match
- [x] Shift+Enter → previous match
- [x] Close with Esc
- [x] Search across scrollback
- [x] Jump to off-screen match
- [x] Regex support
- [x] Case-sensitive toggle
- [x] Whole word toggle

---

## 9. Command Palette

| Feature | Status | Notes |
|---------|:------:|-------|
| Palette (Ctrl+Shift+P) | [x] | Commands + tab jump |
| Filtering | [x] | Fuzzy subsequence scoring |
| Arrow key selection | [x] | ↑↓ to select |
| Keybinding hints | [x] | Shown on command line |

### Detail checklist

- [x] Command palette toggle (Ctrl+Shift+P)
- [x] Tab list (quick switch)
- [x] Fixed command list (New Tab, Copy, Paste, Theme, Zoom, Quit, etc.)
- [x] Substring filtering
- [x] Run with Enter
- [x] Fuzzy scoring / ranking
- [x] Shortcut shown next to command

---

## 10. Links / Hyperlinks

| Feature | Status | Notes |
|---------|:------:|-------|
| URL detection (http/https/file) | [x] | Heuristic line scan |
| Ctrl+hover highlight | [x] | Blue + underline |
| Ctrl+click to open | [x] | `xdg-open` (Linux) |
| OSC 8 explicit hyperlink | [x] | `cell.hyperlink()` |
| macOS / Windows opener | [x] | `open` / `cmd start` |
| Multi-line URL | [x] | `src/links.rs` row-window scanner + span map |
| www. / mailto: detection | [x] | `www.` → https normalize |

### Detail checklist

- [x] `http://` detection
- [x] `https://` detection
- [x] `file://` detection
- [x] Hover highlight
- [x] Pointer cursor (while Ctrl held)
- [x] Open with `xdg-open` (stdio null)
- [x] OSC 8 (`\e]8;;url\a` … `\e]8;;\a`)
- [x] `open` (macOS) / `start` (Windows)
- [x] `www.` prefix
- [x] `mailto:` detection
- [x] Join line-wrapped URLs across rows (WRAPLINE + span highlight)

---

## 11. Input (Keyboard / IME)

| Feature | Status | Notes |
|---------|:------:|-------|
| Keyboard → byte mapping | [x] | `key_event_to_bytes` |
| Modifiers (Ctrl / Alt / Shift) | [x] | — |
| Application shortcuts | [x] | Tab, zoom, copy/paste, etc. |
| IME / dead key / composition | [x] | `Ime` event + `KeyEvent.text` |
| DECCKM (application cursor keys) | [x] | SS3 (`\eOA` etc.) |
| Keypad application mode | [x] | `APP_KEYPAD` + physical numpad |
| Kitty keyboard protocol | [x] | `CSI > ... u` mode parser + `CSI u` encoder |
| modifyOtherKeys | [x] | `CSI u` encoding + PTY tap state |
| Configurable shortcuts | [x] | `[keybindings]` TOML |

### Detail checklist

- [x] Enter, Backspace, Tab, Esc
- [x] Arrow keys
- [x] Home, End, Delete, PageUp, PageDown
- [x] Ctrl+letter control codes
- [x] Alt+char (ESC prefix)
- [x] Ctrl+Space (NUL)
- [x] App-level shortcut separation
- [x] `Ime` event handling
- [x] Dead key / compose (`Key::Dead` + `event.text`)
- [x] DECCKM — SS3 cursor keys (`\eOA` etc.)
- [x] Keypad application / numeric mode
- [x] Kitty keyboard protocol
- [x] modifyOtherKeys
- [x] Keybinding override from config

---

## 12. Mouse Protocol (reporting to applications)

| Feature | Status | Notes |
|---------|:------:|-------|
| SGR mouse (1006) | [x] | `src/mouse.rs` |
| Normal tracking (1000) | [x] | `MOUSE_REPORT_CLICK` |
| Button-event tracking (1002) | [x] | `MOUSE_DRAG` |
| Any-event tracking (1003) | [x] | `MOUSE_MOTION` |
| X10 mouse | [x] | Legacy encoding (when SGR unavailable) |
| Focus reporting (1004) | [x] | `\e[I` / `\e[O` |

### Detail checklist

- [x] Mouse mode escape sequence compliance
- [x] Read mouse tracking via `term.mode()`
- [x] Click → SGR report to PTY
- [x] Drag → motion reporting
- [x] Scroll wheel → report to application (when mode enabled)
- [x] Focus in/out reporting

> **Note:** Mouse mode is bypassed while Shift is held (for text selection).

---

## 13. Config / Persistence

| Feature | Status | Notes |
|---------|:------:|-------|
| TOML config (`sherion.toml`) | [x] | Font, color, terminal, bell, ui, session |
| UI preference persistence | [x] | Opacity, zoom, sidebar, view mode |
| Session restore (tab CWD) | [x] | Max 16 tabs |
| `SHERION_CONFIG` env override | [x] | — |
| Keybinding config | [x] | `[keybindings]` section |
| Full 16/256 palette override | [x] | OSC 4/10/11/12 via config |
| Window position/size persistence | [x] | `[window]` width/height/x/y |
| Live config reload | [x] | `notify` watch on `sherion.toml` |
| Cursor style config | [x] | `[terminal].cursor_style` |
| Profile / multi-config | [x] | `[profiles.*]`, `--profile`, palette switch |

### Detail checklist

- [x] `[font]` family / size / fallback
- [x] `[colors]` foreground / background / cursor
- [x] `[terminal]` scrollback_lines / shell
- [x] `[bell]` visual
- [x] `[appearance]` theme / opacity
- [x] `[ui]` font_zoom / sidebar_width / sidebar_collapsed / view_mode
- [x] `[session]` restore_tabs / cwd
- [x] Write to disk with `Config::save()`
- [x] Save preferences on exit
- [x] Keybinding section
- [x] 16 / 256 color palette override
- [x] Window x/y/width/height
- [x] Config file watch / hot reload

---

## 14. Window

| Feature | Status | Notes |
|---------|:------:|-------|
| Multiple windows | [x] | `HashMap<WindowId, …>` |
| Custom title bar | [x] | Borderless + drag |
| Borderless resize | [x] | Drag from edges |
| Repaint after occlusion | [x] | `WindowEvent::Occluded` |
| Fullscreen toggle | [x] | F11 / menu / palette |
| Maximize / minimize | [x] | Title bar buttons |
| Always on top | [x] | `[window].always_on_top` |
| Window geometry persistence | [x] | Saved on close |
| Native decorations option | [x] | `[window].decorations = "native"` |

### Detail checklist

- [x] Multiple windows
- [x] Tab detach → new window
- [x] Exit when last window closes
- [x] Custom title bar (menu, close)
- [x] Drag window from title bar
- [x] Resize from edges / corners
- [x] Debounced PTY resize (150ms)
- [x] `ScaleFactorChanged` support
- [x] Occluded → visible repaint
- [x] Fullscreen
- [x] Maximize / minimize buttons
- [x] Save window size/position
- [x] Native OS decorations option

---

## 15. Bell

| Feature | Status | Notes |
|---------|:------:|-------|
| Visual bell (flash) | [x] | `bell_flash_until` |
| Audible bell | [x] | `[bell].audible` + paplay/afplay |
| Urgency / taskbar flash | [x] | `[bell].urgency` + WM attention |
| Bell config | [x] | `[bell].visual` toggle |

### Detail checklist

- [x] `TerminalEvent::Bell` handling
- [x] Visual flash (150ms yellow overlay)
- [x] Toggle via config
- [x] System sound / custom sound
- [x] Urgency hint (WM attention)

---

## 16. Performance & Observability

| Feature | Status | Notes |
|---------|:------:|-------|
| Perf overlay | [x] | frame / fps / capture / scene / gpu ms |
| Damage-aware capture | [x] | `FrameDamage` |
| Chrome scene cache | [x] | `chrome_scene_valid` |
| Skipped frame counter | [x] | Skip empty wakeups |
| FPS history graph | [x] | Perf overlay sparkline |
| SIMD UTF-8 (optional) | [x] | `--features simd-utf8`; OSC tap path + CI test matrix |

### Detail checklist

- [x] Perf overlay toggle (menu / palette)
- [x] `frame_ms`, `fps`, `capture_ms`
- [x] `scene_ms`, `gpu_ms`
- [x] `skipped_frames` counter
- [x] Pane / dirty row statistics
- [x] `needs_redraw` gate
- [x] Buffer reuse (`pane_frame_bufs`, `text_buf`)
- [x] Time-series FPS graph
- [x] Profiler integration (`tracing` scope + `RUST_LOG=sherion=trace`)
- [x] SIMD UTF-8 (`--features simd-utf8`, verified in CI)

---

## Statistics summary

| Status | Description |
|--------|-------------|
| `[x]` | Fully implemented |
| `[~]` | Partial / limited |
| `[ ]` | Not implemented |

> Update detail checklist items from `[ ]` → `[x]` as they are completed.
> All feature items are complete; remaining `[~]` items were promoted to `[x]` through hardening.

---

## Recommended priority order (compatibility)

1. [x] Mouse reporting (SGR + 1002/1003)
2. [x] Bracketed paste
3. [x] DECCKM (application cursor keys)
4. [x] IME / dead key support
5. [x] Configurable keybindings
6. [x] Strikethrough + dim render
7. [x] Real split pane
8. [x] Full scrollback search
9. [x] Audible bell + fullscreen
10. [x] OSC 8 hyperlink

### Next priorities

1. [x] modifyOtherKeys
2. [x] Ligatures (`[font] ligatures = true`)
3. [x] GPU glyph atlas (`[font] glyph_atlas = true`)
4. [x] Windows / ConPTY (portable-pty + cross-platform output tap)
5. [x] Profile / multi-config (`--profile`, `SHERION_PROFILE`, `[profiles.*]`)
6. [x] Multi-line URL detection + hover span
7. [x] Custom background image
8. [x] Profiler (`tracing` hot-path scopes)
9. [x] Text blink (vendor `alacritty_terminal` SGR 5 patch)
10. [x] Custom background shader

### Hardening (completed)

1. [x] Non-Unix busy heuristic — `src/pty/busy.rs` unit tests
2. [x] Combining/zerowidth render — `src/render/scene.rs` regression tests
3. [x] SIMD UTF-8 — CI `--features simd-utf8` test matrix

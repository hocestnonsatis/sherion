# Sherion Architecture

Sherion is a GPU-accelerated terminal emulator written in Rust. The codebase is organized around a single main event loop that owns multiple windows, each with its own tabs, PTY sessions, and renderer.

## High-level flow

```
winit event loop (App)
  ├── WindowState (per window)
  │     ├── TabManager → PtySession → alacritty_terminal::Term
  │     └── GpuRenderer (vello + wgpu)
  └── UserEvent channel (PTY output, title changes, …)
```

1. **Input** — keyboard and mouse events are handled on the main thread and written to the active tab's PTY.
2. **PTY reader thread** — each session runs a blocking read loop on the PTY master and forwards bytes to the main thread via an MPSC channel.
3. **Terminal state** — incoming bytes are parsed by `alacritty_terminal`, which maintains the cell grid, scrollback, selection, and cursor.
4. **Render** — on `RedrawRequested`, the active tab's grid is turned into a vello `Scene` and presented through wgpu.

## Threading rules

- Never block on PTY reads in the UI thread.
- PTY writes (keyboard input) should be as direct as possible.
- Terminal parsing and rendering stay on the main thread; only I/O is offloaded.

## Core modules

| Module | Role |
|--------|------|
| `app.rs` | Event loop, multi-window routing, global config/theme |
| `window_state.rs` | Per-window UI state: tabs, layout, input, redraw |
| `tabs.rs` | Tab list, spawn/close/detach |
| `pty/` | Cross-platform pseudo-terminal and reader thread |
| `render/` | Layout, scene building, chrome (title bar, sidebar, menu) |
| `config.rs` | TOML config, colors, font, theme mode |
| `input.rs` | Keybinding → action mapping |
| `clipboard.rs` | Copy/paste via `arboard` |

## Rendering

Terminal text uses **parley** for shaping and **vello** for GPU glyph drawing. Each visible cell is laid out individually; backgrounds and the cursor are drawn as filled rectangles in the same vello scene. Chrome (title bar, tab sidebar, menu) is rendered in the same pass.

Layout (`TerminalLayout`) derives column/row counts from window size, font size, and chrome offsets. Cell dimensions are computed from a fixed font-size ratio, then the grid is stretched to fill the content area exactly.

Presentation path:

1. Build vello `Scene` (terminal grid + chrome)
2. `vello::render_to_texture` → internal surface texture
3. Blit to swapchain texture
4. `present()`

Window transparency is handled by choosing a non-opaque wgpu surface alpha mode and clearing with a transparent base color when terminal opacity is below 1.0.

## Multi-window and tabs

- `App` holds `HashMap<WindowId, WindowState>`.
- Tabs can be detached into a new window (`detach_tab`).
- Each window has its own renderer, layout snapshot, and tab manager.

## Configuration

Config is loaded once at startup from the user's config file. Theme mode (light / dark / auto) and terminal opacity are applied per window. Font zoom is a runtime multiplier on top of the configured font size.

## Performance

Current optimizations:

| Area | Technique |
|------|-----------|
| Render lock | Terminal grid captured into a [`TerminalFrame`](src/render/frame.rs) under a short lock; GPU work runs after the lock is released |
| Damage-aware capture | `term.damage()` drives partial row/column updates; full capture on resize, zoom, theme, or scroll |
| Persistent terminal scenes | Per-pane vello `Scene` kept across frames; partial damage patches only dirty rows |
| Glyph cache | Shaped glyph runs cached by character + style + font size ([`glyph_cache.rs`](src/render/glyph_cache.rs)); foreground brush applied at draw time |
| Text layout | Each cell drawn at its fixed grid column (no run coalescing) to keep the monospace grid exact |
| Allocations | Reused `Scene` (`scene.reset()` for chrome), persistent pane terminal scenes, row buffers, `text_buf`, cached chrome tab entries |
| Font config | `FontFamily` list cached on `SceneBuilder` until config or layout changes |
| GPU | Vello `AaConfig::Area`; surface configure only on resize |
| Busy tabs | Elapsed timer redraws at most once per second |
| Event loop | `needs_redraw` gate; duplicate `request_redraw` coalesced |

Not yet implemented: SIMD UTF-8 (handled inside `alacritty_terminal`), partial GPU/scissor redraw.

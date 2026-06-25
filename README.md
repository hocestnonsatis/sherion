# Sherion

A high-performance Rust terminal emulator built on winit, wgpu, alacritty_terminal, parley, and vello.

## Requirements

- Rust 1.88+
- Linux (primary), Windows (ConPTY via alacritty_terminal)
- GPU with Vulkan, Metal, DX12, or compatible wgpu backend
- A monospace font (DejaVu Sans Mono, Liberation Mono, or Noto Sans Mono)

## Build

```bash
cargo build --release
```

## Run

```bash
cargo run
```

Set log verbosity with `RUST_LOG`:

```bash
RUST_LOG=sherion=debug cargo run
```

## Configuration

Copy or edit [`sherion.toml`](sherion.toml) in the project root, or point to a custom file:

```bash
SHERION_CONFIG=/path/to/sherion.toml cargo run
```

## Stack

| Layer | Crate |
|-------|-------|
| Window + events | winit 0.30 |
| GPU | wgpu 29 |
| Terminal core (grid, VTE, PTY) | alacritty_terminal 0.26 |
| Text shaping | parley 0.10 |
| GPU rendering | vello 0.9 |
| Config | serde + toml |

## Architecture

PTY I/O runs on a dedicated alacritty_terminal event-loop thread. The winit UI thread renders terminal state via parley text shaping and vello scene rendering, blitted to the window surface through wgpu.

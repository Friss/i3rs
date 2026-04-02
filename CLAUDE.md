# CLAUDE.md

## Project Overview

i3rs is an open-source cross-platform alternative to MoTeC i2 Pro for motorsport telemetry analysis. It parses MoTeC `.ld` binary log files and `.ldx` XML sidecar files. The full roadmap is in `docs/PLAN.md`.

## Build & Test

```bash
cargo build --release          # Build all crates
cargo test                     # Run tests (5 unit tests in i3rs-core)
cargo run --release -p i3rs-app -- test_data/VIR_LAP.ld   # Run GUI with test file
cargo run --release -p i3rs-cli -- test_data/VIR_LAP.ld   # Run CLI inspector
```

Rust 2024 edition — requires rustc 1.85+.

## Workspace Structure

Three crates in `crates/`:

- **i3rs-core** — Pure library. Parsing, data model, lap detection, downsampling. No GUI dependencies.
- **i3rs-app** — egui desktop GUI. Depends on i3rs-core.
- **i3rs-cli** — Simple CLI that prints session info and channel stats. Depends on i3rs-core.

## Key Source Files

### i3rs-core (`crates/i3rs-core/src/`)
- `lib.rs` — Public API re-exports: `LdFile`, `Session`, `ChannelMeta`, `Event`, `DataType`, `LdxFile`, `LdxLap`, `Lap`, `detect_laps`, `downsample_minmax`, `DownsampledPoint`
- `ld_parser.rs` — Binary .ld format parser using memmap2. Key types: `LdFile` (entry point), `Session` (metadata), `ChannelMeta` (channel info + lazy data access). Methods: `read_channel_data()`, `read_channel_range()`
- `ldx_parser.rs` — XML sidecar parser for lap timing. `find_ldx_for_ld()` locates the .ldx next to a .ld file
- `lap_detect.rs` — Detects lap boundaries from "Lap Number" channel data
- `downsample.rs` — Min-max decimation for efficient chart rendering

### i3rs-app (`crates/i3rs-app/src/`)
- `app.rs` — Top-level `I3rsApp` struct, egui-dock layout, file open logic
- `state.rs` — `SharedState`: cursor position, zoom range, selected lap, channel data cache
- `workspace.rs` — Save/load workspace layouts as JSON
- `panels/graph.rs` — Main graph panel (largest file ~23KB): multi-channel time-series, overlay/tiled modes, dual Y-axes, zoom/pan
- `panels/channel_browser.rs` — Searchable channel list with drag-and-drop
- `panels/cursor_readout.rs` — Shows all plotted channel values at cursor time
- `panels/timeline.rs` — Overview bar with draggable zoom window

## Architecture Notes

- Files are opened via `memmap2` — no full file read, OS pages data on demand
- Channel sample data is decoded lazily when requested (not at file open time)
- All panels share state through `SharedState` for cursor/zoom synchronization
- GUI uses immediate-mode rendering (egui) — redraws every frame during interaction
- egui-dock provides the dockable/tabbable panel layout system

## Binary Format

The .ld format is little-endian throughout. Key constants in `ld_parser.rs`:
- Header size: `0x6E2` (1762 bytes)
- Channel metadata entry size: 120 bytes
- Magic byte: `0x40`
- Channel metadata is a linked list (each entry has `next_chan_meta_ptr`)
- Data types: u8/u16/u32/i8/i16/i32/f16/f32 (see `DataType` enum)

Full format docs: `docs/ld-file-format.md`

## Test Data

- `test_data/VIR_LAP.ld` (~4.8MB) — single lap at Virginia International Raceway
- `test_data/VIR_LAP.ldx` — accompanying lap metadata XML
- `examples/` (gitignored) — larger files up to 100MB for manual testing

## Current Status

Milestones 1–3 complete (core parsing, multi-channel graphs, workspace layout with cursor sync). Next up: Milestone 4 (math engine + reports).

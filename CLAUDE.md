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
- `math_expr.rs` — Recursive descent expression parser (AST: `Expr`, `BinOp`)
- `math_engine.rs` — Expression evaluator with built-in functions, channel resampling, `evaluate_expression()` entry point
- `export.rs` — CSV export with multi-frequency resampling
- `track.rs` — GPS track extraction, normalization, color mapping, sector timing. Key types: `TrackData`, `Sector`, `SectorTime`. Functions: `extract_gps_track()`, `find_nearest_sample()`, `compute_color_map()`, `compute_sector_times()`

### i3rs-app (`crates/i3rs-app/src/`)
- `app.rs` — Top-level `App` struct, egui-dock layout, file open logic, menu bar
- `state.rs` — `SharedState`: cursor position, zoom range, selected lap, channel data cache, `ChannelId` (Physical/Math), `MathChannelDef`
- `workspace.rs` — Save/load workspace layouts + math channels as JSON
- `panels/graph.rs` — Main graph panel: multi-channel time-series, overlay/tiled modes, dual Y-axes, zoom/pan. Uses `ChannelId` for both physical and math channels
- `panels/channel_browser.rs` — Searchable channel list with drag-and-drop, includes math channels section
- `panels/cursor_readout.rs` — Shows all plotted channel values at cursor time
- `panels/timeline.rs` — Overview bar with draggable zoom window
- `panels/math_editor.rs` — Math channel definition UI: add/edit/delete/evaluate expressions, predefined calculation templates, channel alias management
- `panels/report.rs` — Statistics report panel: min/max/avg/stddev per channel per lap
- `panels/track_map.rs` — GPS track map panel: rainbow coloring by channel value, sector editor, sector time report, cursor sync, reference lap selection

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

Milestones 1–5 complete. Next up: Milestone 6 (histograms, scatter, FFT, gauges).

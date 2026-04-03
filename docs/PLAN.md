# Plan: Open-Source MoTeC i2 Pro Alternative

## Context

MoTeC i2 Pro is the industry-standard motorsport telemetry analysis tool, but it's Windows-only, expensive, and locked to MoTeC's licensing model. We're building a cross-platform (Windows/macOS/Linux) open-source alternative that can open MoTeC .ld log files and provide the same core analysis workflow: time-series graphs, lap overlays, track maps, math channels, and statistical reports.

## Technology Stack

**Fully native Rust application: egui + wgpu**

- **egui** (via `eframe`) for the GUI — immediate-mode rendering is a natural fit for a data visualization tool where pan/zoom means redrawing every frame anyway
- **wgpu** for GPU-accelerated chart rendering — custom line/area shaders for rendering millions of downsampled data points at 60fps
- **egui-dock** for the docking/panel layout system (workbook/worksheet metaphor)
- **memmap2** for memory-mapped file access — opening a 100MB file is instant, OS pages in data on demand
- **rayon** for parallel channel parsing and math evaluation
- **half** crate for IEEE 754 float16 (already used in the Rust parser)
- **nom** or **pest** for math expression parsing (Milestone 5)

**Why this stack:**
- Single language, single binary, zero runtime dependencies
- Fastest possible data path: mmap → parse → downsample → GPU render, all in Rust
- egui-dock provides the panel docking system; rerun.io proves egui scales for complex data visualization
- Cross-platform via wgpu (Vulkan/Metal/DX12/OpenGL backends)

## Architecture

### Data Flow: File to Pixels

```
.ld file on disk
    │
    ▼
memmap2 (memory-mapped, zero-copy access)
    │
    ▼
i3rs-core: parse header (~1.7KB), walk channel metadata linked list (~24KB)
    │  Channel sample data stays on disk until requested
    ▼
On-demand channel access: channel.samples(t0..t1)
    │  Reads + decodes + scales only the requested time range
    ▼
Min-max downsampler: N samples → 2*pixel_width points
    │  Preserves visual peaks/valleys, visually lossless
    ▼
wgpu vertex buffer upload → GPU line/area shader → screen
```

### Key Performance Strategy: Min-Max Decimation

A 100Hz channel over 30 minutes = 180,000 samples. With 200+ channels, total data is 10–50M points. We never render all of them.

- Divide visible time range into N buckets (N = chart pixel width, ~1500)
- Per bucket: emit `(min, max)` from all samples in that bucket
- Render as filled area between min/max lines when zoomed out
- Switch to raw points when zoomed in enough (bucket < 2 samples)
- Result: 3,000 floats sent to GPU per channel regardless of zoom level

### Module Structure

```
crates/
├── i3rs-core/            # Pure library: parsing, data model, math, downsampling
│   ├── ld_parser.rs      # Evolved from existing parser
│   ├── ldx_parser.rs     # .ldx sidecar XML (lap times)
│   ├── channel.rs        # Channel model + lazy mmap-backed data access
│   ├── session.rs        # Session/Header/Event metadata
│   ├── downsample.rs     # Min-max decimation + LTTB
│   ├── math_engine.rs    # Expression parser + evaluator (Milestone 5)
│   ├── track.rs          # GPS track generation
│   ├── lap.rs            # Lap detection, beacon handling, sector times
│   └── export.rs         # CSV export
├── i3rs-app/             # egui desktop application
│   ├── main.rs           # eframe entry point
│   ├── app.rs            # Top-level App state + egui-dock layout
│   ├── state.rs          # Shared application state (sessions, cursor, zoom)
│   ├── panels/
│   │   ├── graph.rs      # Time-series graph panel (wgpu custom rendering)
│   │   ├── channel_browser.rs  # Searchable channel list with drag-and-drop
│   │   ├── track_map.rs  # GPS track visualization
│   │   ├── histogram.rs  # Distribution histogram
│   │   ├── scatter.rs    # XY scatter plot
│   │   ├── gauge.rs      # Configurable gauges
│   │   ├── report.rs     # Channel/section time reports
│   │   └── details.rs    # Session metadata viewer
│   └── renderer/
│       ├── line_shader.rs    # wgpu line/area rendering pipeline
│       ├── track_shader.rs   # Track map rendering
│       └── viewport.rs       # Pan/zoom transform management
└── i3rs-cli/             # Optional CLI (replaces current standalone parser)
```

---

## Milestones

### Milestone 1: Core Library + Single Graph Viewer ✅

**Goal**: Open a .ld file, browse channels, plot one channel with smooth zoom/pan.

- [x] Cargo workspace with three crates: i3rs-core, i3rs-app, i3rs-cli
- [x] Refactor parser into `i3rs-core` library crate with memory-mapped file access (memmap2)
- [x] `LdFile` → `Session` (metadata) + `ChannelMeta` (lazy data access via mmap)
- [x] On-demand channel data reading: `read_channel_data()` and `read_channel_range()`
- [x] Min-max downsampler with unit tests
- [x] eframe app scaffold with session info bar
- [x] Channel browser sidebar (searchable list of 200+ channels: name, unit, freq)
- [x] Single graph panel using egui_plot: line rendering, scroll-wheel zoom, drag pan
- [x] File open dialog + drag-and-drop .ld file support
- [x] Multi-channel plotting with color-coded lines
- [x] Fix event block parsing: venue_ptr and vehicle_ptr are uint32, not uint16 (bug in all original implementations)
- [x] Updated ld-docs.md with corrected pointer types
- [x] **Validated**: All three test files (16MB, 38MB, 100MB) parse correctly with 199–204 channels

### Milestone 2: Multi-Channel Graphs + Laps ✅

**Goal**: Plot multiple channels, navigate by lap.

- [x] Multiple channels per graph with independent Y-axes
- [x] Tiled graph mode (channels stacked vertically, linked X-axis)
- [x] Parse .ldx sidecar files for lap times and beacon data
- [x] Lap detection from Lap Number channel in .ld data
- [x] Lap markers on time axis (dashed vertical lines)
- [x] Lap selector (click to zoom to a lap, with lap times)
- [x] Channel drag-and-drop from browser to graph panel
- [x] Right-click context menus (remove channel, change color, Y-axis assignment)
- [x] Dual Y-axis support (left/right for different units)
- [x] Overlay vs Tiled graph mode toggle (View menu)
- [x] Updated to egui 0.34 / egui_plot 0.35 APIs

### Milestone 3: Workspace Layout + Cursor Sync

**Goal**: Multiple synchronized panels in a configurable docked layout.

- [x] egui-dock: multiple graph panels, resizable, dockable, tabbable
- [x] Worksheet tabs (multiple layouts)
- [x] Cursor synchronization: vertical line follows mouse across all panels
- [x] Zoom synchronization: all time-series panels share the same time window
- [x] Cursor readout panel (all plotted channel values at cursor time)
- [x] Timeline overview panel (draggable zoom window over full session)
- [x] Save/load workspace layout to JSON

### Milestone 4: Math Engine + Reports

**Goal**: Computed channels and statistical analysis.

- [x] Math expression parser (hand-written recursive descent) and evaluator
  - Syntax: `(WheelSpeed_RL - GPS.Speed) / GPS.Speed * 100`
  - Built-in functions: `smooth()`, `derivative()`, `integrate()`, `abs()`, `atan2()`, trig, `pow()`, `clamp()`, etc.
  - Evaluation with caching, channel name resolution (underscore↔space/dot, case-insensitive)
  - Automatic frequency resampling when mixing channels of different sample rates
- [ ] Predefined calculations: oversteer angle, pitch/roll, wheel slip
- [x] Math file save/load/import/export (JSON format)
- [ ] Automatic unit conversion (km/h ↔ mph, C ↔ F, kPa ↔ psi)
- [x] Channel report panel: min/max/avg/stddev per channel per lap
- [ ] Channel aliases (map different names across sessions to a common name)
- [ ] Data gating (exclude regions by boolean condition)
- [x] CSV export of selected channels and time range
- [x] Math channel editor panel (define, edit, delete math channels with inline error display)
- [x] Math channels integrated into channel browser and graph panels
- [x] ChannelId abstraction (Physical/Math) throughout the UI

### Milestone 5: Track Map

**Goal**: GPS track visualization.

- [ ] Track map panel from GPS lat/lon channels
- [ ] Rainbow track map coloring by any channel value (speed, throttle, brake, etc.)
- [ ] Track section editor (define sectors by clicking on the map)
- [ ] Section time report (split times, gained/lost per sector between overlays)
- [ ] Reference lap selection

### Milestone 6: Histograms, Scatter, FFT, Gauges

**Goal**: Advanced analysis components.

- [ ] Histogram panel (distribution of values, per-lap breakdown)
- [ ] Scatter/XY plot (channel vs channel, e.g., throttle vs RPM)
- [ ] FFT panel (frequency analysis for vibration diagnosis)
- [ ] Mixture map (2D heatmap: mixture vs RPM vs load)
- [ ] Gauges panel (analog/digital/bar gauges showing value at cursor time)
- [ ] Steering wheel angle widget/gauge
- [ ] All components synchronized with cursor/zoom

### Milestone 7: Overlays

**Goal**: Compare laps across sessions.

- [ ] Overlay system: load multiple laps (same session or different .ld files) on the same graph
- [ ] Overlay time alignment (graphical offset adjustment)
- [ ] Lap stretching (warp time axis of one lap to align with another)
- [ ] Graph X axis by distance driven vs time

### Milestone 8: Polish + Packaging

**Goal**: Production-quality release.

- [ ] Application profiles (Circuit, Drag, Bike, Rally) with preconfigured layouts
- [ ] Project system (save all sessions, math, layouts for a race weekend)
- [ ] Global channel preferences (colors, scales, units)
- [ ] Vehicle setup sheets
- [ ] Session details editor + side-by-side comparison
- [ ] Dark/light/high-contrast themes
- [ ] Keyboard shortcuts for all operations
- [ ] Performance optimization pass
- [ ] Cross-platform packaging: Windows MSI, macOS DMG, Linux AppImage
- [ ] User documentation

### Milestone 9: Video Sync + Animation

**Goal**: Link in-car camera footage with data.

- [ ] Embedded video player panel (platform video decoding)
- [ ] Manual video-to-data time synchronization
- [ ] Playback animation: cursor moves at real-time or adjustable speed
- [ ] All components animate during playback
- [ ] Video overlay generation (export video with gauges burned in)
- [ ] Export worksheet to PNG

---

## Verification Plan

After each milestone, validate against the real test data:

1. **Milestone 1** ✅: Open `/examples/S1_#28299_20251122_174510_2.ld` (100MB). Channel list shows 204 channels. Plotting works with zoom/pan.
2. **Milestone 2**: Plot Engine.Speed + Throttle.Position + Brake.Pressure overlapped. Lap markers should appear. Click a lap to zoom.
3. **Milestone 3**: 4-panel layout (engine, chassis, GPS, driver inputs). Move cursor in one panel — all panels track.
4. **Milestone 4**: Open two different session files. Overlay fastest laps. Track map colored by speed. Sector times should show where time is gained/lost.
5. **Milestone 5**: Create math channel `WheelSlip = (WheelSpeed.RL - GPS.Speed) / GPS.Speed * 100`. Verify values match manual calculation. Export to CSV and validate.
6. **Milestone 6**: Histogram of Engine.Speed for one lap. Scatter plot of Throttle vs RPM. FFT of a vibration channel.

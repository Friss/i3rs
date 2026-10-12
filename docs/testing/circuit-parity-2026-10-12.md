# Circuit workspace comparison — 12 October 2026

Follow-up to [the parser comparison and five fixes](i2-comparison-2026-10-12.md).
Same log: `S1_#28299_20260508_174455_02.ld`, with i2 Standard 1.1.4.457 in VMware
as the reference. This pass uses the working tree after those fixes.

**Current result:** the follow-up now includes the 33 named VIR distance sections,
corrected front-wheel speed/distance, full-rate oil scatter, lambda-delay alignment,
and centered load bands. The latest evidence and limits are below; earlier passes
are retained as a dated record and their old totals are superseded here.

## GUI dependency upgrade — completed 12 October 2026

Upgraded the compatible GUI set together: **egui/eframe 0.36.2**, **egui_plot
0.37.0**, and **egui_dock 0.21.1**. The lockfile also updates their rendering/font
dependencies, including wgpu 30.0.1 and the matching wasm-bindgen family. Other
direct dependency requirements are unchanged. The application declares Rust
1.95 as its minimum; README and AGENTS build instructions match it.

The migration adapts native file drops to egui's `DroppedFile::path()` API.
Browser filenames are not treated as filesystem paths; existing web byte/picker
loading remains in place. Headless graph tests and the CPU benchmark explicitly
clear texture updates because they have no GPU texture manager, as required by
egui's new checks. Production texture handling remains eframe's responsibility.

### Regression results

- **178 standard tests pass**, including graph spikes/gaps, native/adaptive cache
  switching, workspace serialization, and backward-compatible workspace fields.
- **Six local i2 reference tests pass** for values/laps, corrected distance,
  section timing, mixture counts, and oil scatter.
- Clippy (`--workspace --all-targets -- -D warnings`), formatting, and diff checks
  pass.
- Native release build, WebAssembly target check, and **Trunk release web bundle**
  build pass. Trunk installs its matching wasm-bindgen 0.2.129 tooling.
- A separate native test instance launches and renders its initial dock layout.
  The existing working app was left open. Automated native menu interactions
  did not produce an observable response, so loading/interacting with a log in
  that new native instance is not counted as verified here. The graph/persistence
  regression tests and benchmark did execute successfully.

### Paired rendering measurements

Six release benchmark runs per version, alternating old/new order on this Mac.
Each run draws 30 frames, discards the first five, and reports the remaining 25.
Below are medians of those six per-run medians. The workload is unchanged: six
100 Hz channels, 1,200 × 800 plot, CPU plot construction plus tessellation with
cached points. GPU presentation and event-loop idle CPU are outside this test.

| View | Points (both versions) | Before | After | Change |
|---|---:|---:|---:|---:|
| 134.114 s lap-length view, normal adaptive policy | 80,472 | 1.461 ms | 1.454 ms | −0.5% |
| Whole outing, adaptive overview | 27,896 | 0.655 ms | 0.653 ms | −0.3% |
| Whole outing, every sample | 2,350,926 | 48.800 ms | 48.050 ms | −1.5% |

There is **no material rendering regression or established rendering speedup**.
Overview preparation is also stable: 9.345→9.308 ms. The small differences are
within the observed variation, and the previous overview optimization remains
necessary. These results do not measure the upstream Wayland busy-loop fix or
claim a whole-application frame-rate improvement.

All five Criterion core benchmarks were compared with a fresh saved baseline
(20 samples, 1 s warmup, 2 s measurement per case). Channel decoding showed no
change; downsampling, expression evaluation, and FFT changes were within
Criterion's noise threshold. Nearest-track lookup measured faster, but uses the
same unchanged core benchmark binary, so that difference is not attributed to
the GUI update. No core regression was reported.

Evidence: [paired raw measurements and summary](../../target/i2-comparison/gui-paired-benchmarks.json),
[core baseline](../../target/i2-comparison/gui-upgrade-before-core-bench.txt),
[core comparison](../../target/i2-comparison/gui-upgrade-after-core-bench.txt),
[standard tests](../../target/i2-comparison/gui-upgrade-tests.txt),
[i2 references](../../target/i2-comparison/gui-upgrade-reference-tests.txt),
[Clippy](../../target/i2-comparison/gui-upgrade-clippy.txt),
[native build](../../target/i2-comparison/gui-upgrade-build.txt),
[WASM check](../../target/i2-comparison/gui-upgrade-wasm-check.txt),
[web bundle](../../target/i2-comparison/gui-upgrade-web-build.txt).
Local builds used `DEVELOPER_DIR=/Library/Developer/CommandLineTools` because
the separately installed Xcode had a pending license prompt; no license was
accepted or system developer-directory setting changed.

## Earlier dependency audit — 12 October 2026

The following records the pre-upgrade audit; its locked versions and statement
that dependencies were unchanged describe that earlier point in the work.

Checked actual `Cargo.lock` versions against the live crates.io registry and
reviewed published source/changelogs. Dependency manifests and the lockfile were
not changed by this audit; no upgrade performance measurements are claimed.

| Dependency | Locked | Latest stable | Relevance |
|---|---|---|---|
| eframe / egui | 0.35.0 | 0.36.2 | Best candidate: native redraw scheduling and hidden-window fixes |
| egui_plot | 0.36.0 | 0.37.0 | Required companion upgrade; boxed-zoom visibility fix, no new dense-line reduction |
| egui_dock | 0.20.1 | 0.21.1 | Required companion upgrade; use .1 for old serialized DockState compatibility |
| wgpu (transitive) | 29.0.4 | 30.0.1 | Comes through the GUI upgrade; GPU-side benefit needs separate measurement |
| winit (transitive) | 0.30.13 | 0.30.13 | Already current |
| half / memmap2 / rustfft | 2.7.1 / 0.9.11 / 6.4.1 | Same | No newer parser/FFT release to evaluate |

The strongest evidence is eframe's
[redraw busy-loop fix](https://github.com/emilk/egui/pull/8398), released in
[0.36.2](https://github.com/emilk/egui/blob/0.36.2/crates/eframe/CHANGELOG.md).
Inspection of the published 0.35.0 source confirms it contains the affected
`ControlFlow::Poll` path, so this is relevant to our locked version. Upstream's
measurements were on Linux/Wayland; their improvement is not a measured macOS
i3rs speedup. Version 0.36 also avoids running an egui pass for hidden windows
or browser tabs while retaining application logic, improving state preservation
and avoiding unnecessary work
([implementation](https://github.com/emilk/egui/pull/8387)).

The [plot release](https://github.com/emilk/egui_plot/releases/tag/0.37.0)
and the published 0.36.0→0.37.0 source diff contain no replacement for our dense
overview reduction. Source changes in series code include iteration cleanup for
filled areas and hover searches. The
[epaint update](https://github.com/emilk/egui/blob/0.36.2/crates/epaint/CHANGELOG.md)
includes a color-conversion optimization and thin rotated-rectangle correctness
fix; its tessellator source does not add a major dense-polyline optimization.
An upgrade should therefore target scheduling/robustness first, with the current
native/adaptive plot benchmark rerun as a regression check.

Upgrade these four direct GUI packages together. Their new minimum Rust version
is 1.95; the installed compiler is 1.97.1. `TabViewer::id` is now required and
i3rs already implements it. Use
[egui_dock 0.21.1](https://github.com/anhosh/egui_dock/blob/main/CHANGELOG.md),
since 0.21.0 broke deserialization of older saved layouts. Validate native and
WebAssembly builds, saved workspaces, graph interactions, and idle CPU afterward.
The README's existing Rust 1.85 claim is already below egui 0.35's 1.92 minimum
and should be corrected when recording the upgraded toolchain requirement.

Other available direct updates: crossbeam-channel 0.5.16→0.5.17, quick-xml
0.41.0→0.42.0, serde_json 1.0.151→1.0.154, clap 4.6.4→4.6.7, log
0.4.33→0.4.34, wasm-bindgen 0.2.126→0.2.129 with js-sys/web-sys
0.3.103→0.3.106 and wasm-bindgen-futures 0.4.76→0.4.79. These are separate
maintenance candidates, not established fixes for the measured graph cost.
Crossbeam's new release includes bounded-channel correctness fixes; JSON's
patches concern correctness/features rather than this rendering path.

Local evidence: [registry snapshot](../../target/i2-comparison/dependency-audit.json),
[published rendering source comparison](../../target/i2-comparison/dependency-render-source.diff),
[old event loop](../../target/i2-comparison/eframe-0.35.0-event-loop.rs),
[new event loop](../../target/i2-comparison/eframe-0.36.2-event-loop.rs).

## Latest sampling follow-up: dense overview performance

MoTeC's [i2 API User Guide](https://www.motec.com.au/hessian/uploads/i2_API_User_Guide_330ed28c83.pdf)
documents per-channel linear interpolation or previous-sample reads and
resampling to higher/lower rates, including average/minimum/maximum anti-alias
options. **i2 supports resampling**, but this API does not establish whether or
how its graph renderer reduces samples to screen pixels. That remains unknown.

The native renderer below established that lap-sized views are inexpensive but
whole-outing views can cost tens of milliseconds. The current graph renderer
therefore uses a display-only reduction for traces with **more than 100,000
visible points and more than eight samples per physical pixel column**. It
retains each column's first, minimum, maximum, and last actual samples in their
original order. Extrema keep their exact timestamps and values; invalid runs
remain gaps. It never averages extrema or moves them to synthetic timestamps.
Interior subpixel oscillations may be omitted, so this is not a guarantee of
pixel-identical rendering to every original line segment.

Ordinary lap views retain every sample. Decoded data, cursor reads, statistics,
and analysis retain their previous numerical behavior. This display policy
applies to primary, math, lap, and external-session graph traces. Prepared points
are cached; viewport width, display scale, sample rate, and the resolution
setting participate in invalidation. Right-click a graph and enable **Draw every
sample** to bypass reduction at any zoom. The setting is saved per graph in the
workspace, with a compatible default for older files.

### Performance

Release benchmark: six 100 Hz cylinder-knock channels from this recording,
1,200 × 800 logical-point plot, 30 measured frames after five warmups. These are
**CPU plot construction plus tessellation** measurements with cached point
arrays, excluding GPU rendering, decoding, and the rest of the application.

| Interval | Rendering | Points | CPU median | CPU p95 | Point preparation |
|---|---|---:|---:|---:|---:|
| First 134.114 s | Every sample | 80,472 | 1.678 ms | 1.870 ms | 0.145 ms |
| First 134.114 s | Adaptive (same native points) | 80,472 | 1.403 ms | 1.485 ms | 0.095 ms |
| Whole 3,918.210 s outing | Every sample | 2,350,926 | 52.963 ms | 96.392 ms | 3.093 ms |
| Whole outing | Adaptive overview | 27,896 | 0.632 ms | 0.665 ms | 11.151 ms |

The whole-outing cached drawing median improved about **84×**. The approximately
11 ms preparation cost is paid when those cached traces must be rebuilt, such
as after changing the viewport. Lap-sized paths draw identical point arrays;
their timing differences are measurement variation. These figures are not
end-to-end frame-rate or browser-performance claims.

### Validation

All **178 standard tests** pass. New coverage checks exact extrema timestamps,
time ordering, bucket endpoints, gaps and isolated events, unchanged native lap
points, both primary/lap rendering caches when toggling resolution, and workspace
persistence including older files. Clippy with warnings denied, the release
build, and the WebAssembly target check pass. The rebuilt native application was
opened on the Driver worksheet with the full outing for visual verification.

Local evidence: [benchmark](../../target/i2-comparison/sampling-adaptive-benchmark.txt),
[tests](../../target/i2-comparison/sampling-adaptive-all-tests.txt),
[Clippy](../../target/i2-comparison/sampling-adaptive-clippy.txt),
[WebAssembly check](../../target/i2-comparison/sampling-adaptive-wasm.txt),
[native graph capture](../../target/i2-comparison/i3rs-adaptive-sampling.jpg).
Reproduce with
`cargo test --release -p i3rs-app --lib native_sampling_render_benchmark -- --ignored --nocapture --test-threads=1`.

## Earlier sampling follow-up (overview policy superseded above)

This section records the initial native-only implementation and its measurements.
The dense-overview rendering policy above supersedes its no-reduction statements.

The time-series renderer was doing **lossy downsampling**, rather than
supersampling. Dense analog channels went through pixel-width min/max buckets
(or a cached level-of-detail pyramid), but the renderer then plotted
`(min + max) / 2` at each bucket's midpoint time. That can halve an isolated
positive spike over a zero baseline and move its apparent time. Discrete channels
already bypassed that path. The decoded arrays themselves were not reduced.

Graphs now use every recorded sample in the visible interval, at its original
sample time. This applies to physical and math channels, tiled/overlapped graphs,
lap comparisons, and external-session overlays. Distance mode maps those same
sample times onto the distance axis. Binary-search culling retains adjacent
samples for correct edge clipping and sub-sample zooms, including stationary
plateaus on a distance axis. The plot cache remains; the unused LOD pyramid,
background reduction jobs, and reduction caches were removed. Display unit
transforms and invalid-data gaps are preserved.

Gauges also now use the cursor readout's sample-hold policy for enums and known
state channels. Previously a gauge could interpolate between two states and
round into the next state before the actual transition.

### Sampling policies after this change

| Path | Current policy |
|---|---|
| Time/distance graphs and lap overlays | Original samples; connecting lines; no pixel-bucket reduction |
| Cursor/legend/gauges | Linear interpolation for analog values; previous sample for discrete states |
| Histogram/report | Native valid samples in the selected range; histogram color channel is interpolated at each base sample time |
| Scatter | Faster axis rate; slower axis interpolated; no point stride |
| Mixture map | Slowest of X/Y/color rates, with configured sensor delay/filter; fixture channels are all 20 Hz |
| Math | Highest referenced channel rate; slower inputs linearly interpolated |
| CSV export | Highest selected rate; slower inputs use nearest neighbor |
| Track colors | Color channel interpolated onto GPS sample times |
| FFT | Selected native sample slice; spectral processing is separate from graph rendering |

Interpolation aligns channels or evaluates a cursor between timestamps; removing
it would change analysis semantics rather than simply improve graph fidelity.
Mixed-rate mixture maps can still discard faster-channel detail. CSV, math and
mixed-rate state-channel interpolation are not a single consistent policy, and
have not been established as matching i2. These are documented comparison gaps;
this pass does not silently change their numerical contracts.

### What is established about i2

MoTeC's [i2 1.1.4 feature guide, page 17](https://www.motec.com.au/hessian/uploads/i2_V1_1_4_Feature_Guide_0d03b00fa8.pdf)
shows independent sample/pixel cursor snapping and point/line trace controls.
This establishes that cursor placement and trace presentation are separate
choices; it does **not** establish i2's internal dense-trace reduction algorithm.
i3rs currently has interpolated cursor readouts but no equivalent sample-snap
selector. No claim of bit-for-bit renderer parity is made.

The earlier controlled numerical comparisons remain relevant: Lap 3 oil scatter
matches at 6,706 samples; the equal-rate mixture totals/bands match when i2's gear
filter is disabled. The six local i2 regressions were rerun successfully after
this change. The one-sample oil out-lap discrepancy and gear-filter difference
listed below remain unresolved.

### Measured performance and validation

Release benchmark on the current Mac: six 100 Hz cylinder-knock channels from
this recording, 1,200 × 800 logical-point plot, 30 frames with five warmups.
The table measures **CPU plot construction plus tessellation**, using cached
point arrays; it excludes GPU rendering, file decode, and the rest of the app.
The reduced comparison uses 1,200 buckets per channel, not the old physical
channel's variable LOD selection.

| Interval | Rendering | Points | CPU median | CPU p95 |
|---|---|---:|---:|---:|
| First 134.114 s (lap-length window) | Bucket midpoint comparison | 7,200 | 0.216 ms | 0.220 ms |
| First 134.114 s | Native | 80,472 | 1.501 ms | 1.620 ms |
| Whole 3,918.210 s outing | Bucket midpoint comparison | 7,260 | 0.194 ms | 0.210 ms |
| Whole outing | Native | 2,350,926 | 52.584 ms | 73.177 ms |

Native point preparation took 0.097 ms / 3.230 ms respectively. These results
support full-sample lap analysis, but **do not support a blanket 60 fps claim for
multi-million-point full-outing views**. No lossy fallback is applied. Browser/GPU
performance remains unmeasured. A future full-outing optimization should preserve
sample extrema and time ordering rather than restore averaged buckets.

Three new regressions cover dense spikes/timestamps/gaps, lap transforms, and
viewport edge clipping on distance axes. All **173 standard tests**, **six local
i2 reference tests**, and the release benchmark pass. Clippy with warnings denied, the release build, and the WebAssembly target check
pass. The rebuilt Driver worksheet was also opened with the full outing and its
traces/readouts checked visually.

Local evidence: [benchmark output](../../target/i2-comparison/sampling-benchmark.txt),
[test output](../../target/i2-comparison/sampling-all-tests.txt),
[i2 regressions](../../target/i2-comparison/sampling-i2-regressions.txt),
[native graph capture](../../target/i2-comparison/i3rs-native-sampling.jpg).
Reproduce the benchmark with
`cargo test --release -p i3rs-app --lib native_sampling_render_benchmark -- --ignored --nocapture --test-threads=1`.

## Follow-up: VIR definitions and numerical reconciliation

### Delivered corrections

- Added [VIR Full definition](../../crates/i3rs-core/data/vir-full-sections.json),
  transcribed from all 33 i2 Track Editor rows. The model length is **5,337.5 m**;
  boundaries shown by the editor are whole metres. The front straight wraps from
  4,785 m to 403 m. Turns 14 and 15 are adjacent, with no intervening straight.
- Track Report and Section Times have a **VIR Full sections** action. Select a
  complete lap first. Timing retains distance boundaries independently of GPS
  marker indices, interpolates crossings, and stretches only laps within 5% of
  the model length, as configured in the reference. Most timed laps in this
  recording lie outside that tolerance. JSON/workspace persistence retains these
  distance boundaries; incomplete passages remain missing.
- i2's corrected-speed setup takes the faster front wheel, with longitudinal-G
  correction disabled. Shared core logic now uses that source for distance and
  track speed annotations. Explicit Corr Speed takes precedence, with available
  wheel/vehicle/GPS fallback. Primary and external-overlay results agree. Raw
  distance channels are converted to metres before use; unsupported units are
  skipped. Higher-priority asynchronous decodes are awaited consistently.
- Oil scatter uses the faster channel's sample rate and counts every valid point.
  It no longer loses pressure samples through slower-channel sampling or stride
  decimation. Zero frequency and out-of-range intervals are handled safely.
- Mixture maps align lambda at `t + sensor delay` with RPM/load at `t`, keeping
  both times inside the chosen range. Circuit defaults use **0.150 s** and exclude
  lambda below **0.500**. The visible 0.7–1.2 scale no longer silently discards
  samples from totals or color-band statistics. Out-of-scale points are clipped
  from drawing, not from statistics. Delay/filter settings persist in workspaces.
- Added centered color bands, using the reference's **11 centers, 0–100%**.
  Boundary tests cover the 5%/95% transitions and noncentered legacy settings.
- Dense map annotations use spaced columns with leader lines, after the 33-section
  native check exposed overlapping labels.

### Measured comparisons

| Check | i2 | i3rs | Result |
|---|---:|---:|---|
| Whole-outing corrected distance | 89,967 m (rounded) | 89,966.948 m | Matches displayed precision |
| Lap 3: 31 named sections from Turn 1 through Str 16-17 | Captured individually | Largest absolute difference 35.525 ms | Within 50 ms regression tolerance; not millisecond parity |
| Lap 3: all 33 native sections summed | Lap duration 134.114 s | 134.114 s | Full coverage, no gaps or overlap |
| Out-lap mixture bank 1, gear filter disabled | 6,821 | 6,821 | Exact |
| Out-lap mixture bank 2, gear filter disabled | 6,833 | 6,833 | Exact |
| Oil scatter, Lap 3 | 6,706 | 6,706 | Exact |
| Oil scatter, Out Lap | 17,577 | 17,576 | One sample remains unresolved |

All 11 mixture load bands also match with the same settings. Ascending centers:
bank 1 `[6794, 3, 3, 2, 1, 1, 0, 1, 4, 6, 6]`; bank 2
`[6806, 3, 3, 2, 1, 1, 0, 1, 4, 6, 6]`.
These totals and the 33-section preset were verified in the rebuilt native UI.

### Remaining differences and reference limits

- **Gear-change filtering:** i2 enables an RPM-rate filter (±2,500 rpm/s,
  0.500 s exclusion period, 0.250 s derivative filter time). It removes 20 samples
  from each out-lap mixture bank, yielding 6,801 / 6,813. Its precise derivative
  and exclusion semantics are not reproduced. The native panel explicitly says
  that gear-change filtering is not applied. The controlled comparison temporarily
  disabled that filter in i2, then restored it and verified the original counts.
- **Turn 17 reference anomaly:** i2 displays **−2.332 s** on Lap 3 and negative
  times on several subsequent laps, despite forward editor boundaries
  4,604–4,785 m. Its front straight is also split into start/end rows. Those two
  sections are excluded from the 31-section parity claim. Native sections retain
  positive forward/wrapped interval semantics. The cause of i2's anomaly is not
  established; no negative-time workaround was copied into i3rs.
- The whole-metre editor boundaries, lap-distance differences and interpolation
  leave small section-time differences. Exact sub-metre definition export and
  per-lap i2 correction behavior remain necessary for tighter agreement.
- Proprietary i2 track-map import, GPS-free reconstruction, and exhaustive
  cross-lap histogram/bin parity remain outside the verified scope.

### Follow-up validation and evidence

`cargo test --workspace`: **170 passed**, 9 opt-in/local-fixture tests ignored.
Six explicit local-reference regressions pass (three app, three core), including
all 31 section references, total lap coverage, and distance-definition JSON
round trip. `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo build --release` pass.

Local evidence (ignored build directory):
[corrected speed settings](../../target/i2-comparison/i2-distance-settings.jpg),
[stretch report](../../target/i2-comparison/i2-stretch-report.jpg),
[section timing final rows](../../target/i2-comparison/i2-section-numbers-final.jpg),
[mixture filter settings](../../target/i2-comparison/i2-mixture-settings.jpg),
[load-band settings](../../target/i2-comparison/i2-mixture-load-bands.jpg),
[controlled i2 counts](../../target/i2-comparison/i2-mixture-no-gear-filter.jpg),
[native mixture](../../target/i2-comparison/i3rs-mixture-delay-bands.jpg),
[native section matrix](../../target/i2-comparison/i3rs-vir-section-matrix.jpg),
[readable VIR map callouts](../../target/i2-comparison/i3rs-vir-callouts.jpg),
[reference regression output](../../target/i2-comparison/final-reference-tests.txt).
All 33 editor captures are `target/i2-comparison/i2-vir-section-00.jpg` through `32.jpg`.

## Earlier implementation pass (superseded measurements)

## Implementation and validation

### Correctness repairs

- Distance integration holds its accumulated value across missing speed intervals
  and resumes when both ends of an interval are valid. It does not invent travel
  during an outage. Primary and external-overlay paths produce the same finite,
  monotonic series. This recording now ends at **89,993.732 m**, compared with the
  i2 exported distance of **89,967 m** (26.732 m / about 0.030% difference). The
  uncorrected-speed integration is not claimed to reproduce i2 corrected distance.
- Reports own their channels and statistics cache. Hiding the source graph no
  longer empties the report. They support channel selection/removal, display
  preferences, math channels, units, missing-value markers, and workspace save/load.
- GPS normalization excludes paired zero fixes and invalid coordinates, preserves
  their timestamps as gaps, and computes the origin from valid paired fixes only.
  Map picking is constrained to the current lap/zoom range.
- Selecting a lap in the browser or section table updates the shared time range;
  a cursor outside that lap moves to its start. Sample-based analysis uses the
  half-open interval `[start, end)` consistently.
- Section crossings interpolate between adjacent GPS samples. A boundary that is
  never approached within 30 m produces a missing result. This is nearest-path
  interpolation, not a claim of exact i2 section-boundary parity.
- Gauge value labels and scale limits use separate positions.

### Added Circuit workflows

| Workflow | Delivered behavior |
|---|---|
| Mixture Map | Individual samples colored by configurable throttle bands, per-band and total counts, click-to-cursor; original heatmap remains selectable. All valid samples contribute to counts. |
| RPM Histo | Percentage or count mode, optional second color channel, configurable color bands, stacked bars, per-band counts, common bin edges for lap comparisons, explicit underflow/overflow/missing-color buckets. |
| Oil Pressure | Sample count and linked oil-pressure overview graph. |
| Channel Report | Default independent report seeded with RPM, speed, throttle, pressures, temperatures, and both lambda banks. |
| Track Report | Default GPS worksheet, selected-lap trace/picking, editable section names, per-section min/max speed and gear annotations. |
| Section Times | Separate default worksheet with section × lap matrix, best-cell highlighting, reference deltas, missing results, and selectable section trend. |
| Section setup | Split a complete lap into generic equal-distance sections, edit/name boundaries, import/export versioned geographic JSON definitions. Workspaces save geographic boundaries so another recording can resolve its own sample indices; unrelated tracks are rejected. Legacy index-only boundaries are reused only on the original recording. |
| Playback | Play/pause and 0.1×–16× speed over the current lap/zoom; shared cursor drives graphs, readout and map. Stops at range end; stationary hover does not override the playback clock. |

### Automated validation

- `cargo test`: **165 passed**, 7 intentionally ignored (local fixtures or other
  opt-in tests). A temporary-directory collision between parallel CLI installer
  tests was repaired with a process-local atomic suffix.
- Four explicit local-reference regressions passed: original beacons/validity/
  precision/value checks, GPS bounds/RPM count, both distance-generation paths,
  and mixture count.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo build --release`: passed.
- Additional regression coverage exercises missing-speed recovery, GPS gap and
  valid-zero-coordinate handling, selected-range map picking, fractional section
  crossings, geographic section transfer between recordings and wrong-track
  rejection, report/workspace round trips and legacy workspace defaults,
  histogram percentages/band boundaries, and playback range/hover behavior.

### Live native validation on the same recording

The rebuilt app was exercised through its native UI. The background window
initially did not refresh; activating it through Finder restored normal updates.
A process sample showed the main thread waiting for events, not blocked in
parsing or application locks.

- Out-lap RPM histogram: **7,031** samples, matching i2. Percentage mode and
  throttle-band counts are visible. [Capture](../../target/i2-comparison/circuit-fixed-rpm.jpg).
- Out-lap mixture: bank 1 **6,801** samples, matching i2; bank 2 **6,809** shown but
  not independently reconciled to i2. [Capture](../../target/i2-comparison/circuit-fixed-mixture.jpg).
- Channel Report remains populated with its graph hidden.
  [Capture](../../target/i2-comparison/circuit-fixed-report.jpg).
- Lap 3 Track Report has useful bounds, section speed/gear annotations and a
  moving cursor. Playback was observed advancing from 814.954 s through 832.587 s,
  then pausing at 837.671 s. [Capture](../../target/i2-comparison/circuit-fixed-track-sections.jpg).
- Four sections generated from Lap 3 total **134.114 s**; the matrix shows
  interpolated times on other laps, best cells, missing incomplete results and
  a trend plot. [Capture](../../target/i2-comparison/circuit-fixed-sections.jpg).
- Distance mode shows a positive distance axis and a complete RPM trace; the
  histogram retains its 2,682 samples in the selected Lap 3 range.
  [Capture](../../target/i2-comparison/circuit-fixed-distance.jpg).
- Oil Pressure shows the XY sample count and linked overview.
  [Capture](../../target/i2-comparison/circuit-fixed-oil.jpg).

### Remaining parity limits

The generated sections have generic names and equal-distance boundaries. i2's
specific VIR corner/straight definition has not been imported or reproduced;
matching named-section numerical comparisons require that definition. JSON
import/export is supported; proprietary i2 track-map import and GPS-free track
reconstruction are not implemented. Full per-band, bank-2 and oil-scatter numeric
reconciliation remains open. The exact i2 corrected-distance algorithm is also
not reproduced. These limits do not invalidate the confirmed sample totals or
fixed consumer bugs above.

## Original audit (before the fixes below)

## New correctness finding: distance mode collapses to zero (P1)

The graph's `integrate_speed_series()` adds missing speed samples directly to its
running sum. The first missing sample makes that sum NaN permanently. The later
`max(previous_distance)` call then keeps returning the previous distance, zero.

On this file, both Vehicle Speed and GPS Speed begin with three missing samples.
The recorded Lap Distance channel fails the monotonic-distance check, so the
graph falls back to integrating speed. The same fallback is used when loading an
external overlay.

| Probe of current graph function | Result |
|---|---:|
| Vehicle Speed: finite input samples | 77,560 |
| Vehicle Speed: maximum/final integrated distance | 0 / 0 m |
| GPS Speed: finite input samples | 77,560 |
| GPS Speed: maximum/final integrated distance | 0 / 0 m |
| i2 entire-outing export end distance | 89,967 m |

The i2 distance is evidence that this outing is nonzero, not an assertion that
integrating either uncorrected speed channel must reproduce its exact distance.
The newly honored validity intervals expose an unhandled consumer of missing
data. The earlier five fixes therefore do not establish correctness of distance
mode.

Source: [graph integration and source selection](../../crates/i3rs-app/src/panels/graph.rs).
Evidence: [probe output](../../target/i2-comparison/distance-probe-result.txt),
[standalone probe](../../target/i2-comparison/distance_probe.rs),
[i2 export header](../../target/i2-comparison/i2-export-header.png).
The probe copied the current integration/unit-conversion functions verbatim and
linked the release core library to read this file. It did not substitute a new
integration algorithm. No production fix was made in this audit.

The repair should explicitly handle missing intervals, resume integration when
valid samples return, and define the meaning of distance through an outage.
Regression coverage needs missing data at startup and mid-session, plus both
primary-session and external-overlay distance generation.

Live confirmation: switching the RPM worksheet's graph to **Distance** makes
its trace disappear around zero distance. The histogram above also becomes
empty as the shared range changes. [Screenshot](../../target/i2-comparison/i3rs-distance-zero.png).

## Further reproduced correctness failures

### Report loses channels when its source panel is hidden (P2)

From Oil Pressure with two channels visible, choose **View → Add Report Panel**.
The report becomes the active tab in the same dock leaf and immediately shows
“Plot some channels to see statistics”; the readout also becomes empty.
Report reads `display_channel_registry`, which is rebuilt by visible plotting
panels every frame. It needs a durable, independently configured channel list.
[Live reproduction](../../target/i2-comparison/i3rs-report-alone.png).

### Zero GPS coordinates make VIR disappear into the map bounds (P1)

Adding Track Map and docking it produces a long diagonal, with the real circuit
compressed into an endpoint. This log has five finite zero latitude values and
five finite zero longitude values. `extract_gps_track()` accepts these along
with the VIR coordinates; the resulting normalized X range is approximately
−0.00818 to 63.61682 and Y range −36.56191 to 0.00705. The finite-value check alone
cannot distinguish a lost GPS fix from a useful track coordinate. Add fix-quality
or physically plausible continuity handling, preserving gaps and timestamps;
do not globally reject latitude or longitude zero individually.

[Native map](../../target/i2-comparison/i3rs-circuit-track-map.png),
[probe output](../../target/i2-comparison/track-probe-result.txt),
[probe source](../../target/i2-comparison/track_probe.rs),
[GPS extraction](../../crates/i3rs-core/src/track.rs).
The probe calls the release library's actual `extract_gps_track()`.

## Feature parity matrix

| Circuit workflow observed in i2 | i3rs implementation | Assessment |
|---|---|---|
| Driver, Braking, Engine, Fuel/Ign tiled traces and gauges | Corresponding templates, linked cursor, channel preferences, embedded gauges | Broad foundation present. Native layout is less compact; gauge labels/limits can overlap, and lower traces require scrolling. This is distinct from decoded-value correctness. |
| Mixture Map: two lambda-vs-RPM scatter views, each point colored by throttle; total and per-throttle-band sample counts | Two 32×32 heatmaps with the same three channel roles; cell color is mean throttle and opacity depends on occupancy | Partial. Individual outliers and multiple throttle populations within a bin are merged; no equivalent throttle-band counts. Add a colored scatter mode alongside the heatmap. |
| Oil Pressure: lateral acceleration vs oil pressure scatter, cursor point, sample count, linked overview | Matching XY scatter default and click-to-cursor support | Core analysis exists. Sample count and the worksheet-specific overview are missing from this default. Full numerical scatter comparison remains pending. |
| RPM Histo: percentage distribution, colors and sample counts by throttle band, linked overview | RPM histogram plus graph, configurable bins, per-lap breakdown | Partial. Bars are raw counts, with no secondary throttle channel. Per-lap colors are not equivalent to throttle bands. Bin edges are recomputed separately from each data slice, limiting direct lap comparison. |
| Track Report: VIR map with named turns/straights, min/max corrected speed and gear annotations | GPS map, optional channel coloring, cursor, manual sector points | Partial. No equivalent named-track report, per-section speed/gear annotations, or automatic VIR section setup. |
| Section Times: named-section × lap matrix, best-cell highlighting, and trends across laps | Manually defined sectors and a compact timing table with reference-lap deltas inside Track Map | Partial. No separate default worksheet, named section import/setup, or section trend charts. |
| Channel Report: two configured report components | Generic min/max/average/stddev report of currently plotted channels | Different workflow. The i2 default reports **missing configured channels for this file**, so it is not a usable numerical reference without reconfiguration. i3rs has no independently configured report channel list. |
| Playback button and speed selector in Circuit toolbar | Synchronized hover/click cursor | Playback is absent from the current app implementation; cursor inspection is available. Playback behavior itself was not exercised. |

Default i3rs worksheets are Driver, Braking, Engine, Fuel/Ign, Mixture Map,
Oil Pressure, RPM Histo, and Spare. Track Report, Channel Report, and Section Times
are not default worksheets, although generic map and report panels can be added.
Lap overlays, alignment/stretching, unit conversion, workspace persistence,
math, and distance-axis controls already exist; they should not be listed as
wholly missing features. Distance-axis correctness is the separate P1 above.

Implementation references:
[defaults](../../crates/i3rs-app/src/default_layouts.rs),
[mixture map](../../crates/i3rs-app/src/panels/mixture_map.rs),
[histogram](../../crates/i3rs-app/src/panels/histogram.rs),
[scatter](../../crates/i3rs-app/src/panels/scatter.rs),
[track map](../../crates/i3rs-app/src/panels/track_map.rs),
[report](../../crates/i3rs-app/src/panels/report.rs).

## Further code-level correctness concerns to verify in the GUI

- **Track selection scope:** Track Map renders and searches the whole session's
  GPS trace. Its nearest-point lookup does not restrict candidates to the selected
  lap or zoom range. On overlapping laps this makes cursor picking ambiguous and
  differs from a lap-specific analysis workflow. Verify after selecting a timed
  lap, then hovering a shared section of track.
- **Sector timing precision:** Current timing chooses the nearest GPS sample to
  each boundary. This log's GPS rate is 20 Hz, giving 50 ms sample spacing, while
  i2 displays section times to milliseconds. Matching decimal formatting alone
  is insufficient; numerical parity requires equivalent section definitions and
  crossing interpolation. No equivalent-boundary timing comparison was completed.

## Additional numeric checks

The Out Lap is `[0, 351.680)` seconds in both applications after the lap fix.
i2 RPM Histo reports **7,031 samples**. Native extraction contains 7,034 sample
positions, with the first three invalid, giving exactly **7,031 finite RPM samples**.
This independently supports the startup validity fix in an analysis worksheet.

Native lambda-bank-1 extraction has 6,821 finite samples in that interval;
6,801 lie in the displayed 0.7–1.2 lambda range, matching the 6,801 shown by i2's
Mixture Map. This is a consistency check, not a complete reproduction of i2's
scatter filtering. Bank 2 and the oil-pressure scatter counts were not reconciled
fully; their sampling and boundary conventions need a controlled export comparison.

Evidence: [native Out Lap extraction](../../target/i2-comparison/circuit-outlap.json),
[oil/acceleration extraction](../../target/i2-comparison/circuit-oil-outlap.json).
The prior beacon, metadata, validity, and numeric regression results remain in
the earlier report. The test suite was not rerun during this documentation-only
pass; the distance probe was compiled and run against the existing release build.

## Visual evidence and remaining scope

Captured directly in the Circuit workspace:

- [Track Report](../../target/i2-comparison/i2-circuit-track-report.png)
- [Section Times](../../target/i2-comparison/i2-circuit-section-times.png)
- [Mixture Map](../../target/i2-comparison/i2-circuit-mixture-map.png)
- [RPM Histo](../../target/i2-comparison/i2-circuit-rpm-histo.png)
- [Oil Pressure](../../target/i2-comparison/i2-circuit-oil-pressure.png)
- [Channel Report missing-channel state](../../target/i2-comparison/i2-circuit-channel-report.png)

The native Fuel/Ign layout was inspected again; its three-decimal lambda values,
20 lap segments, and 65:18.210 endpoint remain visible. Earlier
[native evidence](../../target/i2-comparison/i3rs-fixed-precision.png) also records
the cramped gauge/readout layout. After the Mac was unlocked and the test app restarted, native Mixture Map,
RPM Histo, Oil Pressure, Report, Track Map, and Distance controls were exercised.
The native captures confirm heatmap cells, raw-count histogram bars, the XY oil
plot, and the report/map/distance failures above. The Track Map initially opened
as a blank separate window; closing it docked the map and allowed inspection.
This observation alone does not establish a persistent pop-out defect.

Native evidence:

- [Mixture Map](../../target/i2-comparison/i3rs-circuit-mixture-map.png)
- [RPM Histo](../../target/i2-comparison/i3rs-circuit-rpm-histo.png)
- [Oil Pressure](../../target/i2-comparison/i3rs-circuit-oil-pressure.png)

**Range semantics:** clicking Out Lap in the native lap list highlights it but
does not change the shared zoom range. The native histogram therefore still
shows the full outing (including a bin above 8,000 samples, exceeding the entire
Out Lap's 7,031 valid RPM samples). Mixture Map and Scatter likewise use
`zoom_range`, not `selected_lap`. These native screenshots validate layout and
behavior, not equal-lap numeric distributions. The histogram's “10%” control is
Y-axis headroom, not percentage normalization.

Remaining numerical checks: explicitly equal time/zoom ranges for the native
distributions, comparable section boundaries, lap-scoped track picking after
GPS cleanup, and a configured i2 channel report.
The evidence combines direct i2 observations, native inspection, and source/probe
checks; it is not a claim that every native panel passed a live end-to-end test.

Suggested order: repair distance generation and GPS fix handling, make reports
independent of visible graphs, add normalized/color-banded distribution views, then complete the named
track and section-analysis workflow. Keep the current heatmap and generic panels
as useful alternatives.

Screenshots and diagnostic artifacts remain local under ignored
`target/i2-comparison/`; cleaning that directory removes this working evidence.

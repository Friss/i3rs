# I3R-4–I3R-6 implementation and validation — 2026-10-16

Implementation and validation for [I3R-4](https://linear.app/exotherm/issue/I3R-4/w04-i2-parity-shared-comparisons-and-graph-interaction), [I3R-5](https://linear.app/exotherm/issue/I3R-5/w05-i2-parity-workspace-and-component-configuration) and [I3R-6](https://linear.app/exotherm/issue/I3R-6/w06-i2-parity-distributions-and-specialized-calculations), on `friss/i3r-4-6-parity`. Unrelated local changes are preserved outside the PR. No private recording is added to Git.

The implementation covers the three workstreams' UI, persistence, calculation, export and comparison consumers. Validation uses the native macOS release, Windows i2 **Standard 1.1.4.457** in VMware, known numerical fixtures and saved-workspace round trips. The user confirmed that i2 Pro is unavailable. Pro-specific behavior has implementation and automated coverage, but **has not received live Pro differential validation**. Review must retain that distinction; this report does not close every historical audit finding or claim full i2 Pro parity.

## Delivered behavior

### I3R-4 — shared comparisons and graph interaction

- Shared Main/Overlay roles, visibility, colors, lap selection, offsets, stretch and clear across worksheets. External Main swaps load the selected recording asynchronously, rebase comparison paths/offsets and rebind component slots. Missing external files retain their references.
- Each recording resolves physical channels, aliases, source precedence, scoped constants, corrections and derived dependencies independently. Background evaluation caches results/failures by the relevant settings. Failures are visible; disconnected workers finish with an error instead of waiting forever.
- GPS alignment uses an actual GPS clock channel, including aliases and compatible time units. Time-of-day clocks require compatible recording dates. Missing or incompatible clocks report a failure and suppress the unaligned overlay.
- Continuous variance uses **Overlay elapsed time minus Main elapsed time**, the same distance origin, manual offset and stretch as graph traces, and the native Main distance grid. Slower overlays are positive. Uncovered distance is omitted.
- Cursor A/B values, difference, endpoint average, secant slope, time/distance deltas and native interval min/max/mean/stddev/RMS/integral/counts. Overlay intervals use the shared correspondence mapping and report their source intervals.
- Explicit zoom/pan, full-outing/selected/A–B ranges, saved-range restore, offset adjustment/reset, annotations, beacon and event/error layers. Linked, full-outing Overview, Independent and FollowCursor modes have separate range contracts and optional local cursors.
- Native detached worksheets share session/cursor/comparison state and retain link policy and window geometry. Closing a worksheet window docks it back. Existing track-map pop-outs are preserved.

### I3R-5 — configuration and persistence

- Version-2 workspace definitions save shared comparisons, dock splits/tabs/floating topology, detached-window geometry and component properties. Unversioned/legacy JSON and legacy local comparisons migrate.
- Common portable channel slots retain missing references, aliases, source units and display transforms throughout graphs, gauges, histograms, scatter, mixture, FFT, reports and calculations. Physical, derived, generated and scoped constant sources can rebind after recording/settings changes. A pending derived result does not remove its slot.
- Gauge properties include fixed scales, precision, arc angles, ticks, font size/family and low/high warning/fault display bands. Unit changes reproject configured scales/thresholds. Digital and steering widgets honor the same font/precision settings. These are offline visual bands, separate from future live alarms.
- Worksheet rename/navigation and default F11/F12 bindings; editable application shortcuts, font/color choices, help, English/French/Japanese navigation/options/help. Domain-specific component text and channel names remain English/original-source text. Japanese glyphs use an OFL-licensed Noto Sans JP subset with provenance included beside the font.
- User preferences remain outside workspace/project files. Native saves use one coalescing writer and atomic replacement rather than competing write threads.
- Searchable channel dropdowns stay open while editing their search field and close on source selection.

### I3R-6 — distributions and calculations

- Histogram count/width/centering, shared Main/Overlay bin edges, exclude/clamp/separate overflow and count/percent/native-occupancy seconds. Included samples determine auto bounds; rejected gates do not expand them.
- Per-component gates include finite positive raw results only. Zero, negative, missing and unavailable gates exclude samples. W03 trusted-range checks apply at each native timestamp. Missing inputs report their status.
- Independent load/color-band visibility/colors; changing presentation does not change totals or denominators. Generic scatter has a separate color channel and density display that retains its native population.
- Linear regression, finite pair counts, slope/intercept/R², fit display and CSV. Constant X has no fit; constant Y has undefined R². A UI action creates editable recording-scoped math channels for the coefficients/R² using the captured range and gate/trust configuration. Polynomial fitting is not implemented.
- Scatter X-interval means and mixture per-load-band X-interval means use native included pairs, draw mean markers/curves, and export the same rows with X units, counts and source selection metadata. This is distinct from time-window averages.
- Mixture Map delay selection is expressed on the lambda timeline: pair X/load at `t` with lambda at `t + delay`, with `t` inside `[selection.start − delay, selection.end − delay)`, clipped to the recording and source coverage.
- Mixture RPM exclusion uses a centered least-squares slope over the configured odd native window and a non-retriggered exclusion period. Defaults are +2500/−2500 rpm/s, 0.500 s hold and 0.250 s filter time. Missing slope windows are excluded. Counts below validate this kernel against the available reference.
- Interval/Suspension Analysis panel consumes W02 slots, W03 ranges and shared overlays. Time-window averages and suspension summaries have CSV exports. Suspension accepts position or supplied velocity, sign inversion, centered finite-window smoothing, stationary deadband and low/high bump/rebound split. Position velocity is a native backward difference; missing samples are never bridged.
- Suspension bar/line histograms share the summary population, support count/percent/occupancy seconds and export their bins. Different units use separate plot groups. Gates apply after velocity derivation so an excluded position timestamp does not fabricate a derivative discontinuity.

## Matched i2 Standard checks

Reference recording: local May 8 VIR outing, `S1_#28299_20260508_174455_02.ld`. Lap 3 is `[814.574, 948.688)` seconds; Lap 4 is `[948.688, 1076.961)` seconds. Lambda delay is 0.150 s; minimum lambda is 0.500. The eleven centered throttle bands run from 0% through 100%.

The totals and **every band count** agree for both lambda banks:

| Selection / RPM exclusion | Total per bank | Band counts, low to high |
|---|---:|---|
| Lap 3, disabled | 2682 | 733, 452, 122, 56, 56, 24, 25, 14, 18, 39, 1143 |
| Lap 3, enabled, 0.250 s filter | 2612 | 686, 442, 116, 54, 56, 23, 25, 14, 17, 38, 1141 |
| Lap 3, enabled, 0.500 s filter | 2622 | 702, 441, 115, 54, 56, 23, 25, 14, 17, 38, 1137 |
| Lap 4, enabled, 0.250 s filter | 2456 | 747, 180, 91, 100, 28, 11, 15, 11, 14, 22, 1237 |

Out-lap unfiltered totals also match: Bank 1 **6821**, Bank 2 **6833**. Their lowest bands are 6794/6806, with identical remaining bands `3, 3, 2, 1, 1, 0, 1, 4, 6, 6`. Existing live-reference RPM histogram, oil scatter, beacon/validity, corrected-speed and VIR section timing checks continue to pass.

For variance, Main is Lap 3 and Overlay is Lap 4, with zero manual offset and no optional distance stretch. i2 status-bar distances are rounded to metres. The regression allows **50 ms**, one native 20 Hz distance interval. This validates sign and curve at sampled locations, not millisecond-identical proprietary calculation internals.

| Distance (m) | i2 variance (s) | i3rs nearest native distance point (s) |
|---:|---:|---:|
| 975 | −3.129 | −3.172197 |
| 2626 | −2.377 | −2.419048 |
| 3909 | −2.894 | −2.930062 |
| 4276 | −4.932 | −4.961451 |
| Curve minimum | −5.459 | −5.471812 |
| Curve maximum | +0.015 | 0.000000 |

The maximum observed discrepancy at these locations is 43.2 ms. The implementation retains exact metadata lap boundaries and interpolated distance origins; it does not introduce guessed timestamp snapping to fit this one outing.

## Automated verification

All commands pass:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test -q --workspace
cargo test -q --workspace -- --ignored --skip native_sampling_render_benchmark
cargo build --release
cargo check -q -p i3rs-app --target wasm32-unknown-unknown
cargo fmt --all -- --check
cargo publish -p i3rs-core --dry-run
git diff --check
```

- PR-isolated regular suite: **275 passed**, 16 ignored, zero failures (110 app + 15 CLI + 128 core + 22 validation). The earlier full working-tree run passed 277 tests; its two additional pre-existing web-sample/track-label tests belong to unrelated local changes excluded from this PR.
- Data-dependent/reference checks: **15 passed**, zero failures. The separate native sampling/render performance benchmark was intentionally excluded; there is no new performance claim.
- Coverage includes independent derived overlays, external role swaps/rebinding, GPS clock units/date compatibility/failure, distance offset/stretch/no extrapolation, view ownership, legacy migration, missing-slot repeated saves/rebinding, all analysis properties/topology/window round trips, gauge unit propagation, preference defaults, signed/missing mixed-rate gates, bin/overflow boundaries, occupancy denominators, regression degeneracy/reusable outputs, X/time interval exports, exact RPM-band fixtures and suspension rate/gap/sign/filter/split behavior.

## Native GUI verification

- Release opens both bundled VIR and the local May recording; default alias-bound graphs/gauges and analysis worksheets render.
- Lap 3 mixture banks each show **2612** samples and the matched band counts. Enabling band means keeps counts unchanged. Native Save-dialog CSV export produces **250 populated 100 RPM rows**, whose counts sum to **2612** and reproduce all eleven displayed band totals. Units are `rpm`/`LA`; selection metadata remains `[814.574, 948.688)` seconds.
- Oil-pressure scatter shows **6706** native pairs; its linear fit is `y = −3.234299 x + 56.344797`, `R² = 0.173233`. Creating scoped maths produces three evaluated 50 Hz channels with coefficient units. This is a native i3rs workflow check, not a live Pro regression comparison.
- French and Japanese options/navigation render, including Japanese glyphs; English was restored. F11/F12 worksheet navigation works.
- Detached mixture worksheet renders the same shared 2612-sample populations and returns to its owner on close.
- Native workspace save/reload restores the split layout, Main selection, calculation settings and scoped regression definitions. The fixed source picker accepts search text without dismissing its dropdown. The velocity panel renders its histogram and seven result rows using Clutch Position as a generic millimetre input (671 included velocities); this exercises the UI and does not treat clutch travel as physical suspension telemetry.
- External Main swap passes in both directions, May Lap 3 → bundled VIR Lap 1 → May Lap 3, including shared distribution presentation and source labels. Clearing comparisons restores both May mixture totals to 2612. The pass found and fixed a layout-restore wait when every scoped math definition settles locally with an unavailable-source error, plus overlay controls expanding narrow panels. Both cases now have regression tests; the final native toolbar wraps within its panel.

## Validation boundaries for review

Live Pro checks remain unavailable for Pro-only dual-cursor, component-gate, regression, suspension and advanced workbook behavior. These paths have known-data, population, persistence and native i3rs checks. Reference expectations are informed by the official [i2 1.1.4 feature guide](https://www.motec.com.au/hessian/uploads/i2_V1_1_4_Feature_Guide_0d03b00fa8.pdf). They do not establish live Pro equivalence. GPS clock alignment is covered with compatible/incompatible synthetic clocks rather than a simultaneous multi-logger reference capture. Further W07 consumers remain in their own ticket scope.

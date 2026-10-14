# I3R-3 ranges, corrections and track semantics — 2026-10-14

Implementation for [I3R-3](https://linear.app/exotherm/issue/I3R-3/w03-i2-parity-ranges-corrections-and-track-semantics), covering W03's implementation for C03, G06, G07, G08, G09 and G21. The ticket remains In Progress for matched reference acceptance and downstream workflows. The [user guide](../ranges-and-corrections.md) documents the editors, persistence and numerical policies.

## Delivered behavior

- Recording-owned ranges have stable IDs, names, exact `[start,end)` times, Lap/Out Lap/In Lap/Run/Stage/Transport types, trust, comments and optional run reaction time. Selection retains type/trust separately from view zoom. The browser, timeline and Values expose these semantics. Stage Start/End markers generate intervening transports with independent stage/transport numbering.
- Range and marker editors support recorded beacon extraction and directed GPS gate generation, including off-grid crossing interpolation, minimum intervals and missed-beacon trust flags. Invalid fixes remain gaps; missed beacons do not create invented crossings. Ranges reject duplicate identities, overlaps and invalid recording bounds.
- Correction settings expose enable flags, the fastest-wheel pair, ordered speed fallbacks, longitudinal wheel-lock prediction and clamp actions. Distance supports corrected-speed integration, explicit recorded distance with reset unwrapping, configurable section stretch tolerance and optional track length. Stretching receives competition ranges selected by type rather than relying on editable names.
- GPS settings expose sources, interpolation, repeated-fix interpolation, manual/automatic advance, quality validation and speed zeroing. Independent Corr GPS Speed remains available when wheels supply Corr Speed. Generated Corr Speed, Corr Dist and Corr GPS Speed are usable through MoTeC-source maths and all-channel Values. Failures reach Channel Status / Messages, and recording/settings keys reject stale background results.
- Track sections retain stable identities, Corner/Straight/Other classification, categories and geographic/distance boundaries. The existing 33-section VIR model now carries explicit semantics without changing its distances. Section names, kinds, categories and boundaries are editable; distance edits update their map markers. Track exports include semantics in version 2 and still read version 1.
- C03 annotations use maximum speed on straights, minimum speed on corners and native gear transitions at actual event positions, including wrapped sections. Their visibility, speed units and precision are explicit. Missing GPS omits the affected event. Section-start speed/gear-range summaries are replaced by event labels; section names remain independently selectable.
- Editable driver, venue, vehicle, event, notes and custom fields persist with the recording. Workspace/project files store versioned recording-analysis records; native opening discovers an adjacent `.i3rs.json` sidecar. Invalid or mismatched sidecars produce diagnostics and cannot attach edits to another recording. Content identities keep identical moved/copied recordings associated with their settings.

## Reference evidence and explicit limits

The saved i2 Standard 1.1.4.457 [Track Report properties](evidence/i3r-3/45-track-properties.png) specify Section Maximum on Straights, Section Minimum on Corners and Gear On Channel Value Change. The [Stage Editor](evidence/i3r-3/131-stage-editor.png) distinguishes stages from transports and marks trusted stages.

The inspected [corrected-speed](evidence/i3r-3/59-corrected-speed.png), [distance](evidence/i3r-3/60-corrected-distance.png), [GPS](evidence/i3r-3/61-corrected-gps.png) and [GPS-speed](evidence/i3r-3/62-gps-speed.png) dialogs establish the exposed policy families. They do not establish the exact i2 wheel-lock or automatic GPS time-shift kernels. i3rs uses documented longitudinal prediction and a fraction of the median finite fix-change interval respectively; these additional choices are not recorded as numerically identical to i2.

A fixture regression checks that default speed, integrated distance, GPS coordinates and timestamps remain bit-for-bit equal to the existing builders. Automatic graph distance preserves its existing monotonic recorded/derived-distance preference; map distance keeps corrected-speed integration. Corrections are recording-owned. External comparisons still need W04 to consume each recording's edited policies consistently.

The live VM was readable and showed i2 with no log loaded. Keyboard input did not open a log picker, and a subsequent click returned `noWindowsAvailable`. Launching the new native validation bundle timed out. Consequently a fresh matched GUI/annotation comparison was not completed. Headless egui renders the editor/diagnostics, while numerical/persistence regressions exercise the underlying changes. Applicable live Pro checks remain outstanding.

## Validation

All commands used `DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 235 passed; 11 reference/performance tests ignored |
| `cargo test --workspace -- --ignored --skip native_sampling_render_benchmark` | 10 local reference regressions passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo build --release` | Passed |
| `cargo check -p i3rs-app --target wasm32-unknown-unknown` | Passed |
| `cargo publish -p i3rs-core --dry-run --allow-dirty` | Packaging/verification passed; no upload |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

Focused regressions cover off-grid beacon/GPS/range boundaries, stage/transport trust and numbering, missed-beacon flags, invalid GPS, duplicate interpolation, manual/automatic shift, independent GPS-speed validity, speed priorities/clamps/wheel-lock gaps, distance stretch continuity/tolerance, section wrap and event placement, metadata/category persistence, stale job rejection, generated maths units, unchanged workspace restore, deferred project restore during recording loads, recording switches and sidecar identity/JSON rejection. Existing parser, native-sample, corrected-distance and VIR section-timing reference tests pass.

## Acceptance still required

C03's matched selected-lap event/position comparison and live verification of the added correction choices remain open. W04 owns shared comparisons and applying edits to external overlay workflows; W06/W07 must consume type/trust in their selectors and statistics where applicable. W08 adds complete Drag/Rally profiles and run milestone/time-slip reports. W09 owns raw `.ld` rewrite/export and database/output workflows. Saved analysis metadata does not rewrite the raw log, and no audit finding is closed merely because its editor or sidecar now exists.

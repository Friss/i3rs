# I3R-1 correctness repairs — 2026-10-14

Implementation for [I3R-1](https://linear.app/exotherm/issue/I3R-1/w01-i2-parity-immediate-correctness-repairs), covering C01, C02 and C06. Automated checks pass; the remaining live reference acceptance checks are listed below.

## Changes and numerical contract

- **C01:** GUI CSV export uses the active readout registry's display scale, offset, unit and precision for physical and math channels. Each cell is `raw * display_scale + display_offset`; headers are `Name (unit)` with CSV escaping, or `Name` for unitless channels. Source arrays stay unmodified. Missing samples and non-finite transformed results remain empty cells. The existing highest-rate nearest-neighbor resampling and time-window behavior are retained for this bounded repair.
- **C02:** New RPM Histo worksheets use 90 bins over 0–9000 rpm (100 rpm each), percentage output and ten non-centered throttle bands over 0–100%. Underflow, overflow and missing-color populations remain visible. Non-centered bands use `[lower, upper)` with the final outer endpoint included; centered Mixture Map bands retain eleven centers and their existing endpoint behavior. The subsequent live Lap 3 comparison corrected the initial handling of exactly 100% throttle, moving 566 samples from overflow into the top band to match the reference populations. Existing saved workspaces retain their configured bin counts/bands; saving and reloading the corrected settings preserves them.
- **C06:** Spare requests `CP Engine Speed Targeting Throttle Control Feed Forward` and includes `ECU Acceleration Z` in both its channel list and acceleration group.

The core `ExportChannel` public struct now requires `unit`, `display_scale` and `display_offset`. Callers exporting source values should supply the source unit, scale `1.0` and offset `0.0`. This change concerns the GUI's wide CSV export; the CLI analysis output has its own documented format.

## Validation

All commands used `DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 194 passed; 11 reference/performance tests ignored |
| `cargo test --workspace -- --ignored --skip native_sampling_render_benchmark` | 10 passed against the local May 8 recording |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo build --release` | Passed |
| `cargo fmt --all` and `git diff --check` | Passed |

Focused regressions cover kPa→psi, C→°F including offset, mixed rates, output precision, negative scale, escaped headers, missing/overflow values, readout/export agreement, RPM/throttle boundary populations and percentage totals, independent centered Mixture Map defaults, settings JSON round-trip, and Spare resolution/grouping with channel-bearing fixture metadata. Existing parser, beacon, native-sample, corrected-distance, section-timing, scatter and mixture reference regressions pass.

## Reference evidence and remaining acceptance limits

The implementation was checked against the saved [RPM bin settings](evidence/i3r-1/40-histo-properties.png), [throttle bands](evidence/i3r-1/42-histo-color.png) and [Spare view](evidence/i3r-1/54-spare.png) from i2 Standard 1.1.4.457. The throttle table labels intervals lower-inclusive/upper-exclusive, including its last interval. However, the [fresh Lap 3 populations](i3r-1-3-live-validation-2026-10-14.md) place all samples decoded by i3rs as exactly 100% into the top band. The candidate now includes that final endpoint to reproduce the measured distribution; this does not establish i2's internal boundary arithmetic from the table alone.

The local reference recording `S1_#28299_20260508_174455_02.ld` contains neither Feed Forward nor ECU Acceleration X/Y/Z. The reference Spare screenshot lists them without displayed values. A fixture with those names verifies resolution and tile placement; a local-reference test verifies missing names do not bind to unrelated channels. Resolving these channels against an actual recording that contains them remains an acceptance check. Preserving visible unresolved worksheet slots belongs to W05.

The prior i2 export-header capture in `target/i2-comparison/i2-export-header.png` shows a MoTeC metadata preamble and a separate channel-name row. It does not expose the later unit row or establish which unit transformations i2 exports. Thus `Name (unit)` is i3rs' explicit export contract, not a claim of identical MoTeC CSV formatting or complete G20 parity. A fresh i2 exported-file comparison remains needed.

The repaired release app opened the May 8 recording and rendered its default Driver worksheet. Desktop automation could not activate RPM Histo reliably and subsequently returned `noWindowsAvailable`; the live i2 VM was also locked. The matched interactive RPM/CSV checks were therefore not completed. Automated worksheet configuration, downstream exporter and persistence checks passed, but C01/C02/C06 are not recorded as fully closed by this acceptance record.

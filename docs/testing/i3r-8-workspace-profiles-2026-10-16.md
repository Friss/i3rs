# I3R-8: workspace profiles validation

Reference: [I3R-8](https://linear.app/exotherm/issue/I3R-8/w08-i2-parity-complete-workspace-profiles), i2 **Standard 1.1.4.457** running in VMware Fusion / Windows 11 Arm. Circuit and Engine are the configured audit references; Drag and Rally use the installed MoTeC samples. Usage and calculation contracts are in [workspace-profiles.md](../workspace-profiles.md).

## Reference comparison

The live pass checked Circuit Driver trace/gauge grouping, workspace switching, Drag run details and distance reports, and Rally stage reports. The earlier 33-sheet audit captures informed the complete compositions. Corrections from this pass include Driver's Oversteer gauge and grouping, Cam Control's separate nine-gauge arrangement, standalone Drag RPM histogram, Rally overview above its RPM histogram, manifold-pressure histogram bands, differential channel aliases, and per-channel Rally report statistics.

The vendor samples were copied locally from `C:\MoTeC\Logged Data\Samples Standard` for matched checks. They are not distributed in this PR. `i2_specialty_reference.rs` contains only transcribed expected values and can run with `I3RS_I2_SAMPLES` pointing to the local files `Drag.ld`, `Drag.ldx`, `Rally.ld`, `Rally.ldx`.

A matched failure exposed the legacy ADL clock: header +0x42 is 1,024,000 rather than 1,000,000. Applying nominal integer rates had put the wrong samples at the run origin. The parser now exposes the physical fractional rate (102.4 Hz for Drag RPM, 20.48 Hz for Rally RPM/oil pressure, 1.024 Hz for Rally coolant). No samples are resampled or discarded. The same rate flows through corrections, graphs, maths, reports, distributions, CSV and overlays. A synthetic regression checks both clock values and unchanged sample validity/data.

### Drag

LDX imports Run Q1 beginning **176.750 s**, measured ratios **1.60 / 1.28 / 1.00**, reaction **0.400 s**, and cumulative ET **1.200 / 2.810 / 4.070 / 5.160 / 6.138 s**. Reaction remains separate. Staging and Post are Transport intervals. Boundaries use physical seconds; no fixture-specific timestamp adjustment exists in the tests.

| Split | i2 RPM min / max / avg / start / end | i3rs displayed |
| --- | --- | --- |
| 0–60 ft | 5327 / 8240 / 7686 / 5327 / 8240 | Matches |
| 60–330 ft | 8240 / 8904 / 8612 / 8240 / 8815 | Matches |
| 330–660 ft | 8796 / 9114 / 8957 / 8815 / 9089 | Matches |

The native UI showed the same rows and all five milestone groups. Upper channel and lower time reports use the common table/chart/CSV population. Multiple-statistic charts display the reference Min/Max/Avg series. Official time-slip values override computed distance crossings individually. Synthetic regressions also cover missing/nonmonotonic distance, off-grid crossings, no crossings outside a run, missing endpoints, cumulative versus split windows, reaction exclusion and matching-milestone comparison deltas.

Evidence: [i2 reports](evidence/i3r-8/i2-drag-reports.png), [i2 Run editor](evidence/i3r-8/i2-drag-run.png), [native reports](evidence/i3r-8/i3rs-drag-reports.png).

### Rally

LDX pairs import Stage 1 **7.9848535–141.984848 s** and Stage 2 **157.841872–277.841856 s**, with Transport gaps. Lap Number values 684–688 do not replace the stage intervals. Marker trust remains Unknown; undocumented flag bits are not inferred as trust.

| Stage | Oil min (kPa) | RPM min / max / avg | Coolant max (°C) |
| --- | --- | --- | --- |
| 1, both apps | 343.9 | 1176 / 5730 / 3561 | 94.9 |
| 2, both apps | 347.1 | 1068 / 6102 / 3690 | 92.6 |

The native Channel Report contains selected-stage and all-stage components, each with table/chart split, and excludes Transports. Charts honor Oil Min, RPM Min/Max/Avg and Coolant Max rather than trying to draw unavailable Avg values for every channel. Exact stored marker bounds are retained; range-duration labels differ by 1 ms because i2 truncates while i3rs rounds.

Evidence: [i2 reports](evidence/i3r-8/i2-rally-reports.png), [native reports](evidence/i3r-8/i3rs-rally-reports.png).

## Composition accounting

Every slot is explicitly declared in `default_layouts.rs` / `default_layouts/profiles.rs`, including unavailable channels. No worksheet is conditional on recording channel availability.

| Profile | Worksheet | Components and distinguishing slots |
| --- | --- | --- |
| Circuit | Driver | Tiled graph, speed/RPM/steering/Oversteer/G-force/gear gauges; Oversteer, weighted Oversteer, inlet/ambient air |
| Circuit | Braking | Tiled graph; derived front/rear left-minus-right hub RPM |
| Circuit | Engine | Tiled graph and health gauges; temperatures, pressures, voltage, warnings |
| Circuit | Fuel / Ign | Tiled graph and fuel/ignition gauges; both lambda bars and digitals, cam/trim channels |
| Circuit | Mixture Map | Dual-bank mixture heatmaps |
| Circuit | Oil Pressure | Lateral-G/oil-pressure scatter above an oil-pressure graph, retained without oil data |
| Circuit | RPM Histo | RPM distribution components |
| Circuit | Track Report | Track map with report controls |
| Circuit | Channel Report | Shared channel report |
| Circuit | Section Times | Track/section timing components |
| Circuit | Spare | Configured torque-control, throttle, limits and accelerometer groups |
| Engine | General | Tiled graph and health gauges; RPM/limit/idle, lambda/aim/trims, temperatures, warnings |
| Engine | Cam Control | Tiled graph and nine gauges; cam aims/positions/control, oil warning |
| Engine | Knock | Tiled graph; six knock levels, threshold, ignition compensation, trims |
| Engine | Fuel Usage | Tiled graph; tank level/voltage, usage, injector duty, six knock trims |
| Engine | Wheel Speeds | Tiled graph; four hubs, vehicle/GPS speed, boost, fan, traction cut |
| Drag | EGTs | Tiled graph and ten gauges; eight exhaust temperatures and three shaft/RPM channels |
| Drag | Drive Line | Tiled graph; shafts, clutch slip, longitudinal G, front/corrected speed, Gear |
| Drag | Fuel | Tiled graph; shafts, pump pressure, corrected speed, throttle |
| Drag | Engine Health | Tiled graph; shafts, oil/manifold pressure, ambient/inlet air, voltage |
| Drag | Acceleration | Run graph, longitudinal-G gauge, independent staging overview |
| Drag | Boost | RPM/manifold-pressure scatter colored by throttle |
| Drag | Reports | Channel and elapsed-time reports with split charts; five distance milestones |
| Drag | RPM Histogram | Standalone RPM histogram, 60 bins, 4000–10000 rpm, centered throttle bands |
| Rally | Driver | Tiled graph and gauges; steered angle, G, gear/brake |
| Rally | Engine | Tiled graph and health gauges |
| Rally | Fuel / Ign | Tiled graph and dual-bank fuel/ignition gauges |
| Rally | Mixture Map | Dual-bank mixture heatmaps |
| Rally | Boost | RPM/manifold-pressure scatter colored by throttle |
| Rally | RPM Histogram | Stage/transport overview above histogram; 80 bins, 0–8000 rpm, manifold-pressure bands |
| Rally | Diff Controller | Tiled graph and gauges; ground/front/rear/four-wheel speeds, slip/lock/duty/current/mode |
| Rally | Channel Report | Selected and all-stage reports with per-channel statistics and split charts |
| Rally | Spare | Engine RPM graph |

Round-trip tests compare every saved component, channel slot, tile group, gauge style, dock topology and report policy after restoring with all physical channel names missing. A headless render test visits all 33 worksheets. Changing recordings retains the sheet set and missing slots. Profile switching has an Arc-identity regression for retaining the loaded file, selection, cursor, zoom and edited analysis. The native pass also switched Drag → Rally with the file/ranges retained and loaded Rally while Drag's complete sheet set remained present.

## Checks

- `cargo test --workspace --quiet`: **313 passed, 18 ignored**, no failures. Ignored checks require local fixtures or manual/performance environments.
- Matched core reference checks: **4 Circuit + 2 specialty passed**.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo check -p i3rs-app --target wasm32-unknown-unknown`: passed.
- `cargo build --release`, `cargo fmt --all -- --check`, `git diff --check`: passed.
- Built and opened `/Users/friss/Applications/i3rs Dev.app` through Computer Use, using native file dialogs. Verified its persistent `io.github.friss.i3rs.dev` signature with the existing certificate `eaf787aad55ebb71b723f35f3608dd169e37cd34`.

Generated Drag maths use the shared evaluator and recording ownership; focused tests cover ratio changes with stable IDs, disabled outputs, zero/neutral/missing inputs, sidecar persistence and external comparison contexts. Existing native-sample, parser and section-timing regressions remain passing. Multiple chart statistics round-trip and render through the same report values as the table/CSV.

## Deliberate differences and limits

Selecting a profile in i3rs keeps the file loaded, while the observed i2 workspace switch cleared it. Circuit Oversteer requires real road-wheel angle and wheelbase inputs; missing setup remains visible rather than assuming vehicle geometry. Gauge strips and report tables use i3rs' existing egui layout, so this validates component/slot composition and the stated numerical populations, not pixel-identical nested gauge geometry or every possible source-channel statistic. Pro-only behavior is outside this Standard comparison. The issue remains open for PR review; this record does not change its status automatically.

## Review corrections — October 17

All three initial review findings were addressed. Legacy saved analysis migrates through sidecar loading, workspace restore, deferred recording activation and comparison contexts, gated by file identity and the known nominal duration. Tests preserve edited ranges, markers, trust, selected identities, details, ratios and official time slips, migrate timeline offsets, reject unrelated/invalid data and verify repeated save/load is idempotent. The source sidecar is not rewritten during loading.

Both Drag and Rally Boost now use Throttle Position with a 0–100 color scale, including when channels are missing. The recording editor prunes orphaned time slips before Apply and after range edits independently of collapsible section visibility. Deletion and Run-to-Stage changes succeed on the first apply; retained invalid slips still reject Apply without changing saved analysis.

Updated validation: **320 tests passed, 18 ignored**; all **4 Circuit + 2 specialty** matched reference checks passed. Clippy with warnings denied, wasm32 check, release build, formatting and diff checks passed.

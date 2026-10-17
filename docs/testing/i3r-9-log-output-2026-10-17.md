# I3R-9: log browsing and output validation

Ticket: [I3R-9](https://linear.app/exotherm/issue/I3R-9/w09-i2-parity-log-browsing-and-output). Implementation and API policies: [log-output.md](../log-output.md).

## Automated coverage

- New writer round trips preserve source channel values/rates, enums, driver/event metadata, custom details, range identities/types/trust/comments and sidecar recording identity.
- Off-grid cropped logs reopen at the declared output grid, with exact clipped range bounds and continuous/state sampling policies. Sample storage uses f64 so decoded values do not incur a new f32 quantization.
- Replacement tests verify original log bytes remain in the recovery backup, with its filename manifest, and all three output files reopen.
- Browser tests cover metadata filtering, sidecar range discovery and deleted-log removal.
- CSV tests cover off-grid mixed-rate continuous/state values, display transforms, quoted headers, missing cells and independently sampled distance. KML tests cover escaped labels and GPS outage splitting.
- App tests cover recording/range ownership, explicit maths inclusion, source/display unit choice, all export-dialog formats, quoted multiline print cells and a self-contained PNG print page.
- Existing local May 8 i2 parser/native sample/section-timing reference tests and Drag/Rally report reference tests remain part of the validation run.

## Live reference

Inspected i2 Standard **1.1.4.457** in VMware Fusion / Windows 11 Arm on 2026-10-17. The File menu offers Export Data, Split Data, Earth Export, Create Reference Lap, Print Preview and Print to Clipboard. Export Data distinguishes Entire Outing / Current Range / Visible Data. Its CSV format exposes sample rate, timestamp and distance controls; its log format exposes included details/device configuration. These observations establish the control contract, not a claim that i3rs copies every i2 serialization/interpolation detail.

Create Reference Lap identifies a device lap gain/loss / Telemetry Monitor workflow and writes `*.td1`. A fixture was successfully generated from the bundled Rally sample's Stage 1 at `C:\Users\friss\AppData\Local\Temp\i3r9-reference.td1` (27.5 KiB as shown in Explorer). A local shared-folder destination was unavailable. Automatic approval review initially rejected the clipboard transfer for missing cross-boundary authorization. The user subsequently approved the generated fixture and split-log transfers. A source-restricted temporary host/VM endpoint transferred only these artifacts and was stopped afterwards. Clipboard/shared-folder delivery had been unreliable; no permission settings were changed. The fixture now lives in `test_data/reference/i3r9/`, with its provenance and checksum recorded there.

## Native i3rs acceptance

Rebuilt `/Users/friss/Applications/i3rs Dev.app` with `scripts/build-macos-dev.sh`. The certificate-backed designated requirement remains `io.github.friss.i3rs.dev` with leaf fingerprint `eaf787aad55ebb71b723f35f3608dd169e37cd34`; no privacy settings were changed.

- Opened VIR through its native picker. Generated and saved a CSV for `[0.125,1.375)` at 20 Hz with only Engine Speed selected. The [saved file](evidence/i3r-9/off-grid-rpm.csv) has exactly 25 rows, time `0.000000000` through `1.200000000`, units `rpm`, and first/last values `6721.4` / `6890.3`.
- Saved the [printable HTML](evidence/i3r-9/native-output.html) through the result preview. Independently parsed its table and verified every cell equals the CSV, including headers and all 25 rows. The worksheet preview also captured the visible charts and selected stage without a print job.
- Indexed `test_data` through the folder picker: six recordings. Driver query `Roger` selects Rally. Expanded details show its custom fields and five typed stage/transport ranges. Added Stage 1 as Overlay, then opened it as Main; cursor `7.985 s`, typed Stage selection and its graph zoom updated.
- Wrote selected Rally Stage 1 through the `.ld` export dialog at 20 Hz. Paired analysis records `Stage 1`, `Stage`, `Unknown` trust, `[0,133.9999945)`, with log duration `134.0 s`. The [saved analysis evidence](evidence/i3r-9/native-split-analysis.json) retains its exact range identity and custom details. Validation binaries are kept in `/tmp/i3r9-final-split.*`, not added as vendor sample fixtures.
- Reopening the split confirmed one typed Stage range and cursor origin zero. This caught loss of legacy display precision when converting scaled samples to f64: cropped logs now write the dedicated display-precision/format bytes, verified against every original channel. CLI automatic selection now uses validated `.i3rs.json` ranges; a focused CLI test covers typed range preservation and wrong-recording rejection. Explicit `--source ld` / `--source ldx` retain their independent policies.
- The final installed build was tested by switching from original Rally Stage 1 to the exported one-range log: the Main range selector remains Stage 1, typed Stage/Unknown is retained, and the cursor rebases to zero. The final split reopens with `0.02 G`, `80.8 C`, `94.9 C`, `53.2 C` and `26.3 %` at the new origin, retaining the source precision. A saved workbook’s previous Main index no longer displaces a newly opened export’s selected range identity; a two-case regression covers both valid and invalid old indices.
- Native result preview initially wrapped numeric columns too narrowly. Its final grid has explicit 150–320 point column bounds. Browser status and export status are separate, and browser Main selection also updates comparison Main ownership.

## Commands

`cargo test --workspace -q`: **341 passed**, 18 intentionally ignored local/benchmark checks. Additional local runs: `cargo test -p i3rs-core --test i2_reference -- --ignored` (**4 passed**) and `I3RS_I2_SAMPLES=/Users/friss/Desktop/Dev/i3rs/test_data/reference/i3r8 cargo test -p i3rs-core --test i2_specialty_reference -- --ignored` (**2 passed**). One initial specialty invocation used a relative fixture path from the crate test working directory; rerunning with the absolute directory passed.

`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` and `git diff --check` pass. `cargo check -p i3rs-app --target wasm32-unknown-unknown` passes. An additional strict wasm Clippy run reports three existing lints outside this change (needless returns in `app.rs`/`preferences.rs`, and `WebHandle::new` without `Default`); those unrelated functions were left intact. The final full suite includes the CLI sidecar round trip and writer display-precision assertions.

## TD1 GUI/API acceptance after transfer approval

- Added a bounded v1 TD1 decoder/encoder and selected-range device-reference output. The complete 28,192-byte i2 fixture decodes and re-encodes byte for byte. The observed layout, known/unknown fields, spatial/time units and supported variant are documented in [td1-reference-format.md](../td1-reference-format.md).
- In the signed native app, selected original Rally Stage 1 as Main through the indexed browser, chose **Device reference .td1**, and saved `/tmp/i3r9-native-reference.td1` through its native dialog. The [saved output](evidence/i3r-9/native-reference.td1) has 3,241 points, 134.00 s final time and 3,239.5 m final distance. The core decoder independently reopened it and its encoder reproduced those bytes. Distance uses the current i3rs corrected-distance model; it is not asserted identical to i2's 3,267 m curve.
- Core tests cover one-metre inversion, off-grid boundaries, stops, the exact recording endpoint, missing/backwards distance, malformed headers, truncation and full fixture equivalence. GUI/API tests cover edited metadata, recording ownership and rendering all five export formats without eagerly decoding channels.
- TD1 publication reuses the log writer's staging/recovery path. ASCII metadata is required explicitly. Physical device upload/acceptance has not been tested and is not claimed.

## Independent i2 log acceptance

After the user unlocked the Mac, the final check isolated a binary compatibility error: f64 samples with nonzero scaling exponents did not resolve as continuous channels in i2 Standard 1.1.4.457, although state channels plotted. The final writer stores engineering-unit f64 with exponent zero and keeps the dedicated display-precision bytes for consumers that support them. This supersedes the intermediate source-exponent approach. Legacy i2 rounds some displayed values; this is a formatting limitation, not a change to the stored samples.

The actual Rust writer API regenerated the 20 Hz Rally Stage 1 split and paired sidecars. These were transferred through the authorized, narrowly scoped local endpoint after closing the previous log; the endpoint was stopped afterward. i2 independently reopened the final file with Roger Outback / Belgrave / x234 / Test Session, a 134 s extent and **Stage 1 / 2:13.999** (the exact range end is 133.9999945 s). The Driver worksheet plotted RPM, corrected speed, throttle, steering, both accelerations, Gear and Brake Status. At origin i2 showed RPM 2919 and throttle 26%; the stored values are 2919.17879808 rpm and 26.252979968%.

The Stage 1 channel report resolved successfully: RPM min **1177**, max **5697**, average **3561**, and oil-pressure minimum **344 kPa**. API/native-grid values independently give RPM min 1176.56520192, max 5696.7180288, sample mean 3561.53634147, and oil minimum 343.903059456 kPa. i2's displayed average is its own report aggregation/rounding; exact identity with the sample mean is not claimed. Earlier whole-recording reporting displayed average 3562. Full arrays are checked numerically by the round-trip tests. Typed LDX Stage markers use microseconds and the proper Layer/MarkerBlock hierarchy. Generic ranges are private JSON details, avoiding ambiguous i2 MarkerPairs, and empty specialty groups are omitted.

Current API sidecar/analysis and checksums are saved as [evidence](evidence/i3r-9/output-files.json). Physical device acceptance of TD1 remains outside this validation; the complete observed i2 TD1 fixture round-trips byte-for-byte.

## Final native acceptance and completion

Rebuilt `~/Applications/i3rs Dev.app` with the same persistent signing identity and opened the final API split through its native file dialog. It retained the typed Stage range, 79 channels, 134 s extent, zero cursor origin and original i3rs display precision (80.8 C, 94.9 C, 53.2 C, 26.3%). The final GUI then exported selected Stage 1 at 20 Hz to `/tmp/i3r9-gui-final.ld`; its completion message confirmed the saved path. Comparing all **79 × 2680 = 211,720** stored samples against the API split found **zero finite difference**, with missing-value positions preserved. Reopening that GUI-created log through the file dialog restored **Main range: Stage 1**, Stage/Unknown, cursor 0.000 s and the same readings. The paired native analysis is retained as evidence.

Final validation: **341 workspace tests + 6 local i2 reference checks = 347 passed**. Native strict Clippy, web compilation, formatting, diff whitespace checks and the certificate-signed release build pass. G04/G20 and W09 are complete for the documented adapters; legacy i2 display rounding, exact cross-tool report aggregation and physical TD1 device acceptance retain the limits above.

## PR review follow-up

Three review findings were fixed with regression tests. Carried worksheet snapshots now have transient restore intent, preserving a browser-selected range even when that recording already exists in the saved analysis map and math restoration is deferred; explicitly loaded workspaces still restore their saved selection. Overlay reference export marks the chosen range identity before cropping, for both Main-session and external overlays, and uses the overlay's loaded range list when analysis is uncached. KML uses the recording's configured GPS pair, or the established GPS resolver for automatic selection, and rejects unavailable explicit sources instead of choosing a misleading latitude-named field.

The updated workspace suite passes **344 tests** (18 intentionally ignored checks). The prior six local i2 reference checks are unchanged. Strict Clippy, wasm compilation and formatting pass on the follow-up revision.

The next review added two regressions: KML emits only contiguous segments with at least two coordinate tuples, as required by the [KML LineString reference](https://developers.google.com/kml/documentation/kmlreference#linestring), and rejects disconnected singletons. Browser Main selection now shares Overlay's identity resolver and carries the range ID through deferred installation, covering reordered/inserted/removed cached ranges. A removed range returns a refresh error. The deferred-restoration regression also verifies that choosing the full recording remains unselected. The updated suite passes **346 workspace tests**, plus strict Clippy, formatting and wasm compilation.

A further review added validated external-overlay analysis loading before export, preserving saved corrections/details/range metadata and caching it for CSV distance and TD1 evaluation. Cached unsaved analysis remains authoritative; malformed/mismatched disk sidecars now produce explicit errors. “All” folder refresh scans every shortcut in one worker, and a changed recording at a pending Main path clears the stale request with a refresh error. Regressions cover all three cases.

The Windows rename-overwrite finding was checked against the [Rust rename contract](https://doc.rust-lang.org/std/fs/fn.rename.html) and [Rust 1.95 Windows implementation](https://github.com/rust-lang/rust/blob/1.95.0/library/std/src/sys/fs/windows.rs): the implementation already uses MOVEFILE_REPLACE_EXISTING. The publisher remains unchanged; expanded output round-trip tests exercise replacement locally and compile in the existing native build matrix. Windows runtime execution of these new tests is not claimed; the available GitHub OAuth credential lacks workflow-edit scope. The tests include overwriting a memory-mapped log while preserving its recovery backup and replacing TD1 output. The suite passes **350 workspace tests** locally, with strict Clippy, formatting and wasm compilation passing.

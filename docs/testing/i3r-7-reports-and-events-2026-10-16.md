# I3R-7 acceptance: reports and event analysis

Implemented W07 (G16 and D10) on 2026-10-16. This record covers configurable reports, acceleration events and stable-app acceptance.

The user explicitly accepted implementation against the documented specification for Pro features because only i2 Standard is available in VMware. Live Pro comparison is therefore not a completion gate. This record describes the implemented numerical contract; it does not claim undocumented, bit-for-bit equivalence to i2 internals.

## Reports

- Channel, Track and Section statistics use `i3rs_core::report`, including all nine statistics: Min, Max, Range, Average, Abs Max, Start, End, Change and population Std Dev. Apply display scales and offsets before aggregation.
- Select current outing/ranges, the selected range, the linked window, Main with visible overlays, selected outings, or all loaded outings. “Loaded” means the main recording and recordings loaded into shared comparisons. Merely listed project files are not loaded implicitly.
- Resolve external physical, generated and derived channels in each recording, with its aliases, corrections, units and interpolation. Retain missing slots as blank results with status diagnostics. Resolution and corrected tracks run in workers; immutable result caches retain source allocations and invalidate for population/source changes.
- Use a stable section selection or section headings, including beacon-wrapping sections. Transfer geographic section definitions onto external corrected tracks. Unavailable sections stay unavailable; never reinterpret another recording's sample indices as the same section.
- Trusted-only intersects populations with explicitly Trusted ranges. Unknown/Untrusted ranges are excluded. Native statistics use `[start,end)`; boundary mode adds exact endpoints using channel interpolation without extrapolation or duplicate start samples. Overlapping intervals are united, gaps are preserved, missing endpoints stay missing, and empty populations show `—`.
- Time Reports calculate the union duration of selected intervals, including section/trust intersections, independently of channel availability. They do not publish a speed series under an elapsed-time label in Readout.
- Select common or per-channel statistics (channel context menu). Support row/column orientation, reverse range order, extrema highlights, absolute/Main/Overlay 0 values, table/chart/split views, both split orientations, and trend lines that break over missing values. Section deltas use the matching stable section identity.
- Table cells, range labels and charts share selection. Local external-row selection highlights both views while preserving the current main recording. Main-recording range selection updates shared lap/zoom state. Linked-window selection preserves its window. G cycles report views; Shift+G changes split orientation.
- Copy report CSV from the same values displayed in the table/chart, with units, reference role and population counts. Reports retain their own channel slots. Settings, per-channel overrides and new report kinds persist through the existing backward-compatible workspace format.
- The original Section Times component, timing matrices and compact section trends remain available.

## Acceleration Report

Source: [MoTeC i2 1.1.4 Feature Guide, pp. 22–25](https://assets.motec.com.au/strapi/i2_V1_1_4_Feature_Guide_0d03b00fa8.pdf). The guide specifies report chart/split interaction, speed-point pairs, configurable speed source, hysteresis, separate acceleration/deceleration hold times, start/end offsets, time/distance/average-G outputs and exporting all zones.

Open **View → Add Acceleration Report**, or choose Acceleration in an existing report. The default source is `MoTeC::Corr Speed`; another speed channel may be selected. Points and hysteresis are edited in km/h, with calculations and structured outputs in SI units.

Hysteresis confirms the initial direction and subsequent reversals. Confirmed regions retain their actual extrema; crossings are linearly interpolated at the requested speed points, not shifted to confirmation samples. Separate hold times apply to the observed direction region. Every ordered speed pair is emitted once per accepted region. The report gives each pair's minimum/maximum and all individual events.

Padding expands the Main/selected-range search window, clipped to source timestamps and the optional trusted mask. Only complete pairs overlapping the unpadded selection are included. Overlapping padded windows are united to avoid duplicates. Invalid/negative speeds, missing samples and trust gaps break regions. No extrapolation occurs.

Time is crossing-to-crossing duration; distance is the piecewise-linear speed integral; average G is signed speed change divided by elapsed time and 9.80665. The speed plot marks Main boundaries in green. Event selection highlights its interval and moves the shared cursor. U cycles time/distance/G. Table, chart, all-event CSV and serializable `AccelerationReport`/`AccelerationEvent` results use the same engine output, ready for W09 exporters. W09's broader export-format work remains in W09.

## Verification

- `cargo test --workspace --quiet`: **299 passed, 16 existing ignored**, no failures.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo check -p i3rs-app --target wasm32-unknown-unknown`: passed.
- `cargo build --release`: passed for all workspace crates.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Coverage includes native/off-grid/interpolated statistics, missing endpoints, display transforms, trust union/gaps, wrapped and unavailable sections, stable section delta identity, independent external derived-channel evaluation, all-loaded/selected population scopes, Main/Overlay deltas, per-slot statistics, time durations/readout isolation, old workspace migration and settings round trips. Acceleration tests cover all speed pairs in both directions, exact crossings/integrals/G, unit/sample-rate invariance, hysteresis, separate hold controls, padding, trust clipping, missing gaps, invalid settings, duplicate-window suppression, and identical results in all views.

## Native stable-app verification

Built and used `/Users/friss/Applications/i3rs Dev.app`, bundle ID `io.github.friss.i3rs.dev`, consistently signed with the existing `i3rs Development` certificate. The designated requirement remained identical across rebuilds (`eaf787aad55ebb71b723f35f3608dd169e37cd34`). Reopened the app and loaded recordings through its native file picker without additional privacy or Computer Use approval prompts.

Observed with `test_data/VIR_LAP.ld`:

- Channel Report table/split chart rendered correctly. Engine Speed averages were 4955.4 rpm for All and 4964.0 rpm for Lap 1. Selecting a table cell selected Lap 1 in the browser, table and chart; Delta to Main showed −8.6 rpm for All and 0.0 for Lap 1.
- Acceleration Report produced 23 speed-pair results with defaults. The 100–140 km/h acceleration pair had five events and 3.067–5.879 s extrema. Distance mode showed 102.476–191.813 m; G mode showed 0.193–0.369 G. U changed metric. Clicking a result point highlighted the corresponding speed interval and set the cursor to its crossing (observed 12.831 s).
- Time Report showed 0.101 s Out Lap, 128.392 s Lap 1 and 4.507 s In Lap. The final UI uses Duration and leaves the active Readout free of a mislabeled speed series.
- Loaded `test_data/S1_#28299_20260415_110834_2.ld` through Add Overlay File. All loaded outings included its outing duration of 3619.710 s and its individual range durations, alongside VIR's 133.000 s outing and ranges.

i2 Standard 1.1.4.457 was observed in VMware. The existing Standard property captures and the vendor's Pro feature guide supply the reference behavior. Pro-specific acceptance uses the specification and numerical tests as requested.

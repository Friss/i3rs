# i3rs compared with MoTeC i2 — 12 October 2026

Tested `S1_#28299_20260508_174455_02.ld` (61,469,369 bytes), with its local `.ldx`, at i3rs commit `c52a4ba`. Reference: MoTeC i2 Standard 1.1.4.457 in the running Windows VMware VM. i2 is the expected behavior, including unusual recorded laps.

**Initial result:** basic numeric decoding passed the spot checks, but the baseline exposed significant correctness problems in lap detection and sample validity. The fixes and revalidation are recorded below.

The subsequent [Circuit workspace parity audit](circuit-parity-2026-10-12.md)
records an additional distance-axis regression exposed by missing samples, plus
analysis/workflow gaps. The five fixes below do not establish full panel parity.

## Fix verification — same file

All five findings below are fixed in the current working tree. The original findings
remain as a record of the baseline behavior.

| Check against i2 | Fixed result |
|---|---|
| Lap count and boundaries | 20 segments; all 19 beacon timestamps match within 1 microsecond of the millisecond values exported by i2 |
| Closely spaced crossings | Lap 2 is present and selectable at 0.059 s |
| Lambda validity | Startup and 580.000 s return missing values, not zero or stale measurements |
| Lambda display | Three decimal places; GUI shows 0.873 / 0.882 at 849.256 s and `---` inside the outage at 579.355 s |
| Channel names and units | Full extended names recovered; lateral acceleration has unit `G`, charge-cooler air temperature `C`, and calculated fuel level `l` |
| Session endpoint | 3918.210 s (65:18.210), also used for the final lap |
| Existing numeric spot checks | All 14 readings at 178.136894 s still match i2 after conversion and display rounding |

The parser now follows the optional metadata pointers for validity, full names, and
enum references. Display precision is separate from scaling. Missing samples remain
at their original time indices through partial reads and interpolation; statistics
exclude them, CSV cells are empty, JSON values are null, and chart paths break at
gaps even after downsampling. Saved workspace channel lookup accepts the old
truncated names as compatibility aliases.

The session endpoint uses the finest recorded sample grid to avoid slow-channel
padding. It matches this i2 file exactly; it remains an estimate for other file
formats that stop different channels independently. The newly decoded structures
are also covered by synthetic malformed-pointer and transition tests, plus the
checked-in cropped VIR fixture.

Validation commands (all passed):

```bash
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo test
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo test -p i3rs-core --test i2_reference -- --ignored
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo clippy --workspace --all-targets -- -D warnings
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo build --release
```

The [same-file regression test](../../crates/i3rs-core/tests/i2_reference.rs) contains
expectations transcribed from i2. It is explicitly ignored in ordinary CI because
the 61 MB source log is local, and was run explicitly here. The standard suite has
155 passing tests and four fixture/benchmark tests ignored; the explicit i2 run
adds one passing test. No proprietary log was added to the repository.

Updated evidence: [GUI precision, laps, names, and endpoint](../../target/i2-comparison/i3rs-fixed-precision.png),
[GUI missing values](../../target/i2-comparison/i3rs-fixed-invalid.png),
[fixed lap JSON](../../target/i2-comparison/laps-fixed.json),
[fixed metadata](../../target/i2-comparison/channels-fixed.json),
[missing-sample JSON](../../target/i2-comparison/invalid-values-fixed.json),
[fixed summary](../../target/i2-comparison/summary-fixed.json).

## Confirmed findings

### 1. P1 — Lap detection creates false laps and misses a recorded beacon

i2 reports **20 segments: Out Lap, Lap 1–18, In Lap**. i3rs reports **31: Out Lap, Lap 1–29, In Lap**.

| Segment | i2 duration | i3rs duration under the same label |
|---|---:|---:|
| Out Lap | 351.680 s | 112.320 s |
| Lap 1 | 462.835 s | 17.550 s |
| Lap 2 | 0.059 s | 115.230 s |
| Lap 3 | 134.114 s | 61.970 s |
| In Lap | 249.926 s | 59.070 s |

The i3rs sidebar nevertheless shows “Fastest: 0:00.059” from the sidecar, even though no selectable lap has that duration. This is internally inconsistent.

`detect_from_lap_time_running()` in [lap_detect.rs](../../crates/i3rs-core/src/lap_detect.rs) treats every timer drop greater than one second as a crossing. This file has timer resets that are not i2 beacon boundaries, including 112.320, 129.870, 245.100, and 307.070 seconds before the first real boundary at 351.680. The closely spaced i2 boundaries at 814.515 and 814.574 are collapsed into one inferred boundary at 814.501. Even otherwise aligned boundaries are roughly 3–15 ms early.

Impact: lap selection, per-lap statistics, overlays, and any derived lap comparison refer to the wrong intervals. Prefer the actual recorded beacon information, reconcile it with the sidecar, and retain i2's 59 ms segment rather than silently filtering it. A preliminary inspection of `Lap Beacon Ticks` found nonzero transitions corresponding to the reference crossings; exact timestamp decoding still needs implementation and validation.

Evidence: [i2 laps, top](../../target/i2-comparison/i2-laps-top.png), [bottom](../../target/i2-comparison/i2-laps-bottom.png), [i3rs UI](../../target/i2-comparison/i3rs-driver.png), [CLI laps](../../target/i2-comparison/laps.json).

### 2. P1 — Invalid samples become plausible numbers

i2 leaves the lambda channels blank at approximately 9.698 seconds. i3rs supplies zero. More seriously, at **580.000 seconds**, i2's CSV has blank bank-lambda cells while i3rs returns:

| Channel | i2 | i3rs |
|---|---|---:|
| Exhaust Lambda Bank 1 | missing | 1.102 |
| Exhaust Lambda Bank 2 | missing | 1.142 |
| Exhaust Lambda | missing | 1.122 |

These are stale samples, not valid measurements. `read_channel_data()` and `read_channel_range()` in [ld_parser.rs](../../crates/i3rs-core/src/ld_parser.rs) decode and scale the sample buffer without applying validity information. The finite values then propagate to graphs, statistics, exports, and math.

A candidate validity structure exists in this file: for bank 1, metadata offset `0x2fd57c + 0x74` points to a block at `0x2fd5f8`, with frequency 20, count 12, and six-byte records at `0x2fd624`. Interpreting records as `(u16 flag, u32 sample index)` gives invalid intervals including `[0, 10.65)` and `[562.15, 600.05)`, consistent with both observed i2 gaps. This format interpretation is preliminary; validate its semantics across files before adopting it.

Evidence: [early i2 readout](../../target/i2-comparison/i2-early-values.png), [i2 CSV around 580 s](../../target/i2-comparison/i2-invalid-lambda-580.png), [i3rs values at 580 s](../../target/i2-comparison/invalid-values.json).

### 3. P2 — Formatting discards useful sensor precision

In the Fuel/Ign worksheet at cursor time **849.936 s**, both lambda gauges and the readout display **`1 LA`**. The decoded values are **0.894720 and 0.899640**. i2 displays lambda to three decimal places; for example, its readout at 178.136894 s shows 1.013 and 1.020.

[cursor_readout.rs](../../crates/i3rs-app/src/panels/cursor_readout.rs) line 45 and the gauge formatting paths in [gauge.rs](../../crates/i3rs-app/src/panels/gauge.rs) use the channel's scaling `dec_places` field as display precision. That field is zero for these float channels. Display precision needs a separate definition. The same issue rounds other useful fractional values, including throttle, pressure, and ignition timing.

Evidence: [i3rs Fuel/Ign](../../target/i2-comparison/i3rs-fuel.png), [decoded values](../../target/i2-comparison/display-values.json), [i2 fractional readouts](../../target/i2-comparison/i2-mid-values.png).

### 4. P2 — Long channel names are truncated or consume their unit

i2 exports `Fuel Closed Loop Control Bank 1 Trim`; i3rs exposes `Fuel Closed Loop Control Bank 1 `, losing `Trim` from the name. The channel browser also shows malformed names such as:

- `CP Lotus ESP Lateral AcceleratioG`, with an empty unit.
- `CP Lotus Charge Cooler Input AirC`, with an empty unit.
- `CP Lotus Fuel Tank Level Calculal`, with an empty unit.

The 32-byte name parsing and overflow heuristic at [ld_parser.rs](../../crates/i3rs-core/src/ld_parser.rs) lines 532–555 append short unit strings to a full name buffer. Full names need to come from the appropriate extended metadata rather than this heuristic. This affects channel identification, units, searches, and expressions using the i2 channel names.

Evidence: [i2 exported full names](../../target/i2-comparison/i2-export-header.png), [i3rs channel metadata](../../target/i2-comparison/channels.json), [browser examples](../../target/i2-comparison/i3rs-fuel.png).

### 5. P2 — Session duration includes padding beyond i2's endpoint

i2's entire-outing export ends at **3918.210 s**. i3rs reports **3919.000 s**, 0.790 s longer. Its lap detector uses yet another endpoint, **3918.400 s**.

`ChannelMeta::duration_secs()` uses sample count divided by frequency, and `LdFile::duration_secs()` takes the longest channel. Lap detection independently uses its timer channel's sample count. These estimates need a common authoritative session endpoint; otherwise full-session ranges and the final lap disagree.

Evidence: [i2 export header](../../target/i2-comparison/i2-export-header.png), [i3rs summary](../../target/i2-comparison/summary.json), [i3rs laps](../../target/i2-comparison/laps.json).

## Checks that passed

Release build and the existing `cargo test` suite passed; one fixture-dependent integration test was ignored. The build used `DEVELOPER_DIR=/Library/Developer/CommandLineTools` because the selected Xcode installation required license acceptance.

The file opens in the native GUI and CLI, and i3rs decodes 208 channels. Session date, time, vehicle, venue, and comment agree with i2.

At 178.136894 seconds, all 14 checked numeric readings agree with i2 after unit conversion and rounding. Representative examples:

| Channel | i3rs decoded | i2 displayed |
|---|---:|---:|
| Engine Speed | 2366.400764 rpm | 2366.4 rpm |
| Throttle Position | 3.067948 % | 3.1 % |
| Inlet Manifold Pressure | 35.138728 kPa | 5.1 psi |
| Fuel Pressure | 493.980943 kPa | 71.6 psi |
| Injector duty cycle | 4.599787 % | 4.6 % |
| Lambda bank 1 | 1.012786 | 1.013 |
| Lambda bank 2 | 1.020262 | 1.020 |
| Ignition timing | 36.139800 degrees | 36.1 degrees |

Exhaust lambda combined, exhaust camshaft aim, both exhaust camshaft positions, and both inlet camshaft positions also matched. An earlier cursor check near 9.698 seconds matched 11 numeric readings, with the missing lambda values accounted for separately above. Different pressure units alone are not a decoding failure.

## Reproduction and scope

```bash
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo build --release
DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo test
target/release/i3rs summary 'S1_#28299_20260508_174455_02.ld' --format json
target/release/i3rs laps 'S1_#28299_20260508_174455_02.ld' --format json
target/release/i3rs extract 'S1_#28299_20260508_174455_02.ld' \
  --channel 'Exhaust Lambda Bank 1' --channel 'Exhaust Lambda Bank 2' \
  --time 580:581 --resample 1 --format json
```

The GUI comparison covered the Driver, Engine, and Fuel/Ign views, cursor readings, channel metadata, and lap lists. It does not establish parity for every channel, every sample, or every analysis panel. The initial comparison made no production code changes; the subsequent fixes are described above.

i2's 1 Hz entire-outing CSV was saved in the guest's `Documents\MoTeC\Logged Data\EVORA_Friss\2026\May\i3rs-reference-1hz.csv`. Clipboard transfer to macOS did not succeed, so the export checks used visible headers and rows rather than a completed all-channel automated diff. Local screenshots and CLI JSON are in the ignored `target/i2-comparison/` directory; they are working evidence and will disappear if that directory is cleaned.

Reference beacon times transcribed from i2's export (seconds):

```text
351.680 814.515 814.574 948.688 1076.961 1203.760 1331.686
1460.738 1619.022 2482.244 2623.089 2753.985 2881.716
3012.059 3140.984 3272.243 3402.929 3532.347 3668.284
```

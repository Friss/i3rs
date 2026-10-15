# I3R-2 channels, maths and diagnostics — 2026-10-14

Implementation for [I3R-2](https://linear.app/exotherm/issue/I3R-2/w02-i2-parity-channels-maths-and-diagnostics), covering the W02 implementation slices for C04, C09, G10, G11, G12, G19, D03, D05 and D08. The ticket remains In Progress for the matched reference acceptance described below. See [channel-analysis.md](../channel-analysis.md) for user workflows and numerical contracts.

## Implemented behavior

- Physical, derived and unavailable references share deterministic resolution. Ordered aliases try available targets in order; source enable/priority and recording scope determine eligible definitions. Qualified references pin the source. Channel mappings provide names, unit metadata, precision, interpolation, comments, enums and separate display transforms.
- Maths properties carry source, recording scope, quantity, result unit, default display unit, decimals, output rate, interpolation, comments and enums. Workspace, project and maths JSON retain these settings with backwards-compatible defaults. Saved maths choices include source qualification; plotted maths uses stable runtime IDs through list edits.
- Setup constants retain value, unit, comment and recording content identity, so identical recordings moved or copied retain setup values and different recordings cannot reuse them accidentally. File identity is computed in the load worker; logged samples remain lazily decoded.
- Derived result keys include the recording, full definitions and resolution/evaluation settings. Changes clear dependent results; stale background responses are rejected. Resolution cycles, disabled/scoped sources, incompatible units and failed decodes have visible diagnostics. Failed decodes wait for an explicit retry.
- Explicit input units convert through SI while source/result buffers retain their mathematical values. Linear, previous-sample and nearest policies support mixed-rate evaluation and fixed output grids. State/enum inputs use sample hold by default.
- Sample/time smoothing supports trailing and centered windows. Low-pass uses a documented one-pole kernel. Missing data splits finite segments and resets filters. Legacy unqualified `smooth(channel,N)` keeps sample semantics; MoTeC-style unit-qualified `smooth` uses seconds.
- `component_gate_includes` and `component_gate(data,gate)` use finite positive inclusion. Existing general-purpose `gate` and `if_then` nonzero semantics remain intact.
- The editor includes source/setup/mapping, alias, filter and scale tools. Values offers searchable active-component/all-channel views with lazy visible-row loading, cursor time/distance and state labels. Channel Status / Messages exposes resolution, maths and decode failures plus file/save/export errors. Continuous logged channels support zero at cursor for the current recording.

## Oversteer reference and unit path

The user supplied this complete expression:

```text
smooth(choose('Corr Speed'[km/h] < 50, 0, sgn('G Force Lat'[m/s/s]) * (('Vehicle Wheelbase'[m] * 'G Force Lat'[m/s/s] / sqr('Corr Speed'[m/s])) - sgn(stat_mean('Steered Angle'[rad] * 'G Force Lat'[m/s/s])) * 'Steered Angle'[rad])), 0.2)
```

The supplied [properties screenshot](evidence/i3r-2/oversteer-properties.png) confirms Angle, radian result, degree display, two decimals, automatic rate and interpolation enabled. The Oversteer template uses these properties and the exact supplied expression. `choose`, `sgn`, `sqr` and `stat_mean` are supported.

The same screenshot reports **Unknown channel or constant 'Corr Speed'**. A valid corrected-speed source/alias, actual lateral-acceleration and steered-angle units, and measured Vehicle Wheelbase setup constant must be provided to evaluate it on a recording. Raw GPS speed is not substituted automatically. Synthetic tests validate the supplied algebra, 50 km/h gate, sign convention, unit conversions and 0.2-second i3rs smoothing. A separate end-to-end regression checks radian results remain raw for dependent maths while Values/CSV uses degrees with two decimals; saved default units and explicit raw panel overrides survive reload.

## Validation

All commands used `DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 218 passed; 11 reference/performance tests ignored |
| `cargo test --workspace -- --ignored --skip native_sampling_render_benchmark` | 10 reference regressions passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo build --release` | Passed |
| `cargo check -p i3rs-app --target wasm32-unknown-unknown` | Passed |
| `cargo publish -p i3rs-core --dry-run --allow-dirty` | Packaging/verification passed; no upload |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

Focused tests cover ordered/qualified aliases and source precedence, recording identity/scoped constants, stale-response rejection, cycles and failure retry, mixed-rate boundary sampling, unit conversions, filter/gate signed/missing cases, metadata persistence, stable plotted identity, display/raw separation and CSV. Headless egui checks render the editor, Values and diagnostics and verify all-channel Values only requests visible sources. Existing parser, native-sample, section-timing, CLI and local May 8 recording regressions pass.

## Acceptance still required

A fresh matched i2 Standard/Pro GUI and exported-output comparison was not completed. Desktop automation previously lost access to the i3rs window and the i2 VM was locked; this implementation uses numerical/persistence tests and headless UI checks. The supplied user-defined expression is complete but still unresolved in the reference screenshot. **C04 remains open** until valid reference inputs and matched output establish exact-definition behavior, including the i2 smoothing kernel. The documented i3rs smoothing/low-pass contracts do not establish exact i2 kernel parity.

W04 must integrate recording identity/constants with cross-recording overlay workflows. W05 must preserve unavailable component slots. W06 must bind the positive gate predicate to component selection and denominators, completing D03's component workflow. These downstream workflows and applicable live Pro checks must be included before closing their audit findings; source categories here do not import native MoTeC maths libraries.

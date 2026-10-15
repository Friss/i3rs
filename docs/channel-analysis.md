# Channels, maths and diagnostics

Open **View → Math Editor** to manage calculations, sources, recording setup values, aliases and mappings. Open **View → Channel Status / Messages** for resolution failures and file/output errors. The Values sidebar switches between the active component and a searchable all-channel view; visible rows load their data on demand.

## Sources and references

Sources use configurable priorities (lower numbers win): Temporary, Log File, User, MoTeC and Logged. A source can be enabled or disabled. These categories organize i3rs definitions; importing native MoTeC maths libraries is a separate format integration.

An alias contains an ordered list of channel names, one per line. Resolution selects the first name present in an enabled source for the current recording, then applies source priority. Exact spelling wins within a source before normalized spelling. Spaces, underscores, dots and case are normalized consistently. An unresolved expression retains its requested names and reports the attempted targets.

Use a qualified reference such as `'Logged::Engine Speed'` to pin an actual source/channel. Saved maths selections retain this qualification, so a higher-priority definition with the same name does not replace a selected User or Log File channel. Maths graphs and gauges retain a definition's runtime identity through list reordering and deletion. Full persistence of unavailable component slots belongs to W05.

**Recording setup constants** store a name, finite value, unit and comment with the recording's content identity. Identical copies of a recording share that identity after a rename or move. A Log File calculation is associated with its recording; the recording-only checkbox also applies to other sources. Definitions and setup values remain stored when a different recording is opened, and out-of-scope definitions show an unavailable-source status.

## Result and display units

Set a maths result's quantity, result unit, default display unit, decimals, output rate, interpolation and comment in its properties. A default display unit must be compatible with the result unit and persists with the calculation. Individual panel display choices override this default. The result unit describes the numbers produced by the expression. Use explicit input units to normalize recordings with different source units:

```text
'Velocity'[m/s]
'Logged::Fuel Pressure'[psi]
'G Force Lat'[m/s/s]
'Steered Angle'[rad]
```

Supported conversions include speed (km/h, mph, m/s), pressure (Pa, kPa, bar, psi), temperature (C, F, K), angle (deg, rad), length (m, mm, ft), and G acceleration (9.80665 m/s²). Incompatible or unknown conversions report an error. Quantities/result units are declared metadata, rather than inferred dimensional checking of arbitrary arithmetic.

Graph/readout display choices apply scale, offset and display units to the output. CSV uses the same display transformation and precision. Maths evaluates source values with its explicit unit annotations. Channel mappings can rename a channel, override source unit metadata, set precision/interpolation and attach comments/enums; their separate display scale/offset can apply to the workspace or the current recording. **Zero at cursor for this recording** is available on a continuous physical channel's display menu.

A fixed output rate resamples referenced inputs onto that grid. The default rate is the highest input rate. Continuous inputs use linear interpolation by default; logged state/enum inputs use previous-sample interpolation. Explicit Previous or Nearest calculation policies take precedence. Missing samples remain missing. Constant-only calculations span the recording duration.

## Filters and gates

The filter and scale/offset tools populate a new calculation for review and editing. Filters evaluate on the calculation's output grid:

| Expression | Meaning |
| --- | --- |
| `smooth_samples(channel, N)` | Trailing sample-window average |
| `smooth_samples(channel, N, 1)` | Centered sample-window average |
| `smooth_time(channel, seconds)` | Trailing time-window average, rounded up to samples |
| `smooth_time(channel, seconds, 1)` | Centered time-window average |
| `lowpass(channel, cutoff_hz)` | One-pole low-pass; cutoff at or below the output grid's Nyquist frequency |

Smoothing clips windows at contiguous finite-segment boundaries, preserving missing samples and restarting after gaps. Even centered windows put the extra sample after the cursor. Low-pass uses `alpha = 1 - exp(-2π·cutoff/rate)` and restarts at the next valid sample after a gap. These kernel choices are explicit i3rs contracts pending matched i2 filter verification.

Legacy unqualified `smooth(channel, N)` retains sample-window semantics. For `smooth` inputs containing MoTeC-style unit-qualified references, its window is in seconds. Prefer `smooth_samples` or `smooth_time` when authoring new calculations.

`component_gate(data, gate)` includes finite positive gate values and produces missing samples for the remainder. Existing `gate` and `if_then` expression conditionals retain their nonzero truth rule. W06 will connect the positive predicate to component selectors and denominator rules.

## Oversteer and enumerations

The Oversteer template now contains the complete expression supplied in the I3R-2 conversation. It uses corrected speed, lateral acceleration, steered angle, a recording setup constant named **Vehicle Wheelbase**, and `choose`, `sgn`, `sqr`, `stat_mean` and 0.2-second smoothing. It has quantity Angle, result unit radians, default display unit degrees, two decimals, automatic rate and linear continuous interpolation, matching the supplied properties screenshot. Supply the reference channels with their actual input units and measured wheelbase. The screenshot reports **Unknown channel or constant 'Corr Speed'**. The template therefore requires a valid corrected-speed source or an explicit alias supplied by the user; it does not silently substitute raw GPS speed. The algebra/unit conversions and radians-to-degrees Values/CSV path have numerical regressions; matched i2 output/filter validation remains separate.

Enumerations use signed integer `value=label` lines. They display with state sample-hold behavior and persist with source, scope, comments and calculation settings in workspace/project and maths JSON. The existing File menu maths import/export carries enumeration metadata as well as expressions. Legacy JSON without the new fields loads with User source, linear continuous interpolation, automatic output rate and empty metadata.

Maths result keys include recording identity, full definitions, aliases, source/mapping/constant settings and evaluation configuration. Definition/settings edits invalidate dependent results; stale background results are discarded. Cycles, unresolved inputs and decode/unit failures appear in Channel Status / Messages. Failed physical decodes stay failed until **Retry failed channels**, avoiding repeated background decode attempts.

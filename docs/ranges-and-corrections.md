# Ranges, corrections and track annotations

Open **View → Ranges / Corrections / Details** with a recording loaded. The editor keeps a draft; **Apply edits** validates and installs it. Save the workspace/project or an analysis sidecar to retain applied edits across app restarts. **Reload draft** discards unapplied changes. Applied edits follow the recording's content identity when it is renamed or copied, while different recordings keep separate edits.

## Ranges and markers

Ranges have a stable identity, editable name, `[start,end)` times, type, trust and comment. Lap, Run and Stage are competition types. Out Lap, In Lap and Transport remain distinct even after renaming. Runs optionally store a reaction time; time-slip/milestone reports belong to W08. Unknown trust is the default for generated ranges; the editor exposes Trusted and Untrusted explicitly. The browser shows type/trust, the timeline dims non-competition ranges and marks untrusted intervals, and Values retains selection type/trust while zoom changes.

**Read recorded beacons** reads tagged beacon events without rounding their millisecond timestamps. **Replace ranges from markers** creates circuit out/timed/in ranges or stage/run ranges with surrounding transports. Explicit Stage Start/End markers distinguish the transport between stages. An explicit Missed Beacon marker flags the containing range as untrusted without inventing a time crossing. Imported/draft ranges must have unique identities and finite, non-overlapping bounds inside the recording.

GPS generation uses two geographic gate endpoints and a minimum crossing interval. Crossings run from the negative to positive side of the directed gate; reversing endpoints reverses direction. Crossings interpolate between adjacent finite fixes, stay within the gate segment and never bridge invalid fixes. The gate settings and generated markers persist with the recording. Review markers and apply the resulting ranges before selecting them for analysis.

## Speed, distance and GPS

The default corrected-speed source preserves the existing policy: logged Corr Speed, fastest valid front-wheel pair, then ordered available wheel/vehicle/GPS fallbacks. Source fields use logged channel names with the parser's normalized matching. Corrected source values convert to m/s before combining different units. Disabling the pair or editing fallback order changes that choice predictably. Lower/upper speed limits each choose Zero, Invalidate or Clamp.

Optional wheel-lock correction predicts speed from the preceding corrected speed plus the longitudinal acceleration over one output sample. During braking, a wheel speed below that prediction by more than the configured threshold is replaced by the nonnegative prediction. Acceleration uses m/s² with a configurable lag advance; missing speed/acceleration resets the prediction. This is an explicit i3rs numerical policy pending a matched i2 kernel comparison.

Distance can integrate corrected speed or use an explicit recorded distance channel. Recorded lap-distance resets unwrap into a cumulative series; missing intervals hold cumulative distance. Integration retains the validated trapezoidal calculation and holds distance through missing speed intervals. Existing automatic graph distance selection still prefers usable monotonic recorded/derived distance before integrating speed. Track-map distance defaults to corrected-speed integration, preserving the previous validated behavior.

Distance sections retain their model length. By default, laps within 5% of that length use the existing section-boundary stretch calculation. Disabling stretch or changing tolerance affects section timing. An explicit track length stretches supplied competition ranges while keeping cumulative continuity through the rest of the recording; outlaps and transports are excluded by their type. The settings persist per recording. Comparisons with other recordings remain a W04 integration.

GPS controls select latitude/longitude sources, interpolation policy, duplicate-fix interpolation, time shift and validity. Blank coordinate names retain automatic GPS-channel detection. Paired `(0,0)`, non-finite or out-of-bounds fixes remain gaps. A quality threshold uses previous-sample values and optionally invalidates positions. Independent corrected GPS speed can be invalidated by quality or zeroed below a threshold even when wheel speeds supply Corr Speed.

Positive manual GPS time shift advances delayed data: output at `t` samples the input at `t + shift`. Automatic shift is the chosen fraction of the median finite fix-change interval. Duplicate interpolation fills repeated finite fixes up to the next changed fix and never crosses an outage. Position and GPS speed use the same shift. These choices have focused tests; exact equivalence to i2 automatic shift and wheel-lock kernels still requires live reference measurements.

The generated **Corr Speed**, **Corr Dist** and **Corr GPS Speed** outputs are available through the MoTeC source category in maths and all-channel Values. Pin them with `'MoTeC::Corr Speed'[km/h]`, for example. Source errors appear in Channel Status / Messages. Correction jobs carry recording/settings identities; stale results cannot replace current outputs.

## Track sections and annotations

Track Report's section editor exposes stable identities, names, Corner/Straight/Other classification, comma-separated categories and boundaries. Distance-defined sections retain integer-millimetre boundaries; geographic sections retain recording-bound GPS indices and export geographic coordinates. The bundled VIR model includes explicit classifications and identities for its existing 33 sections. Legacy definitions load as Other until classified; renaming a section does not change its kind or identity.

**Annotation properties** independently controls section names, corner minimum speeds, straight maximum speeds and gear changes, with speed units and precision. Extrema use the section's actual interval, including sections crossing the beacon, and are placed at the event time's interpolated GPS position. Gear annotations show transitions at native gear sample times, excluding invalid transitions. An event whose GPS position is missing is omitted. Labels no longer substitute a section-start min/max or gear-range summary for these events.

## Persistence and raw logs

Workspace/project files include version-1 recording analysis records keyed by content identity. A standalone sidecar has the same record schema; native file opening discovers `recording.i3rs.json` next to `recording.ld`. Invalid, mismatched or unsupported sidecars produce a diagnostic and leave raw log data available. Native save/import dialogs and JSON copy/paste support standalone sidecars; browser uploads use explicit JSON import because neighboring files are unavailable.

Track exports use version 2 with section semantics; version-1 definitions remain readable. Both recording-bound indices and geographic/distance definitions are retained where available. Driver, venue, vehicle, event, notes and custom details are editable analysis metadata. The `.ld` bytes and parsed source metadata are unchanged. Raw MoTeC log rewriting, database indexing and downstream export/overlay workflows remain separate workstreams.

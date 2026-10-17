# Workspace profiles

Choose **File → Workspace Profile** to build Circuit, Drag, Engine or Rally worksheets. Circuit and Engine are the **configured references** observed in the October 2026 audit, not universal factory presets. Choosing a profile replaces the current worksheet layout; save a customized workspace/project before selecting another profile. The loaded recording, selected interval, cursor, zoom, user maths, aliases, preferences and recording analysis edits remain available. This deliberately retains the file when switching profiles; the observed i2 Standard workspace switch clears its selection.

A profile may be selected before opening a recording. Missing channels remain named slots in graphs, gauges, distributions and reports. Aliases use ordered, explicit names through the shared channel resolver, including Engine RPM/Engine Speed and the sample logs' abbreviated temperature, pressure and lambda names. A missing specialty channel never removes a worksheet. Save Workspace/Project retains profile identity, dock splits, channel slots, gauge styles, report policies and maths; older workspace files without profile metadata still load.

| Profile | Worksheets |
| --- | --- |
| Circuit — configured reference (11) | Driver; Braking; Engine; Fuel / Ign; Mixture Map; Oil Pressure; RPM Histo; Track Report; Channel Report; Section Times; Spare |
| Drag (8) | EGTs; Drive Line; Fuel; Engine Health; Acceleration; Boost; Reports; RPM Histogram |
| Engine — configured reference (5) | General; Cam Control; Knock; Fuel Usage; Wheel Speeds |
| Rally (9) | Driver; Engine; Fuel / Ign; Mixture Map; Boost; RPM Histogram; Diff Controller; Channel Report; Spare |

All 33 worksheets contain configured components and requested channel slots. The compositions include docked graphs, distributions, reports, embedded gauges and split report charts. Full slot names and tile membership are declared in `crates/i3rs-app/src/default_layouts.rs` and its `profiles.rs` module. No channel availability filter controls whether these sheets exist.

Circuit Driver includes Oversteer, speed-weighted Oversteer and ambient/inlet air temperatures. Oversteer reuses the existing validated template and requires actual Steered Angle, lateral acceleration and a measured Vehicle Wheelbase setup constant. Missing inputs remain diagnostic errors; steering-wheel angle is not silently substituted for road-wheel angle. The captured speed-weighted definition is `Corr Speed [km/h] / 80 * Oversteer`; the stored result remains radians and the default display is degrees. Braking uses left-minus-right front/rear hub RPM maths with explicit RPM inputs. Fuel/Ign has separate lambda bars and digital gauges for both banks, with 0.7–1.1 lambda bar scales.

Engine has the observed cam targets/positions/control, six-cylinder knock/trim groups, fuel-level/usage, wheel hub speeds, boost and cooling-fan channels. Drag has eight EGT channels and gauges, drivetrain RPMs, clutch slip, fuel/health groups, colored boost scatter, and a full-outing staging overview beside the linked Acceleration graph. Rally includes its steered angle/G-force/brake groups, dual-bank mixture, throttle-colored boost scatter and differential speed/slip/lock/control/current/mode slots.

## Run, stage and transport setup

Opening a Drag `.ld` with its `.ldx` imports SOR run markers, the official time slip, reaction time and measured gear ratios. Rally STGSTRT/STGEND pairs import Stage intervals with Transport gaps. A saved `.i3rs.json` sidecar takes precedence, followed by saved workspace analysis edits. Marker trust stays Unknown because undocumented flag bits are not treated as trust evidence. Ordinary Circuit lap detection is unchanged.

Profile selection preserves recording range types. It does not infer a drag launch or a rally stage from a circuit lap number. Use **View → Ranges / Corrections / Details** to create/edit **Run**, **Stage** and **Transport** intervals using the existing recording editor. Select the interval in the channel browser or Main comparison controls. Rally's Channel Report includes Stage ranges; transports remain separately selectable for graphs and other analysis.

Open **View → Drag setup / Run time slips…**, or **Run details / time slip…** in a milestone report:

1. Configure actual run start/end boundaries and reaction time in the range editor.
2. In **Drag: transmission and clutch slip**, select engine RPM, drive-shaft RPM and Gear references. Add the measured forward transmission ratios. No vehicle ratios are assumed. Apply edits to generate Input Shaft Speed and/or Clutch Slip through the shared expression engine.
3. In **Drag: run time-slip details**, optionally enter cumulative elapsed seconds at 60, 330, 660, 1000 and 1320 ft. Times are measured from launch and exclude reaction time. Apply edits, then save the workspace/project or recording sidecar.

Input shaft RPM = drive-shaft RPM × the selected forward gear ratio. Clutch slip (%) = 100 × (1 − input-shaft RPM / engine RPM). Neutral, missing/unconfigured gears and nonpositive engine speed produce missing results. Generated definitions belong to that recording, preserve their identities when ratios change, and participate in the ordinary graph/readout/report/export/overlay resolution. Generation ownership is stored separately from editable comments.

Drag Reports reuse the shared report engine. Profile charts show Min/Max/Avg together; Rally respects each channel’s Min-only, Min/Max/Avg or Max-only selection. Chart selection, tables and CSV share the same result values. Legacy single-statistic chart settings remain compatible. The upper report gives native channel statistics per distance interval with a split chart; the lower gives interval durations and a split time chart. **Cumulative from launch** switches to cumulative intervals. Main/Overlay deltas compare the corresponding distance milestone. Report CSV uses the same displayed result population and units.

Official elapsed times take precedence. Missing milestones use linearly interpolated crossings of corrected distance relative to the run start. Missing/nonmonotonic distance stops crossing calculation; crossings never extend outside the run. Computed crossings inconsistent with entered times are omitted. A split with a missing endpoint stays blank. Reaction time is displayed separately and never added to elapsed times. Trust filtering, selected comparisons, missing-channel diagnostics and boundary interpolation use the common report settings.

## Sample clocks

Channel rates are physical samples per second (`f64`). The legacy ADL clock value 1,024,000 at LD header +0x42 scales nominal rates by 1.024 (100 → 102.4 Hz; 20 → 20.48 Hz). Modern 1,000,000-clock files retain integer rates. Samples, validity masks and sample counts stay unchanged; graphs, corrections, maths, distributions, reports and exports use the native physical grid. Existing JSON files with integer output rates remain readable. Saved analyses on the old nominal clock migrate in memory when both recording identity and nominal duration match. Range/marker timestamps and sample-alignment time offsets move to the physical clock; details, selections, spatial settings and official reaction/time-slip seconds are retained. Saving records the migrated duration, preventing a second conversion on reload.

Matched reference results and GUI evidence are recorded in [the W08 validation record](testing/i3r-8-workspace-profiles-2026-10-16.md).

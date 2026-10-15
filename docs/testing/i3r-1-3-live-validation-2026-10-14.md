# I3R-1–3 consolidated reference validation — 2026-10-14

Branch: `friss/i3r-1-3-i2-parity`. [Draft PR #22](https://github.com/Friss/i3rs/pull/22) contains all three tickets. Linux tests/lint, macOS/Linux/Windows builds, web build, performance reporting and Semgrep passed for the initial consolidated implementation.

Latest result: after the user's desktop approval, the candidate native GUI responded and the full Lap 3 histogram matched all ten observed i2 throttle-band populations. The eleven checked speed labels also appeared with matching values in the candidate Track Report. Project restore, a range trust edit and reload, shifted file shortcuts and readable gear labels passed the GUI smoke check. Full annotation/correction/Oversteer and Pro acceptance remain open; details of this pass are recorded below.

## Fresh VMware observations

Reference: MoTeC i2 **Standard 1.1.4.457**, Circuit workspace, Windows 11 Arm in VMware Fusion. This is not live Pro acceptance.

Switching Fusion from full screen to Single Window restored enough guest input to open the log picker. Selected the May 8 afternoon VIR recording (17:44:55, 58.6 Mb, device 28299) and confirmed the loaded filename `S1_#28299_20260508_174455_02.ld` in i2's recording banner.

| Observation | Candidate comparison | Result |
| --- | --- | --- |
| Selected Lap 2 displays `0:00.059` | Recorded boundaries 814.515–814.574 s | Match |
| RPM Histo displays one sample, 100%, in the 4900–5000 rpm bin and the 90–100% throttle band | Native extraction yields one sample at 814.55 s: 4911.42334 rpm, 99.99623% throttle. The app's sampling/binning functions produce the same bin, band and percentage | Match; local reference regression added |
| RPM Histo axis spans 0–9000 rpm; legend has ten throttle bands plus under/overflow | I3R-1 Circuit configuration | Observed configuration consistent; properties dialog not rechecked |

The local regression `matches_live_i2_lap_histograms` uses the actual recording and the app's histogram sampling/binning path. It is ignored in public CI because the private recording is not distributed. Run it with `cargo test -p i3rs-app matches_live_i2_lap_histograms -- --ignored`.

Guest mouse/keyboard routing remains intermittent: worksheet selection worked, but attempts to select Out Lap/open the Data menu had no reliable effect or opened context/Start menus. The fresh candidate native bundle launch previously timed out. These observations therefore validate reference behavior against the candidate numerical path, not a fresh side-by-side native GUI pass. No raw log, i2 corrections or math definitions were edited.

## Next matched checks

- Export the same interval/channels from i2 and compare displayed units, precision and invalid values with i3rs CSV.
- Select an ordinary timed lap and compare all RPM/throttle bins and counts.
- Compare track speed extrema and native gear events at actual positions on the same selected lap.
- Capture corrected speed/distance/GPS samples under identical source and correction settings; wheel-lock and automatic GPS shift kernels remain unverified.
- Validate Oversteer with resolvable inputs and the actual vehicle wheelbase; retain result radians/display degrees and compare samples.
- Exercise range edits, trust, markers and sidecar reload in the candidate GUI. Keep applicable Pro acceptance separate from the Standard session.

## Review check and retry later on October 14

PR #22 had no review comments, conversation comments or submitted reviews. All checks passed on `7a42bec`. The retry successfully selected Lap 3 with i2's next-lap toolbar button, read Histogram Properties and opened Track Report.

Lap 3 spans 814.574–948.688 s and displays `2:14.114`, with 2,682 histogram samples. i2's ten throttle-band totals, from low to high, are **1037, 204, 94, 59, 36, 29, 16, 15, 23, 1169**; both outlier populations are zero. Before the fix, i3rs's top band held 603 samples and overflow held 566, all decoded as exactly 100%. Including the outer upper endpoint in the final band reproduces every observed total. Internal boundaries and genuinely above-range values retain their existing behavior. The expanded local reference test passes. The i2 table still labels its top interval `90 ≤ v < 100` and overflow `100 ≤ v`; the observed populations, rather than those labels alone, justify this candidate endpoint policy. A controlled constant-input probe would establish i2's internal arithmetic more precisely.

Eleven visible Track Report speed labels match i3rs at one decimal place:

| Section | Event | mph |
| --- | --- | --- |
| Str 0-1 | Maximum | 133.6 |
| Str 1-2 | Maximum | 91.7 |
| Str 3-4 | Maximum | 80.0 |
| Str 5-6 | Maximum | 125.0 |
| Turn 7 | Minimum | 106.0 |
| Turn 8 | Minimum | 98.9 |
| Str 10-11 | Maximum | 104.2 |
| Str 12-13 | Maximum | 141.9 |
| Str 13-14 | Maximum | 64.9 |
| Turn 15 | Minimum | 65.1 |
| Str 16-17 | Maximum | 103.4 |

`matches_visible_live_i2_lap3_track_speed_labels` preserves these comparisons as a local core regression and passes. The live view also has a 101.8 mph maximum near the upper track sections that is not accepted by this test; its section association/boundaries need closer comparison. Candidate Str 2-3 currently yields 67.3 mph. Remaining labels, event positions, annotation suppression and gear transitions are still open acceptance items.

The user explicitly approved launching `/tmp/i3rs-I3R3.app` after automatic review required confirmation for that local build. Launch timed out; the app then appeared running in the inventory, but binding to it by bundle ID also timed out. Thus the candidate native GUI comparison remains unavailable; the recorded matches use its numerical paths. No i2 correction or math settings were changed.

After the endpoint fix, 235 workspace tests and all 12 local reference regressions pass. Workspace Clippy with warnings denied, release build, formatting and diff checks also pass. Raw reference logs remain local.

The three ticket reports retain their explicit acceptance gaps. These partial live comparisons do not close the tickets.

## Native GUI pass after desktop approval

The approved `/tmp/i3rs-I3R3.app` became controllable. The first running process still used the pre-endpoint-fix binary, so the validation project was saved to `/tmp/i3r-gui-validation-20261014.i3rsproj` before restarting. Loading that project through File → Load Project restored Lap 3 (`2:14.114`), the recording and RPM Histo worksheet.

The rebuilt native histogram visibly showed **2,682 samples**, 90 bins, locked 0–9000 rpm, percent mode, ten non-centered 0–100% throttle bands, totals **1037, 204, 94, 59, 36, 29, 16, 15, 23, 1169**, and zero underflow/overflow/missing samples. This confirms the population comparison through the GUI, beyond the earlier numerical regression. Individual RPM-bin counts still need an exported reference comparison.

Applying **VIR Full sections** produced 33 named distance sections and displayed all eleven previously checked speed values. The fresh i2 view still showed the 101.8 mph maximum near the upper sections. i3rs showed a 67.3 mph maximum for Str 2-3 and more extrema labels than i2; section association, boundary treatment and suppression remain unresolved. The six native gear changes rendered, but their exact i2 times/positions remain unverified. The gear transition arrow rendered as a missing-glyph box in this build; labels now use ASCII `->`.

The GUI Oversteer template populated the exact supplied expression, Angle quantity, `rad` result, `deg` display, two decimals, automatic highest-input rate and linear interpolation. A recording-scoped definition produced the explicit diagnostic `no enabled source for 'G Force Lat' (tried G Force Lat)`. No wheelbase or input aliases were invented, so this is a metadata/diagnostic smoke check, not a numerical Oversteer comparison.

In the range editor, Lap 2 trust was changed from Unknown to Untrusted and applied. The browser updated that label while Lap 3 selection remained intact. Saving the temporary project preserved the typed range and bounds, all 33 sections and scoped Oversteer definition. The legacy **Fastest** display still reads the `.ldx` fastest-time string (`0:00.059`), independent of edited trust; this pass does not claim downstream trust-based ranking/exclusion. The raw recording and i2 settings were not edited.

The GUI also exposed `Cmd+Shift+O` opening the `.ld` picker: egui's unshifted shortcut consumed the event before Load Project. Shifted Open/Save shortcuts now run first, as required by egui's modifier matching. Existing app tests (80 passed, 6 ignored), Clippy with warnings denied, release build, formatting and diff checks pass for these two UI fixes. All remote checks passed on `f8ceab2`; there were still no PR review/conversation/inline comments at the latest check.

After saving and verifying the temporary project on disk, another restart was attempted to check the shortcut and glyph fixes live. App launch/binding timed out again although the process appeared running. Those two fixes have compiled/unit validation but their final GUI recheck, and restore of the newly edited trust state, remain pending.

## Successful reconnect and final GUI recheck

After the user reported the desktop issue fixed, the compiled `8070961` app reconnected successfully. **Cmd+Shift+O** opened the project picker and loaded the saved `.i3rsproj` directly, confirming that Open File no longer intercepts it. The browser restored **Lap 2 · Untrusted**, retained Lap 3 selection, and Track Report restored the saved sections and annotations. All six gear-change labels visibly use readable `->` separators, including `G2 -> G3`, `G3 -> G4` and `G4 -> G3`.

**Cmd+Shift+S** opened Save Workspace with default filename `workspace.json`, confirming it no longer invokes Save Project. The save dialog was cancelled after inspection. The earlier shortcut/glyph/trust-reload recheck gaps are now closed; broader reference acceptance listed above is unchanged. At this check, native builds, lint/tests, performance reporting and Semgrep passed on `8070961`; web build was still running. No conversation or submitted review comments had appeared.

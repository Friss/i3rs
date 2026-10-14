# I3R-1–3 consolidated reference validation — 2026-10-14

Branch: `friss/i3r-1-3-i2-parity`. [Draft PR #22](https://github.com/Friss/i3rs/pull/22) contains all three tickets. Linux tests/lint, macOS/Linux/Windows builds, web build, performance reporting and Semgrep passed for the initial consolidated implementation.

## Fresh VMware observations

Reference: MoTeC i2 **Standard 1.1.4.457**, Circuit workspace, Windows 11 Arm in VMware Fusion. This is not live Pro acceptance.

Switching Fusion from full screen to Single Window restored enough guest input to open the log picker. Selected the May 8 afternoon VIR recording (17:44:55, 58.6 Mb, device 28299) and confirmed the loaded filename `S1_#28299_20260508_174455_02.ld` in i2's recording banner.

| Observation | Candidate comparison | Result |
| --- | --- | --- |
| Selected Lap 2 displays `0:00.059` | Recorded boundaries 814.515–814.574 s | Match |
| RPM Histo displays one sample, 100%, in the 4900–5000 rpm bin and the 90–100% throttle band | Native extraction yields one sample at 814.55 s: 4911.42334 rpm, 99.99623% throttle. The app's sampling/binning functions produce the same bin, band and percentage | Match; local reference regression added |
| RPM Histo axis spans 0–9000 rpm; legend has ten throttle bands plus under/overflow | I3R-1 Circuit configuration | Observed configuration consistent; properties dialog not rechecked |

The local regression `matches_live_i2_short_lap_histogram` uses the actual recording and the app's histogram sampling/binning path. It is ignored in public CI because the private recording is not distributed. Run it with `cargo test -p i3rs-app matches_live_i2_short_lap_histogram -- --ignored`.

Guest mouse/keyboard routing remains intermittent: worksheet selection worked, but attempts to select Out Lap/open the Data menu had no reliable effect or opened context/Start menus. The fresh candidate native bundle launch previously timed out. These observations therefore validate reference behavior against the candidate numerical path, not a fresh side-by-side native GUI pass. No raw log, i2 corrections or math definitions were edited.

## Next matched checks

- Export the same interval/channels from i2 and compare displayed units, precision and invalid values with i3rs CSV.
- Select an ordinary timed lap and compare all RPM/throttle bins and counts.
- Compare track speed extrema and native gear events at actual positions on the same selected lap.
- Capture corrected speed/distance/GPS samples under identical source and correction settings; wheel-lock and automatic GPS shift kernels remain unverified.
- Validate Oversteer with resolvable inputs and the actual vehicle wheelbase; retain result radians/display degrees and compare samples.
- Exercise range edits, trust, markers and sidecar reload in the candidate GUI. Keep applicable Pro acceptance separate from the Standard session.

The three ticket reports retain their explicit acceptance gaps. This initial live pass does not close the tickets.

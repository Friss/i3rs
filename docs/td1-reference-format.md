# TD1 reference-lap adapter

Observed with i2 Standard 1.1.4.457 and the [Rally Stage 1 fixture](../test_data/reference/i3r9/README.md). All numeric fields below are little-endian. Unknown versions and header variants are rejected rather than silently rewritten.

| Offset | Type | Meaning |
|---|---|---|
| 0 | u16 | Version 1 |
| 2 | u32 | Header size, 2048 |
| 6 | u32 | Paired point count |
| 10 | u64 | Time-array file offset, 2048 |
| 18 | u32 | Final elapsed time, centiseconds |
| 22 | u64 | Distance-array offset, 2048 + count × 4 |
| 30 | u32 | Final distance, decimetres |
| 40 | char[32] | Source date, NUL padded |
| 72 | char[32] | Source time, NUL padded |
| 104 | char[64] | Driver |
| 168 | char[128] | Vehicle |
| 296 | char[128] | Venue |
| 426 | u32 | Observed value 1; semantics unknown |
| 2048 | u32[count] | Elapsed time, centiseconds |
| 2048 + count × 4 | u32[count] | Distance, decimetres |

Other header bytes are zero in the observed variant. The two arrays are paired, start at zero, and have nondecreasing time and strictly increasing distance. The baseline ends at 134 s / 3267 m, matching the chosen Stage 1 time. The fixture supports the units/layout; it does not expose the original i2 implementation.

`ReferenceLap::decode` and `encode` round-trip the complete fixture byte for byte. `from_distance` independently creates a curve from the selected range's built-in corrected distance in metres: invert the native finite monotone distance with linear interpolation on a one-metre spatial grid, append the decimetre-rounded endpoint, and round times to the nearest centisecond. The final point preserves the range's complete elapsed time, including a stationary finish. At the exact recording end the last source sample is held through its own sampling interval; data is never extended beyond the recording. Missing or backwards distance, uncovered boundaries, zero travel/time, more than one million spatial points, overflow and unsupported metadata produce an error. ASCII metadata is supported; other encodings are rejected explicitly.

GUI **Device reference .td1** requires Selected range scope for Main or Overlay. User maths, plotted channel choices, display transforms and the CSV Hz setting do not affect this format. **Reference lap .ld** remains available for reusable telemetry comparisons. TD1 contains a distance/time curve, not the original logged channel arrays.

`ReferenceLap::write` uses the log writer's staged publication and recovery-backup policy. Physical device upload/acceptance has not been tested; the app does not send data to a device.

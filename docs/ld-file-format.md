# MoTeC i2 Log File (.ld) Format

Binary format documentation for MoTeC i2 `.ld` log files, as produced by MoTeC M1 ECUs
(verified against Lotus Evora T4e/T6e logs from an M1 ECU).

Format is **little-endian** throughout. Strings are **null-padded ASCII**.

## Overview

The file is structured as:

1. **File Header** (fixed, at offset 0x00) — session metadata + pointers
2. **Event/Venue/Vehicle blocks** (pointer chain from header) — extended metadata
3. **Channel metadata** (linked list starting at `chan_meta_ptr`) — one entry per channel
4. **Channel sample data** (starting around `chan_data_ptr`) — raw sample arrays

Typical file sizes range from 5–50 MB for a 15–30 minute session with 200+ channels.

---

## File Header

Starts at offset `0x00`. Total parsed size: **0x6E2 bytes** (1762 bytes).

| Offset | Size | Type     | Field            | Description |
|--------|------|----------|------------------|-------------|
| 0x00   | 4    | uint32   | ld_marker        | Magic number, always `0x40` |
| 0x04   | 4    | —        | (padding)        | Unknown, zeroed |
| 0x08   | 4    | uint32   | chan_meta_ptr     | Absolute file offset to first channel metadata entry |
| 0x0C   | 4    | uint32   | chan_data_ptr     | Absolute file offset to channel data region |
| 0x10   | 20   | —        | (unknown)        | |
| 0x24   | 4    | uint32   | event_ptr         | Absolute file offset to the Event block |
| 0x28   | 24   | —        | (unknown)        | |
| 0x40   | 2+2+2| uint16×3 | (statics)        | Unknown static values |
| 0x46   | 4    | uint32   | device_serial    | ECU serial number (e.g. `28299`) |
| 0x4A   | 8    | char[8]  | device_type      | Device type string (e.g. `"M1"`) |
| 0x52   | 2    | uint16   | device_version   | Firmware version (e.g. `100` → v1.00, `150` → v1.50) |
| 0x54   | 2    | uint16   | (unknown)        | |
| 0x56   | 4    | uint32   | num_channels     | Channel count (from header — may not match actual parsed count) |
| 0x5A   | 4    | —        | (unknown)        | |
| 0x5E   | 16   | char[16] | date             | Date string: `"DD/MM/YYYY"`, null-padded |
| 0x6E   | 16   | —        | (unknown)        | |
| 0x7E   | 16   | char[16] | time             | Time string: `"HH:MM:SS"`, null-padded |
| 0x8E   | 16   | —        | (unknown)        | |
| 0x9E   | 64   | char[64] | driver           | Driver name, null-padded |
| 0xDE   | 64   | char[64] | vehicle_id       | Vehicle identifier (e.g. `"EVORA_Friss"`), null-padded |
| 0x11E  | 64   | —        | (unknown)        | |
| 0x15E  | 64   | char[64] | venue            | Venue/track name (e.g. `"VIR"`), null-padded |
| 0x19E  | 64   | —        | (unknown)        | |
| 0x1DE  | 1024 | —        | (unknown)        | Large unknown block |
| 0x5DE  | 4    | uint32   | pro_logging      | Pro logging flag/marker |
| 0x5E2  | 66   | —        | (unknown)        | |
| 0x624  | 64   | char[64] | short_comment    | Session comment (e.g. `"VIR First Session"`), null-padded |
| 0x664  | 126  | —        | (padding)        | Remainder of header |

### Notes

- The `num_channels` field in the header often does not reflect the actual number of channels.
  The true count comes from walking the channel metadata linked list.
- The `chan_meta_ptr` and `chan_data_ptr` values vary per file. In observed logs:
  - Small log: `meta_ptr=0xFDF5`, `data_ptr=0x20F6F`
  - Larger log: `meta_ptr=0x2F8578`, `data_ptr=0x302D03`

---

## Event Block

Located at the absolute offset stored in `event_ptr`. Contains extended session metadata
and pointers to Venue and Vehicle sub-blocks.

| Offset | Size | Type     | Field       | Description |
|--------|------|----------|-------------|-------------|
| 0x00   | 64   | char[64] | event_name  | Event name, null-padded |
| 0x40   | 64   | char[64] | session     | Session name, null-padded |
| 0x80   | 1024 | char[1024]| comment    | Long comment, null-padded |
| 0x480  | 4    | uint32   | venue_ptr   | Absolute offset to Venue block |

### Venue Sub-block

Located at `venue_ptr`:

| Offset | Size | Type     | Field        | Description |
|--------|------|----------|--------------|-------------|
| 0x00   | 64   | char[64] | venue_name   | Venue/track name, null-padded |
| 0x40   | 1034 | —        | (unknown)    | |
| 0x44A  | 4    | uint32   | vehicle_ptr  | Absolute offset to Vehicle block |

### Vehicle Sub-block

Located at `vehicle_ptr`:

| Offset | Size | Type     | Field           | Description |
|--------|------|----------|-----------------|-------------|
| 0x00   | 64   | char[64] | vehicle_id      | Vehicle identifier, null-padded |
| 0x40   | 128  | —        | (unknown)       | |
| 0xC0   | 4    | uint32   | vehicle_weight  | Vehicle weight |
| 0xC4   | 32   | char[32] | vehicle_type    | Vehicle type string |
| 0xE4   | 32   | char[32] | vehicle_comment | Vehicle comment |

---

## Channel Metadata (Linked List)

Channel metadata entries form a **doubly-linked list**. The first entry is at the absolute
offset `chan_meta_ptr` from the file header. Each entry is **120 bytes**.

| Offset | Size | Type     | Field       | Description |
|--------|------|----------|-------------|-------------|
| 0x00   | 4    | uint32   | prev_addr   | Absolute offset of previous channel entry (0 for first) |
| 0x04   | 4    | uint32   | next_addr   | Absolute offset of next channel entry (0 for last/terminator) |
| 0x08   | 4    | uint32   | data_ptr    | Absolute offset to this channel's raw sample data |
| 0x0C   | 4    | uint32   | n_data      | Number of samples |
| 0x10   | 2    | uint16   | counter     | Some kind of counter/sequence number |
| 0x12   | 2    | uint16   | dtype_a     | Data type class — see Data Types below |
| 0x14   | 2    | uint16   | dtype       | Data type size/code — see Data Types below |
| 0x16   | 2    | uint16   | rec_freq    | Recording frequency in Hz |
| 0x18   | 2    | int16    | shift       | Scaling: additive shift |
| 0x1A   | 2    | int16    | mul         | Scaling: multiplier |
| 0x1C   | 2    | int16    | scale       | Scaling: divisor |
| 0x1E   | 2    | int16    | dec_places  | Scaling: decimal places (power of 10) |
| 0x20   | 32   | char[32] | name        | Channel name (e.g. `"Engine.Speed"`), null-padded |
| 0x40   | 8    | char[8]  | short_name  | Short name — often holds the unit on M1 ECUs (see below) |
| 0x48   | 12   | char[12] | unit        | Unit string (e.g. `"rpm"`, `"kPa"`), null-padded |
| 0x54   | 40   | —        | (padding)   | Zeroed padding |

### Terminator Entry

The linked list ends with a terminator entry where `next_addr = 0` and `n_data = 0`.
The terminator typically has an empty name.

### Channel Name / Short Name / Unit Quirks (M1 ECUs)

On MoTeC M1 ECUs (as used in the Lotus Evora), the three string fields behave differently
from other MoTeC loggers:

1. **The `short_name` field often contains the unit** rather than the `unit` field.
   The 12-byte `unit` field is frequently empty. When parsing, check `unit` first;
   if empty, fall back to `short_name`.

2. **Channel names longer than 32 characters are truncated.** When the 32-byte `name`
   field has no null terminator (completely filled), the name was cut off. In this case
   the `short_name` field may contain either:
   - The **continuation of the name** (e.g. name ends with `"Duty C"`, short_name is `"%"`,
     giving full name `"Duty C%"`)
   - The **unit** for a name that happens to be exactly 32 characters

   **Heuristic used:** If the name field has no null byte AND the short_name is 1–2
   characters AND the last character of the name is alphanumeric, treat short_name as
   a name continuation. Otherwise treat it as the unit.

---

## Data Types

The combination of `dtype_a` and `dtype` determines the binary format of each sample:

| dtype_a | dtype | Format    | Size | Description |
|---------|-------|-----------|------|-------------|
| 0x07    | 0x04  | float32   | 4 B  | IEEE 754 single-precision float |
| 0x07    | 0x02  | float16   | 2 B  | IEEE 754 half-precision float |
| 0x00    | 0x02  | int16     | 2 B  | Signed 16-bit integer |
| 0x03    | 0x02  | int16     | 2 B  | Signed 16-bit integer |
| 0x05    | 0x02  | int16     | 2 B  | Signed 16-bit integer |
| 0x00    | 0x04  | int32     | 4 B  | Signed 32-bit integer |
| 0x03    | 0x04  | int32     | 4 B  | Signed 32-bit integer |
| 0x05    | 0x04  | int32     | 4 B  | Signed 32-bit integer |
| 0x08    | 0x08  | float64   | 8 B  | IEEE 754 double-precision float (used for GPS lat/lon) |
| 0x06    | 0x04  | unknown   | 4 B  | Seen on some state/diagnostic channels, not fully decoded |

### Observed Data Type Usage

From actual Lotus Evora logs:

- **float32** (0x07/0x04): Most analog measurements — RPM, temperatures, pressures,
  throttle position, lambda, voltages, torque, etc.
- **int16** (0x00/0x02 or 0x03/0x02 or 0x05/0x02): State/boolean/enum channels —
  engine state, gear, brake state, diagnostic flags, etc.
- **int32** (0x03/0x04): Counters — lap number, cut counts, beacon numbers.
- **float64** (0x08/0x08): GPS latitude and longitude only.

---

## Sample Data Scaling

Raw sample values are converted to engineering units using four scaling parameters
from the channel metadata:

```
converted = ((raw_value / scale) * 10^(-dec_places) + shift) * mul
```

Where:
- `scale` — divisor (int16, typically 1)
- `dec_places` — decimal shift as power of 10 (int16, typically 0)
- `shift` — additive offset (int16, typically 0)
- `mul` — multiplier (int16, typically 1)

In practice, most channels in M1 logs have `scale=1, dec_places=0, shift=0, mul=1`,
meaning the raw float32 values are already in engineering units. The scaling parameters
are more commonly used with integer-encoded channels.

**Edge case:** If `scale = 0`, skip the division and use raw values directly.

---

## Sample Data Layout

Channel sample data is stored as a **contiguous array** of raw values at the absolute
file offset given by `data_ptr` in the channel metadata. The array contains exactly
`n_data` samples, each of size determined by the data type.

```
data_ptr --> [ sample_0 ][ sample_1 ][ sample_2 ] ... [ sample_{n-1} ]
             |<-- dtype bytes -->|
```

Samples are uniformly spaced in time at the channel's `rec_freq` (Hz). The time axis
can be reconstructed as:

```
time[i] = i / rec_freq    (seconds from start of logging)
```

Different channels may have different sample rates (1 Hz to 100 Hz observed), so their
time axes have different lengths. All channels start at t=0.

### Observed Sample Rates

| Rate   | Typical Channels |
|--------|-----------------|
| 1 Hz   | Coolant temp, ambient pressure, fuel tank level, run time |
| 2 Hz   | Idle aim, cam shaft aims, GPS diagnostic |
| 5 Hz   | Engine state, launch state, lap beacon, sport mode switch |
| 10 Hz  | Boost pressure, throttle aim, oil temp/pressure, fuel used, gear |
| 20 Hz  | Engine speed, throttle position/pedal, lambda, wheel speeds, GPS |
| 50 Hz  | Torque channels, traction control, cruise state |
| 100 Hz | Per-cylinder knock levels, camshaft actuator voltages |

---

## Session Duration

There is no explicit duration field. Estimate it from the channel with the most samples:

```
duration = max(ch.n_data / ch.rec_freq  for all channels where rec_freq > 0)
```

---

## References

- Format originally reverse-engineered by the [gotzl/ldparser](https://github.com/gotzl/ldparser) project.
- Verified and extended against real Lotus Evora S1 logs from a MoTeC M1 ECU (2025).

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
offset `chan_meta_ptr` from the file header. M1 records have a **124-byte base** and
optional pointer-linked blocks. Record sizes vary; follow the pointers rather than
assuming a 212-byte stride.

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
| 0x54   | 16   | —        | display limits | Observed integer and float display limits |
| 0x64   | 1    | uint8    | display decimals | Separate from scaling `dec_places` |
| 0x65   | 1    | uint8    | display format | Observed: 1 = integer, 3 = float |
| 0x66   | 14   | —        | reserved | |
| 0x74   | 4    | uint32   | validity pointer | Optional validity transition block |
| 0x78   | 4    | uint32   | extension pointer | Optional extended attributes block |

### Validity transitions

At the pointer stored at `+0x74`, M1 logs contain a block with `u16 type` (0),
`u16 frequency` (matching the channel), `u32 count`, and `u32 records_ptr`.
Each pointed-to record is six bytes: `u16 valid` (0 or 1) and `u32 sample_index`.
A transition applies starting at that sample index. Invalid intervals are half-open;
a final invalid transition without a following valid transition runs to the end.
A transition at `n_data` is permitted as an end marker.

Decode invalid samples as NaN, including range reads, and retain their positions.
Do not compact the time series or replace invalid samples with zero or held values.
Validate pointer bounds, frequency, flags, counts, and monotonic sample indices
before applying the block. No validity pointer means there is no additional mask.

### Extended channel attributes

The block pointed to by `+0x78` contains an attribute-list pointer at its own `+2`.
The attribute list starts with `u32 version` (1) and `u32 count`. Each entry has
`u32 kind`, `u16 reserved`, `u16 payload_length`, then that many payload bytes:

- Kind 4: null-terminated full channel name.
- Kind 5: two-byte enum table ID.

More than one attribute may be present. Both the extension and list pointers are
absolute file offsets. Validity blocks may appear before the attributes, so the
old `meta + 0xD0 / 0xD2` enum shortcut is only coincidentally correct in records
without validity transitions. It must not be used as a general format rule.

### Terminator Entry

The linked list ends with a terminator entry where `next_addr = 0` and `n_data = 0`.
The terminator typically has an empty name.

### Channel Name / Short Name / Unit Quirks (M1 ECUs)

On MoTeC M1 ECUs (as used in the Lotus Evora), the three string fields behave differently
from other MoTeC loggers:

1. **The `short_name` field often contains the unit** rather than the `unit` field.
   The 12-byte `unit` field is frequently empty. When parsing, check `unit` first;
   if empty, fall back to `short_name`.

2. **The base name is truncated to 32 bytes.** Recover the full name from kind 4
   in the extended attributes when present. The short-name field is independent:
   never append a one- or two-character unit to the channel name. For example,
   `CP.Lotus.ESP.Lateral Acceleratio` plus unit `G` is not a name ending in `G`.

3. **Display precision is independent of numeric scaling.** Float lambda channels
   have scaling exponent zero but display precision three. Use `+0x64` when the
   display format is recognized; older producers without it can fall back to a
   bounded scaling precision.

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
converted = (raw_value / scale) * 10^(-dec_places) * mul + shift
```

Where:
- `scale` — divisor (int16, typically 1)
- `dec_places` — decimal shift as power of 10 (int16, typically 0)
- `shift` — additive offset, in engineering units (int16, typically 0)
- `mul` — multiplier (int16, typically 1)

**Operator order matters.** `shift` is an offset already expressed in engineering
units, so it is applied *after* `mul`, not before. The two orderings agree whenever
`mul = 1`, which hides the difference in most M1 logs — but a file with `mul != 1`
and `shift != 0` comes out offset by `shift * (mul - 1)` if `shift` is folded in
first. Sim-derived logs (ADL device records exported by racing simulators) commonly
use `mul = 2` with a non-zero `shift` to pack a signed range into int16: e.g.
`Throttle Pos` with `shift=50, mul=2, scale=1, dec_places=3` stores 0–100 % as raw
−25000…25000, and the wrong order reports it as 50–150 %.

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

## Enum/State Tables

The ECU configuration section of the file embeds **enum tables** that map integer
sample values to human-readable text labels. These are used for state/enum channels
such as Gear, Brake State, Engine Speed Limit State, Torque Limit State, etc.

Enum tables are located in the latter portion of the file, after the channel sample
data. In observed M1 ECU logs, they appear roughly in the last quarter of the file.
There is no pointer from the header to this section — the tables must be found by
scanning for their header signature.

### Enum Table Header

Each table begins with a 6-byte header:

| Offset | Size | Type   | Field    | Description |
|--------|------|--------|----------|-------------|
| 0x00   | 2    | uint16 | count    | Number of entries in this table |
| 0x02   | 2    | uint16 | type     | Always `0x0002` — used as a signature for scanning |
| 0x04   | 2    | uint16 | enum_id  | Table ID (matches the `enum_id` in channel extended metadata) |

The header is immediately followed by the table entries.

### Enum Table Entries

Each entry is preceded by a 6-byte marker and contains a value–label pair:

```
[03 00 xx xx xx xx] [u16 entry_size] [u16 value] [null-terminated label string]
```

| Offset | Size | Type   | Field      | Description |
|--------|------|--------|------------|-------------|
| 0x00   | 2    | bytes  | marker     | Always `03 00` |
| 0x02   | 4    | bytes  | (unknown)  | Varies per entry, purpose unknown |
| 0x06   | 2    | uint16 | entry_size | Size in bytes of `value` + `label` (i.e. 2 + string length including null) |
| 0x08   | 2    | uint16 | value      | The integer sample value this label maps to |
| 0x0A   | var  | char[] | label      | Null-terminated ASCII label string |

Entries are packed contiguously. Some tables contain `06 00` separator bytes between
sub-groups of entries — these can be skipped during scanning.

Special value codes:
- `0xFFFF` — sentinel, typically mapped to value `-1` for display
- `0xFFFE` — sentinel, typically mapped to value `-2` for display

### Scanning Algorithm

Since there is no direct pointer to the enum tables section:

1. Read `chan_data_ptr` from the file header to identify the start of the raw sample-data region
2. Scan only the non-sample prefix of the file: from `HEAD_SIZE` up to `chan_data_ptr`
3. Search for the signature pattern: `[u16 count] [02 00] [u16 id]` followed by `[03 00]`
4. Validate by checking `count` is in range 1–500
5. Attempt to parse `count` entries starting after the header
6. Accept the table only if exactly `count` entries were successfully parsed
7. After a successful parse, advance to the end of that table instead of rescanning its entry bytes

### Linking Channels to Enum Tables

Read kind 5 from the channel's pointer-linked extended attributes. Its two-byte
payload is the enum ID, matching the `enum_id` in the table header. Look up finite
sample values in that table; missing samples must never be cast to integer zero
and displayed as a valid state.

### Observed Enum Tables

From a Lotus Evora M1 ECU log (VIR_LAP.ld), **275 enum tables** were found. Examples:

| enum_id | Channel                    | count | Example entries |
|---------|----------------------------|-------|-----------------|
| 217     | Gear                       | 10    | 0=Neutral, 1=First, 2=Second, …, 7=Seventh, 65534=Default, 65535=Reverse |
| 149     | Brake State                | 4     | 0=Unknown, 1=Off, 2=On, 65535=Sensor Fault |
| 137     | Engine State               | 4     | 0=Startup, 1=Stop, 2=Run, 3=Crank |
| 205     | Engine Speed Limit State   | 35    | 0=Maximum, 1=Engine Load Average, 2=Throttle Servo Fault, 3=Coolant Temperature, 4=Traction Control, … |
| 194     | Torque Limit State         | 12    | 0=Maximum, 1=Coolant Temperature, 2=Engine Oil Temperature, 3=Lotus ESP, … |
| 175     | Clutch State               | 4     | 0=Clutch Fully Engaged, 1=Clutch Slipping, 2=Clutch Fully Disengaged, 3=Invalid Signal |
| 138     | Traction State / Launch State | 2  | 0=Disabled, 1=Enabled |

The Engine Speed Limit State table (id 205) is notably large (35 entries), covering
limiter states like Maximum, Engine Load Average, Traction Control, Launch, Vehicle
Speed Limit, Anti Lag, Gear Shift, and various warning-based limiters (Coolant,
Oil Temperature/Pressure, Fuel Pressure, Exhaust Lambda/Temperature, etc.).

Multiple channels can reference the same enum table (e.g., Traction State and
Launch State both use table 138).

**Status:** Enum references are read through the extended attribute list described above.
The exact meaning of the 4 unknown bytes in each entry marker (`03 00 xx xx xx xx`)
and the `06 00` sub-group separators is not fully understood.

---

## Session Duration

No explicit session endpoint is currently decoded. Estimate the endpoint from
channels at the highest recorded frequency, taking the longest of those channels:

```
max_freq = max(ch.rec_freq for nonempty channels)
duration = max(ch.n_data / ch.rec_freq for channels with ch.rec_freq == max_freq)
```

Lower-rate channels include a final partial sample period. Taking their maximum
`count / frequency` incorrectly rounds the session up: the May 8 S1 log ends at
3918.210 s in i2 and its 100 Hz channels, but its 1 Hz channels contain 3919 samples.
All lap-detection paths use the same session endpoint. This remains an estimate
for formats that independently stop recording different channels.

## Recorded beacon events

`Beacon (Internal)` is a tagged int16 event stream, not a physical measurement.
An event consists of three consecutive words with high-bit tags `11`, `10`, `01`.
The low 14 bits of the third word contain milliseconds within the second (0–999).
The first word is stored in the sample bucket immediately after the crossing.
For its index `i` and stream frequency `f`, the crossing is in `[(i-1)/f, i/f]`;
combine that interval with the milliseconds to recover local session time. Allow
half a millisecond for timestamp quantization. Validate tags, timestamp range,
and strict ordering. This also handles cropped logs with a retained ECU counter
origin, and closely spaced events in the 68 Hz stream used by the May 8 file.

Use recorded beacons before the `Lap Time Running` or `Lap Number` fallbacks.
A timer reset can mean an ECU restart and is not necessarily a beacon crossing.
Verification against i2: all 19 May 8 crossings match to the millisecond, including
814.515 and 814.574; the cropped VIR fixture retains its 128.392 s timed lap.

---

## References

- Format originally reverse-engineered by the [gotzl/ldparser](https://github.com/gotzl/ldparser) project.
- Verified and extended against real Lotus Evora S1 logs from a MoTeC M1 ECU (2025).

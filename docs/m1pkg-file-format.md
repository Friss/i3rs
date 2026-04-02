# MoTeC M1 Package Archive (.m1pkg-archive) Format

Binary format documentation for MoTeC M1 `.m1pkg-archive` files, as produced by M1 Build
and M1 Tune. These files contain firmware, calibration data, and metadata for M1 ECUs.

Reverse-engineered from a Calibrated Performance Lotus V6 Exige S package.

## Overview

The file is structured as:

1. **Outer header** (12 bytes) — magic, size info, pointer to zlib stream
2. **Zlib-compressed payload** — the bulk of the file, starting at offset 0x14
3. **Inner "Mo" record tree** — a proprietary serialized tree of typed records

The decompressed payload is a flat sequence of **Mo records** — each prefixed with the
ASCII magic `"Mo"` (0x4D 0x6F). Records encode a hierarchical key-value tree containing
package metadata, calibration proxy references, table view definitions (as XML), comment
history, and the actual calibration table data in a proprietary binary encoding.

---

## Outer Header

| Offset | Size | Type   | Field          | Description |
|--------|------|--------|----------------|-------------|
| 0x00   | 4    | uint32 | magic          | Always `0xFFFF0000` |
| 0x04   | 2    | char[2]| signature      | `"Mo"` |
| 0x06   | 2    | uint16 | (unknown)      | |
| 0x08   | 4    | uint32 | (unknown)      | |
| 0x0C   | 4    | uint32 | (unknown)      | |
| 0x10   | 4    | uint32 | compressed_size| Size hint or checksum |
| 0x14   | —    | bytes  | zlib_stream    | Zlib-compressed payload (magic `0x789C`) |

Decompress from offset `0x14` using standard zlib (`zlib.decompress(data[0x14:])`).

---

## Mo Record Format

Each record in the decompressed payload starts with `"Mo"` (2 bytes) followed by a
10-byte header. The header encodes the record type and payload specification.

```
[ 'M' 'o' ] [ b0 b1 b2 b3 b4 b5 b6 b7 b8 b9 ]
  magic (2)          header (10 bytes)
```

### Record Types

| Header pattern (b0–b9)         | Type      | Payload | Description |
|-------------------------------|-----------|---------|-------------|
| `00 00 00 01 00 00 00 00 HH LL` | STRING  | HH:LL bytes of null-padded ASCII | String value; length = `(b8<<8)\|b9` |
| `00 00 01 01 00 08 XX XX XX XX` | VALUE   | 8 bytes inline in header | Typed scalar; first 2 payload bytes = value type |
| `00 00 01 00 XX XX XX XX XX XX` | REF     | none (structural) | Container/reference; links tree structure |
| Other                          | CONTEXT | varies | Embedded in larger data blocks |

String records are followed by null-padding to align to the next record boundary.

### Value Type Codes (first 2 bytes of VALUE payload)

| Code   | Interpretation |
|--------|---------------|
| 0x0201 | Unsigned integer (4 bytes LE following) |
| 0x0203 | Unsigned integer |
| 0x0205 | Unsigned integer |
| 0x0207 | Unsigned integer |
| 0x0208 | Unsigned integer |
| 0x0220 | Unsigned integer |
| 0x0254 | Unsigned integer (state/flags) |
| 0x0005 | Unsigned integer |
| 0x028b | Device serial (6 bytes) |

---

## Payload Structure

The Mo record tree encodes the following sections in order:

### 1. Package Metadata

Key-value STRING pairs for package identity:

| Key | Example Value |
|-----|--------------|
| `description` | `CP Ltd Lotus V6 Exige S [01.01.0032, November 2020] EVORA_Friss , 250825_FirmwareUpdate-015` |
| `name` | `B9569B6A-E185-492C-8F1B-A6D5DE83C3D4.m1pkg` |
| `schema` | (integer) |
| `project.name` | `CP Ltd Lotus V6 Exige S` |
| `version.major` | `0` |
| `version.minor` | `0` |
| `version.build` | (integer) |
| `device.boardtype` | (integer) |
| `device.serialnum` | (6-byte serial) |

### 2. Summary Block

Key-value STRING pairs prefixed with `Summary.`:

| Key | Example Value |
|-----|--------------|
| `Summary.Firmware` | `CP Ltd Lotus V6 Exige S` |
| `Summary.Firmware Version` | `01.01.0032` |
| `Summary.Firmware Version Name` | `November 2020` |
| `Summary.Firmware Description` | HTML changelog (see below) |
| `Summary.Hardware` | `M150` |
| `Summary.System Version` | `01.04.00.0105` |
| `Summary.Firmware Licence` | `CP Ltd Lotus V6 Exige S.November 2020` |
| `Summary.Minimum Data Logging` | `Data Logging Level 1` |
| `Summary.Author` | `Daniel Elias` |
| `Summary.Company` | `Calibrated Performance Pty Ltd.` |
| `Summary.Contact No` | `+61 (0) 7 3497 5863` |
| `Summary.Address` | `2/8 Exeter Way, Caloundra West, QLD 4551, Australia` |
| `Summary.Email` | `info@calibratedperformance.com.au` |
| `Summary.VehicleId` | `EVORA_Friss` |
| `Summary.Comment` | `250825_FirmwareUpdate-015` |
| `Summary.Build Version` | `1.4.4.898` |
| `Summary.Copyright` | `CLE Group Pty Ltd` |

### 3. Firmware Description (HTML Changelog)

The `Summary.Firmware Description` field contains an HTML-formatted changelog covering
all firmware versions. Each version is an `<h4>` heading followed by `<ul><li>` items.

### 4. Calibration Proxy References

Key-value pairs under `pkg.calibrations` specifying sensor/component part numbers:

| Key | Example Value |
|-----|--------------|
| `Fuel.Properties.Calibration.Proxy` | `Gasoline 98 Octane.1.0` |
| `Fuel.Injector.Primary.Calibration.Proxy` | `Denso 23250 31130.1.0` |
| `Ignition.Coil.Calibration.Proxy` | `Denso 90919-A2002.1.0` |
| `Coolant.Temperature.Sensor.Calibration.Proxy` | `Denso 89422-33030.1.0` |
| `Inlet.Air.Temperature.Sensor.Calibration.Proxy` | `Bosch 0 281 002 845.1.0` |
| `Inlet.Manifold.Pressure.Sensor.Calibration.Proxy` | `GM Delphi 2817 2033.1.0` |
| `CP.Lotus.Fuel Tank.Pressure.Calibration.Proxy` | `Bosch 0 261 230 099.1.0` |

### 5. Comment/Notes History

A `comment.notes` STRING record containing the full per-update change log in plain text.
Each entry is formatted as:

```
YYMMDD_FirmwareUpdate-NNN
- Change description
- Change description
```

### 6. Migration History

Records tracking package lineage:

| Key | Description |
|-----|-------------|
| `comment.log` | Build tool version and auto-migration source |
| `pkg.parent.ref` | GUID of parent package |
| `pkg.upgrades` | Firmware project being upgraded |
| `migrated_from.data_hash` | MD5 hash of source calibration data |

### 7. Table View Definitions (Embedded XML)

An XML block (`<?xml ... </Objs>`) defines the **view layout** for calibration tables
in M1 Tune. This includes axis assignments and enable/disable flags but **not** the
actual table values.

Example:
```xml
<?xml version="1.0"?>
<Objs Locale="English_Australia.1252" DefaultLocale="C">
 <Table Name="Fuel.Mixture Aim.Main">
  <Axis Z.Enabled="0"/>
 </Table>
 <Table Name="Ignition.Timing.Main">
  <Axis Z.Enabled="0"/>
 </Table>
 <!-- ... -->
</Objs>
```

#### Known Table Names

**Engine/Fuel/Ignition:**
- `Alternative Fuel.Ignition Timing`
- `Alternative Fuel.Mixture Aim`
- `Fuel.Mixture Aim.Main`
- `Ignition.Timing.Main`
- `Inlet.Manifold.Pressure.Estimate.Main`
- `Engine.Overrun.Ignition Timing.Target`

**CP Custom (Calibrated Performance):**
- `CP.Engine Speed Targeting.Throttle Control.Derivative.Gain`
- `CP.Engine Speed Targeting.Throttle Control.Integral.Gain`
- `CP.Engine Speed Targeting.Throttle Control.Proportional.Gain`
- `CP.Lotus.Exhaust.Bypass Valve.Activate.Engine Speed.Threshold`
- `CP.Lotus.Exhaust.Bypass Valve.Activate.Load.Threshold`
- `CP.Lotus.Exhaust.Bypass Valve.Fuel Mixture Aim`
- `CP.Lotus.Exhaust.Bypass Valve.Fuel Volume Trim`
- `CP.Lotus.Exhaust.Bypass Valve.Ignition Timing Trim`
- `CP.Lotus.Fuel Tank.Level.Sensor.Translation`
- `CP.Lotus.Gauge Pack.Shift Light Centre`
- `CP.Lotus.Gauge Pack.Shift Light Left`
- `CP.Lotus.Gauge Pack.Shift Light Right`
- `CP.Lotus.IPS.Kickdown.Pedal Position`

**Throttle/Idle:**
- `Throttle.Pedal.Translation`
- `Throttle.Aim.Minimum.Main`
- `Throttle.Aim.Minimum.Compensation`
- `Throttle.Area`
- `Idle.Aim.Main`
- `Idle.Mass Flow.Disabled`

**Drivetrain:**
- `Gear.Ratio.Value`
- `Gear.Shift.Fuel Cut.Main`
- `Gear.Shift.Ignition Cut.Main`
- `Gear.Shift.Ignition Timing.Retard`
- `Gear.Shift.Throttle Aim.Main`

**Traction/Torque:**
- `Traction.Aim.Main`
- `Traction.Range.Main`
- `Torque.Control.Throttle.Proportional.Gain`

**Other:**
- `Vehicle.Speed.Limit.Pit.Value`

### 8. Logging System Configuration

Multiple `display_name` entries define the logging system pages:

| Index | Display Name |
|-------|-------------|
| 0     | Normal |
| 1     | System 2 |
| 2     | System 3 |
| ...   | ... |
| 7     | System 8 |

### 9. Calibration Table Data (Binary, Proprietary)

The bulk of the decompressed payload (~3 MB) contains the actual calibration table values.
This data is stored in MoTeC's proprietary binary encoding within the Mo record tree and
**cannot be decoded** without MoTeC's M1 Tune software.

The data does not use standard IEEE 754 float encoding at any alignment — attempts to
interpret it as little-endian or big-endian float32/float64 yield nonsensical values.
The encoding likely involves custom fixed-point representations, compression, or
encryption specific to the M1 platform.

**To extract actual calibration values, use M1 Tune's export/reporting features.**

---

## Extraction Example (Python)

```python
import zlib

with open("package.m1pkg-archive", "rb") as f:
    data = f.read()

# Decompress payload
decompressed = zlib.decompress(data[0x14:])

# Extract readable strings (metadata, table names, comments)
import re
strings = re.findall(b'[\x20-\x7e]{10,}', decompressed)
for s in strings:
    print(s.decode('ascii'))
```

---
## References

- Verified and extended against real Lotus Evora S1 logs from a MoTeC M1 ECU (2025).
- M1 package format reverse-engineered from a Calibrated Performance `.m1pkg-archive` (2025).

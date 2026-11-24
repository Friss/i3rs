//! Native decoding of the plain M1 logging packet stream.
//!
//! Samples retain their actual packet timestamps and source words. Sectors are
//! stored newest first by Tune, while records inside a sector run forwards.
//! Power-on records split recordings; no samples are filled across gaps.

use super::catalog::{Catalog, ValueType};
use super::log_image::{MoRecord, MoRecordIter};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Explicit timing departure from Tune's presentation timeline. The raw clock
/// remains available through segment tick bounds and timestamped samples.
/// Tune's decoder at 0x9109c0 clamps gaps over 30,000,000 us to a rounded
/// previous time plus 5,000,000 us; 0x90fbd0 also floors its initial tick origin.
pub const TIMESTAMP_POLICY: &str = "Native timestamps start each power/reset segment at its first numeric sample and preserve full ECU elapsed time, including logging gaps. M1 Tune can round segment origins and shorten gaps over 30 seconds to approximately 5 seconds; native recording duration and alignment therefore differ from Tune's presentation timeline.";

const SETUPS: u32 = 0x0200_0020;
const SETUP: u32 = 0x0200_0022;
const SETUP_INFO: u32 = 0x0201_0023;
const GROUP: u32 = 0x0200_0024;
const GROUP_INFO: u32 = 0x0201_0025;
const ENTRY: u32 = 0x0201_0026;
const STREAM: u32 = 0x0101_0002;
const SECTOR: u32 = 0x0200_004c;
const SECTOR_ID: u32 = 0x0201_004d;
const SYSINFO: u32 = 0x0201_0028;
const TIMING: u32 = 0x0201_0029;
const POWER_ON: u32 = 0x0201_002a;
const VALUES: u32 = 0x0201_002b;
const SETUP_ID: u32 = 0x0201_0039;
const FIRMWARE_STARTED: u32 = 0x0201_003a;

#[derive(Debug, Clone, Copy)]
pub struct TelemetryLimits {
    pub max_records: usize,
    pub max_sectors: usize,
    pub max_channels: usize,
    pub max_samples: usize,
}

impl Default for TelemetryLimits {
    fn default() -> Self {
        Self {
            max_records: 4_000_000,
            max_sectors: 10_000,
            max_channels: 16_384,
            max_samples: 64_000_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryError(pub String);
impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for TelemetryError {}
fn invalid(message: impl Into<String>) -> TelemetryError {
    TelemetryError(message.into())
}

#[derive(Debug, Clone)]
struct PacketGroup {
    entries: Vec<(u32, u32)>,
    words: usize,
}
#[derive(Debug, Clone)]
struct LoggingSetup {
    groups: BTreeMap<u8, PacketGroup>,
}
#[derive(Debug, Clone, Default)]
pub struct LoggingSetups {
    configurations: BTreeMap<(u8, [u8; 16]), LoggingSetup>,
}

#[derive(Debug, Clone)]
pub struct TimestampedSample {
    /// Seconds since this segment's first numeric packet, preserving gaps.
    pub time_seconds: f64,
    pub value: f64,
    pub source_word: u32,
}
#[derive(Debug, Clone)]
pub struct TelemetryChannel {
    pub data_type: ValueType,
    pub base: u32,
    pub address: u32,
    pub name: String,
    pub unit: String,
    pub enum_labels: BTreeMap<i32, String>,
    pub display_decimals: u8,
    pub frequency_hz: f64,
    pub samples: Vec<TimestampedSample>,
}
#[derive(Debug, Clone)]
pub struct DecodedSegment {
    pub index: usize,
    pub start_ticks: u64,
    pub end_ticks: u64,
    pub channels: Vec<TelemetryChannel>,
}
#[derive(Debug, Clone)]
pub struct DecodedSystem {
    pub system_id: u8,
    pub segments: Vec<DecodedSegment>,
}
#[derive(Debug, Clone)]
pub struct DecodedTelemetry {
    pub systems: Vec<DecodedSystem>,
    /// Device clock, or Tune's observed 125 MHz fallback when unavailable.
    pub clock_hz: f64,
    pub used_default_clock: bool,
}

struct Budget {
    left: usize,
}
impl Budget {
    fn take(&mut self) -> Result<(), TelemetryError> {
        self.left = self
            .left
            .checked_sub(1)
            .ok_or_else(|| invalid("telemetry record limit exceeded"))?;
        Ok(())
    }
}
fn children<'a>(
    record: MoRecord<'a>,
    budget: &mut Budget,
) -> Result<Vec<MoRecord<'a>>, TelemetryError> {
    plain(record)?;
    let mut result = Vec::new();
    for item in MoRecordIter::new(record.payload, budget.left) {
        let item = item.map_err(|e| invalid(e.to_string()))?;
        budget.take()?;
        result.push(item);
    }
    Ok(result)
}
fn plain(record: MoRecord<'_>) -> Result<(), TelemetryError> {
    if usize::try_from(record.header.payload_len).ok() != Some(record.payload.len()) {
        return Err(invalid(
            "telemetry record payload length does not match its span",
        ));
    }
    if record.header.encoding != 0 {
        return Err(invalid(format!(
            "transformed telemetry record {:08x} requires decoding first",
            record.header.record_type
        )));
    }
    Ok(())
}
fn kind(record: MoRecord<'_>, expected: u32) -> Result<(), TelemetryError> {
    plain(record)?;
    if record.header.record_type != expected {
        return Err(invalid(format!(
            "expected telemetry record {expected:08x}, found {:08x}",
            record.header.record_type
        )));
    }
    Ok(())
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, TelemetryError> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated telemetry field"))?
            .try_into()
            .unwrap(),
    ))
}
fn unsupported(record: MoRecord<'_>) -> TelemetryError {
    invalid(format!(
        "unsupported telemetry record {:08x}",
        record.header.record_type
    ))
}

/// Parse logging configurations, retaining their system and configuration GUID.
/// Counts and addresses are checked before expanding contiguous 32-bit entries.
pub fn parse_setups(
    record: MoRecord<'_>,
    limits: TelemetryLimits,
) -> Result<LoggingSetups, TelemetryError> {
    kind(record, SETUPS)?;
    let mut budget = Budget {
        left: limits.max_records,
    };
    budget.take()?;
    let mut result = LoggingSetups::default();
    let mut channels_left = limits.max_channels;
    for setup in children(record, &mut budget)? {
        kind(setup, SETUP)?;
        let system = setup.header.flags;
        if system >= 8 {
            return Err(invalid("logging setup system must be in 0..8"));
        }
        let mut guid = None;
        let mut groups = BTreeMap::new();
        let mut addresses = BTreeSet::new();
        for item in children(setup, &mut budget)? {
            plain(item)?;
            match item.header.record_type {
                SETUP_INFO => {
                    if guid.is_some() || item.payload.len() != 32 {
                        return Err(invalid("invalid or duplicate logging setup info"));
                    }
                    guid = Some(<[u8; 16]>::try_from(&item.payload[16..32]).unwrap());
                }
                GROUP => {
                    let mut header = None;
                    let mut entries = Vec::new();
                    let mut actual_entries = 0usize;
                    for field in children(item, &mut budget)? {
                        plain(field)?;
                        match field.header.record_type {
                            GROUP_INFO => {
                                if header.is_some()
                                    || field.payload.len() != 20
                                    || word(field.payload, 0)? != 1
                                {
                                    return Err(invalid("unsupported logging packet setup header"));
                                }
                                header = Some((
                                    word(field.payload, 8)?,
                                    word(field.payload, 12)?,
                                    word(field.payload, 16)?,
                                ));
                            }
                            ENTRY => {
                                if field.payload.len() != 12 {
                                    return Err(invalid("invalid logging setup entry length"));
                                }
                                let base = word(field.payload, 0)?;
                                let address = word(field.payload, 4)?;
                                let count = word(field.payload, 8)?;
                                if count == 0 {
                                    return Err(invalid("zero logging setup entry count"));
                                }
                                let count = usize::try_from(count)
                                    .map_err(|_| invalid("logging entry count overflow"))?;
                                if count > channels_left {
                                    return Err(invalid("telemetry channel limit exceeded"));
                                }
                                channels_left -= count;
                                actual_entries += 1;
                                for i in 0..count {
                                    let delta = u32::try_from(i)
                                        .ok()
                                        .and_then(|i| i.checked_mul(4))
                                        .ok_or_else(|| invalid("logging entry address overflow"))?;
                                    let address = address
                                        .checked_add(delta)
                                        .ok_or_else(|| invalid("logging entry address overflow"))?;
                                    if !addresses.insert((base, address)) {
                                        return Err(invalid(
                                            "duplicate channel in logging configuration",
                                        ));
                                    }
                                    entries.push((base, address));
                                }
                            }
                            _ => return Err(unsupported(field)),
                        }
                    }
                    let (index, entry_count, bytes) =
                        header.ok_or_else(|| invalid("missing logging packet setup header"))?;
                    let index = u8::try_from(index)
                        .map_err(|_| invalid("logging packet index exceeds 255"))?;
                    // Count fields count source entry records, not expanded words.

                    if entry_count as usize != actual_entries
                        || entries.len().checked_mul(4) != Some(bytes as usize)
                    {
                        return Err(invalid("logging packet setup count mismatch"));
                    }
                    let words = entries.len();
                    if groups
                        .insert(index, PacketGroup { entries, words })
                        .is_some()
                    {
                        return Err(invalid("duplicate logging packet index"));
                    }
                }
                0x0100_0009 | 0x0201_0044 => {} // Version properties and metadata GUID.
                _ => return Err(unsupported(item)),
            }
        }
        let guid = guid.ok_or_else(|| invalid("missing logging setup info"))?;
        if result
            .configurations
            .insert((system, guid), LoggingSetup { groups })
            .is_some()
        {
            return Err(invalid("duplicate system/configuration GUID"));
        }
    }
    Ok(result)
}

#[derive(Default)]
struct SystemState {
    synchronized: bool,
    leading_packets_skipped: usize,
    setup: Option<[u8; 16]>,
    high: Option<u32>,
    anchor: Option<u64>,
    previous: Option<u64>,
    current: BTreeMap<(u32, u32), TelemetryChannel>,
    segments: Vec<DecodedSegment>,
}
impl SystemState {
    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.segments.push(DecodedSegment {
                index: self.segments.len(),
                start_ticks: self.anchor.unwrap(),
                end_ticks: self.previous.unwrap(),
                channels: std::mem::take(&mut self.current).into_values().collect(),
            });
        }
        self.anchor = None;
        self.previous = None;
    }
}

/// Decode one untransformed binary stream into recordings split on power-on.
/// Sector order follows Tune's reverse traversal of its newest-first capture.
/// A missing setup, timing marker, channel definition, or CRC is an error.
pub fn decode_stream(
    record: MoRecord<'_>,
    setups: &LoggingSetups,
    catalog: &Catalog,
    limits: TelemetryLimits,
) -> Result<DecodedTelemetry, TelemetryError> {
    decode_stream_with_capture_start(record, setups, catalog, limits, false)
        .map(|(telemetry, _)| telemetry)
}

pub(super) fn decode_stream_with_capture_start(
    record: MoRecord<'_>,
    setups: &LoggingSetups,
    catalog: &Catalog,
    limits: TelemetryLimits,
    allow_mid_stream_start: bool,
) -> Result<(DecodedTelemetry, [usize; 8]), TelemetryError> {
    kind(record, STREAM)?;
    let clock_hz = catalog.sys_clock_hz.unwrap_or(125_000_000.0);
    if !clock_hz.is_finite() || clock_hz <= 0.0 {
        return Err(invalid("invalid logging clock frequency"));
    }
    let mut budget = Budget {
        left: limits.max_records,
    };
    budget.take()?;
    let mut sectors = Vec::new();
    for item in MoRecordIter::new(record.payload, budget.left) {
        let item = item.map_err(|e| invalid(e.to_string()))?;
        budget.take()?;
        if sectors.len() >= limits.max_sectors {
            return Err(invalid("telemetry sector limit exceeded"));
        }
        sectors.push(item);
    }
    let mut states: BTreeMap<u8, SystemState> = BTreeMap::new();
    let mut samples_left = limits.max_samples;
    let mut total_channels = 0usize;
    let mut sector_ids = BTreeSet::new();
    for sector in sectors.into_iter().rev() {
        kind(sector, SECTOR)?;
        let system = sector.header.flags;
        if system >= 8 {
            return Err(invalid("logging sector system must be in 0..8"));
        }
        let state = states.entry(system).or_default();
        let mut sector_id = None;
        for packet in children(sector, &mut budget)? {
            plain(packet)?;
            match packet.header.record_type {
                SECTOR_ID => {
                    if packet.payload.len() != 4 || sector_id.is_some() {
                        return Err(invalid("invalid or duplicate sector identifier"));
                    }
                    let id = word(packet.payload, 0)?;
                    if !sector_ids.insert((system, id)) {
                        return Err(invalid("duplicate logging sector identifier"));
                    }
                    sector_id = Some(id);
                }
                POWER_ON => {
                    if !packet.payload.is_empty() {
                        return Err(invalid("invalid power-on record length"));
                    }
                    state.flush();
                    state.synchronized = true;
                    state.high = None;
                    state.setup = None;
                }
                FIRMWARE_STARTED => {
                    if !packet.payload.is_empty() {
                        return Err(invalid("invalid firmware-start record length"));
                    }
                }
                SETUP_ID => {
                    let guid: [u8; 16] = packet
                        .payload
                        .try_into()
                        .map_err(|_| invalid("invalid logging configuration GUID"))?;
                    if !setups.configurations.contains_key(&(system, guid)) {
                        return Err(invalid("logged data has no matching logging configuration"));
                    }
                    if state.setup.is_some_and(|old| old != guid) {
                        state.flush();
                    }
                    state.setup = Some(guid);
                }
                TIMING => {
                    if packet.payload.len() != 4 {
                        return Err(invalid("invalid high-timing record length"));
                    }
                    state.high = Some(word(packet.payload, 0)?);
                }
                SYSINFO => {
                    if packet.payload.len() < 4 {
                        return Err(invalid("truncated system-information packet"));
                    }
                    // Debug text carries its own low timestamp but no samples.
                }
                VALUES => {
                    if packet.payload.len() < 8 || packet.payload.len() % 4 != 0 {
                        return Err(invalid("invalid logging values packet length"));
                    }
                    let body = packet.payload.len() - 4;
                    if crc32fast::hash(&packet.payload[..body]) != word(packet.payload, body)? {
                        return Err(invalid("logging values packet CRC mismatch"));
                    }
                    // A circular buffer can begin mid-recording even when all
                    // planned sectors were downloaded. CRC-check these packets
                    // but wait for explicit setup and clock markers to decode.
                    if allow_mid_stream_start
                        && !state.synchronized
                        && (state.setup.is_none() || state.high.is_none())
                    {
                        state.leading_packets_skipped += 1;
                        continue;
                    }
                    state.synchronized = true;
                    let guid = state
                        .setup
                        .ok_or_else(|| invalid("numeric packet precedes logging configuration"))?;
                    let setup = &setups.configurations[&(system, guid)];
                    let group = setup.groups.get(&packet.header.flags).ok_or_else(|| {
                        invalid("numeric packet has no matching packet configuration")
                    })?;
                    if (packet.payload.len() - 8) / 4 != group.words {
                        return Err(invalid(
                            "numeric packet word count differs from configuration",
                        ));
                    }
                    let high = state
                        .high
                        .ok_or_else(|| invalid("numeric packet precedes high-timing marker"))?;
                    let ticks = (u64::from(high) << 32) | u64::from(word(packet.payload, 0)?);
                    if state.previous.is_some_and(|previous| ticks < previous) {
                        return Err(invalid(
                            "logging clock moved backwards without a power-on marker",
                        ));
                    }
                    state.previous = Some(ticks);
                    let anchor = *state.anchor.get_or_insert(ticks);
                    let time_seconds = (ticks - anchor) as f64 / clock_hz;
                    if !time_seconds.is_finite() {
                        return Err(invalid("logging timestamp exceeds numeric range"));
                    }
                    let frequency = *catalog
                        .events
                        .get(&u32::from(packet.header.flags))
                        .ok_or_else(|| {
                            invalid("logging packet scheduler event is missing from metadata")
                        })?;
                    if !frequency.is_finite() || frequency <= 0.0 {
                        return Err(invalid("invalid logging scheduler frequency"));
                    }
                    if group.words > samples_left {
                        return Err(invalid("telemetry sample limit exceeded"));
                    }
                    samples_left -= group.words;
                    for (i, &(base, address)) in group.entries.iter().enumerate() {
                        let definition =
                            catalog.channels.get(&(base, address)).ok_or_else(|| {
                                invalid(format!("metadata missing channel at {base}:{address:08x}"))
                            })?;
                        let raw = word(packet.payload, 4 + 4 * i)?;
                        let value = match definition.data_type {
                            // Tune marks these two finite payloads as missing
                            // for Traction Aim. Keep the original word for
                            // inspection and mask it before applying units.
                            ValueType::F32
                                if matches!(raw, 0x7d8f_5c28 | 0x7e19_9999)
                                    && definition.name == "Traction Aim" =>
                            {
                                f64::NAN
                            }
                            ValueType::F32 => f32::from_bits(raw) as f64,
                            ValueType::U32 => raw as f64,
                            ValueType::I32 | ValueType::Enum => raw as i32 as f64,
                            ValueType::I32E7 => (raw as i32 as f64) * 1e-7,
                        };
                        let value = value * definition.display_scale + definition.display_offset;
                        let channel = match state.current.entry((base, address)) {
                            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                            std::collections::btree_map::Entry::Vacant(entry) => {
                                total_channels = total_channels
                                    .checked_add(1)
                                    .ok_or_else(|| invalid("telemetry channel count overflow"))?;
                                if total_channels > limits.max_channels {
                                    return Err(invalid("telemetry channel limit exceeded"));
                                }
                                entry.insert(TelemetryChannel {
                                    data_type: definition.data_type,
                                    base,
                                    address,
                                    name: definition.name.clone(),
                                    unit: definition.unit.clone(),
                                    enum_labels: definition.enum_labels.clone(),
                                    display_decimals: definition.display_decimals,
                                    frequency_hz: frequency,
                                    samples: Vec::new(),
                                })
                            }
                        };
                        if channel.frequency_hz != frequency {
                            return Err(invalid(
                                "channel appears at conflicting logging frequencies",
                            ));
                        }
                        channel.samples.push(TimestampedSample {
                            time_seconds,
                            value,
                            source_word: raw,
                        });
                    }
                }
                _ => return Err(unsupported(packet)),
            }
        }
        if sector_id.is_none() {
            return Err(invalid("logging sector identifier is missing"));
        }
    }
    let mut systems = Vec::new();
    let mut skipped = [0; 8];
    for (system_id, mut state) in states {
        state.flush();
        skipped[usize::from(system_id)] = state.leading_packets_skipped;
        if !state.segments.is_empty() {
            systems.push(DecodedSystem {
                system_id,
                segments: state.segments,
            });
        } else if state.leading_packets_skipped != 0 {
            return Err(invalid(format!(
                "logging system {} has numeric packets but no decodable setup and timestamp context",
                system_id + 1
            )));
        }
    }
    Ok((
        DecodedTelemetry {
            systems,
            clock_hz,
            used_default_clock: catalog.sys_clock_hz.is_none(),
        },
        skipped,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{ChannelDefinition, parse_catalog};
    use super::super::log_image::RawLogImage;
    use super::*;

    fn record(t: u32, index: u8, payload: &[u8]) -> Vec<u8> {
        let mut b = b"Mo".to_vec();
        b.extend([index, 0]);
        b.extend(t.to_be_bytes());
        b.extend((payload.len() as u32).to_be_bytes());
        b.extend(payload);
        b
    }
    fn joined(records: Vec<Vec<u8>>) -> Vec<u8> {
        let mut b = Vec::new();
        for r in records {
            while b.len() % 4 != 0 {
                b.push(0);
            }
            b.extend(r);
        }
        b
    }
    fn borrowed(b: &[u8]) -> MoRecord<'_> {
        MoRecordIter::new(b, 1).next().unwrap().unwrap()
    }
    fn setups() -> Vec<u8> {
        let mut info = vec![0; 16];
        info.extend([1; 16]);
        let header = [1u32, 7816, 0, 1, 12]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        let entry = [1u32, 0x100, 3]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        record(
            SETUPS,
            0,
            &record(
                SETUP,
                0,
                &joined(vec![
                    record(SETUP_INFO, 0, &info),
                    record(
                        GROUP,
                        0,
                        &joined(vec![
                            record(GROUP_INFO, 0, &header),
                            record(ENTRY, 0, &entry),
                        ]),
                    ),
                ]),
            ),
        )
    }
    fn packet(ticks: u32) -> Vec<u8> {
        packet_with_float(ticks, 1.5f32.to_bits())
    }
    fn packet_with_float(ticks: u32, float_word: u32) -> Vec<u8> {
        let mut p = [ticks, float_word, (-2i32) as u32, (-123456789i32) as u32]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        p.extend(crc32fast::hash(&p).to_be_bytes());
        record(VALUES, 0, &p)
    }
    fn sector(id: u32, mut packets: Vec<Vec<u8>>) -> Vec<u8> {
        packets.insert(0, record(SECTOR_ID, 0, &id.to_be_bytes()));
        record(SECTOR, 0, &joined(packets))
    }
    fn sync() -> Vec<Vec<u8>> {
        vec![
            record(SETUP_ID, 0, &[1; 16]),
            record(TIMING, 0, &1u32.to_be_bytes()),
        ]
    }
    fn catalog() -> Catalog {
        let mut channels = BTreeMap::new();
        for (address, data_type) in [
            (0x100, ValueType::F32),
            (0x104, ValueType::I32),
            (0x108, ValueType::I32E7),
        ] {
            channels.insert(
                (1, address),
                ChannelDefinition {
                    name: format!("Channel {address}"),
                    unit: String::new(),
                    data_type,
                    enum_labels: BTreeMap::new(),
                    display_decimals: 3,
                    display_scale: 1.0,
                    display_offset: 0.0,
                },
            );
        }
        Catalog {
            channels,
            events: BTreeMap::from([(0, 1.0)]),
            sys_clock_hz: Some(125_000_000.0),
            unit_conversion_warnings: BTreeMap::new(),
        }
    }
    fn decode(b: &[u8], limits: TelemetryLimits) -> Result<DecodedTelemetry, TelemetryError> {
        let s = setups();
        let s = parse_setups(borrowed(&s), TelemetryLimits::default())?;
        decode_stream(borrowed(b), &s, &catalog(), limits)
    }

    #[test]
    fn multiword_values_crc_and_high_word_wrap() {
        let mut p = sync();
        p.push(packet(u32::MAX - 15));
        p.push(record(TIMING, 0, &2u32.to_be_bytes()));
        p.push(packet(16));
        let b = record(STREAM, 0, &sector(0, p));
        let d = decode(&b, TelemetryLimits::default()).unwrap();
        let seg = &d.systems[0].segments[0];
        assert_eq!(seg.start_ticks, (1u64 << 32) | u64::from(u32::MAX - 15));
        assert_eq!(seg.end_ticks, (2u64 << 32) | 16);
        assert_eq!(seg.channels.len(), 3);
        assert_eq!(seg.channels[0].samples[0].value, 1.5);
        assert_eq!(seg.channels[1].samples[0].value, -2.0);
        assert!((seg.channels[2].samples[0].value + 12.3456789).abs() < 1e-10);
        assert_eq!(
            seg.channels[2].samples[0].source_word,
            (-123456789i32) as u32
        );
        assert_eq!(
            seg.channels[0].samples[1].time_seconds,
            32.0 / 125_000_000.0
        );
        let mut corrupt = b.clone();
        let index = corrupt
            .windows(4)
            .position(|w| w == 1.5f32.to_bits().to_be_bytes())
            .unwrap();
        corrupt[index] ^= 1;
        assert!(
            decode(&corrupt, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("CRC")
        );
    }
    #[test]
    fn locale_affine_transform_follows_raw_engineering_decode() {
        let raw_setups = setups();
        let setups = parse_setups(borrowed(&raw_setups), TelemetryLimits::default()).unwrap();
        let mut catalog = catalog();
        let float = catalog.channels.get_mut(&(1, 0x100)).unwrap();
        float.display_scale = 100.0;
        float.unit = "%".into();
        let gps = catalog.channels.get_mut(&(1, 0x108)).unwrap();
        gps.display_scale = 2.0;
        gps.display_offset = 3.0;
        let mut packets = sync();
        packets.push(packet(0));
        let bytes = record(STREAM, 0, &sector(0, packets));
        let decoded = decode_stream(
            borrowed(&bytes),
            &setups,
            &catalog,
            TelemetryLimits::default(),
        )
        .unwrap();
        let channels = &decoded.systems[0].segments[0].channels;
        assert_eq!(channels[0].unit, "%");
        assert_eq!(channels[0].samples[0].value, 150.0);
        assert!((channels[2].samples[0].value - (-12.3456789 * 2.0 + 3.0)).abs() < 1e-10);
        assert_eq!(channels[2].samples[0].source_word, (-123456789i32) as u32);
    }

    #[test]
    fn traction_aim_missing_words_are_masked_before_scaling() {
        let words = [0x7d8f_5c28, 0x7e19_9999, 0.125f32.to_bits()];
        let mut packets = sync();
        packets.extend(
            words
                .into_iter()
                .enumerate()
                .map(|(index, word)| packet_with_float(index as u32, word)),
        );
        let bytes = record(STREAM, 0, &sector(0, packets));
        let raw_setups = setups();
        let limits = TelemetryLimits::default();
        let setups = parse_setups(borrowed(&raw_setups), limits).unwrap();
        let mut catalog = catalog();
        let float = catalog.channels.get_mut(&(1, 0x100)).unwrap();
        float.name = "Traction Aim".into();
        float.display_scale = 100.;
        let decoded = decode_stream(borrowed(&bytes), &setups, &catalog, limits).unwrap();
        let samples = &decoded.systems[0].segments[0].channels[0].samples;
        assert_eq!(
            samples.iter().map(|s| s.source_word).collect::<Vec<_>>(),
            words
        );
        assert!(samples[0].value.is_nan() && samples[1].value.is_nan());
        assert_eq!(samples[2].value, 12.5);

        let ordinary_channel = decode(&bytes, limits).unwrap();
        let samples = &ordinary_channel.systems[0].segments[0].channels[0].samples;
        for (sample, word) in samples.iter().zip(words) {
            assert_eq!(sample.value, f64::from(f32::from_bits(word)));
        }
    }

    #[test]
    fn preserves_long_raw_gap_instead_of_tune_presentation_clamp() {
        let mut packets = sync();
        packets.push(packet(100));
        let end = ((1u64 << 32) | 100) + 8_037_500_000; //64.3s at125MHz.
        packets.push(record(TIMING, 0, &((end >> 32) as u32).to_be_bytes()));
        packets.push(packet(end as u32));
        let bytes = record(STREAM, 0, &sector(0, packets));
        let decoded = decode(&bytes, TelemetryLimits::default()).unwrap();
        let samples = &decoded.systems[0].segments[0].channels[0].samples;
        assert_eq!(samples[0].time_seconds, 0.0);
        assert_eq!(samples[1].time_seconds, 64.3);
        assert!(TIMESTAMP_POLICY.contains("30 seconds"));
    }
    #[test]
    fn sectors_reverse_but_records_forward_and_power_on_splits() {
        let mut old = sync();
        old.push(packet(100));
        let mut new = vec![packet(200), record(POWER_ON, 0, &[])];
        new.extend(sync());
        new.push(packet(20));
        let b = record(STREAM, 0, &joined(vec![sector(1, new), sector(0, old)]));
        let d = decode(&b, TelemetryLimits::default()).unwrap();
        assert_eq!(d.systems[0].segments.len(), 2);
        let a = &d.systems[0].segments[0].channels[0].samples;
        assert_eq!(a.len(), 2);
        assert_eq!(a[1].time_seconds, 100.0 / 125_000_000.0);
        assert_eq!(
            d.systems[0].segments[1].channels[0].samples[0].time_seconds,
            0.0
        );
    }
    #[test]
    fn missing_setup_timing_counts_and_unsupported_fail_explicitly() {
        let b = record(STREAM, 0, &sector(0, vec![packet(0)]));
        assert!(
            decode(&b, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("configuration")
        );
        let b = record(
            STREAM,
            0,
            &sector(0, vec![record(SETUP_ID, 0, &[1; 16]), packet(0)]),
        );
        assert!(
            decode(&b, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("timing")
        );
        let mut p = sync();
        p.push(record(0xdeadbeef, 0, &[]));
        let b = record(STREAM, 0, &sector(0, p));
        assert!(
            decode(&b, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("unsupported")
        );
        let mut p = sync();
        let mut values = 0u32.to_be_bytes().to_vec();
        values.extend(1u32.to_be_bytes());
        values.extend(crc32fast::hash(&values).to_be_bytes());
        p.push(record(VALUES, 0, &values));
        let b = record(STREAM, 0, &sector(0, p));
        assert!(
            decode(&b, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("word count")
        );
    }
    #[test]
    fn limits_and_backwards_clock_fail() {
        let mut p = sync();
        p.push(packet(100));
        p.push(packet(99));
        let b = record(STREAM, 0, &sector(0, p));
        assert!(
            decode(&b, TelemetryLimits::default())
                .unwrap_err()
                .0
                .contains("backwards")
        );
        let mut p = sync();
        p.push(packet(100));
        let b = record(STREAM, 0, &sector(0, p));
        for limits in [
            TelemetryLimits {
                max_samples: 2,
                ..TelemetryLimits::default()
            },
            TelemetryLimits {
                max_records: 2,
                ..TelemetryLimits::default()
            },
            TelemetryLimits {
                max_sectors: 0,
                ..TelemetryLimits::default()
            },
            TelemetryLimits {
                max_channels: 2,
                ..TelemetryLimits::default()
            },
        ] {
            assert!(decode(&b, limits).unwrap_err().0.contains("limit"));
        }
        let s = setups();
        assert!(
            parse_setups(
                borrowed(&s),
                TelemetryLimits {
                    max_channels: 2,
                    ..TelemetryLimits::default()
                }
            )
            .unwrap_err()
            .0
            .contains("limit")
        );
    }
    /// Optional local oracle; private fixture bytes are never embedded in tests.
    #[test]
    fn local_numeric_fixture_if_requested() {
        let Ok(path) = std::env::var("I3RS_M1_TELEMETRY_FIXTURE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 128 * 1024 * 1024);
        let image = RawLogImage::parse(&bytes).unwrap();
        let root = MoRecordIter::new(image.payload, 4096)
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let setup = *root
            .iter()
            .find(|r| r.header.record_type == SETUPS)
            .unwrap();
        let metadata = *root
            .iter()
            .find(|r| r.header.record_type == 0x0200_0046)
            .unwrap();
        let group = MoRecordIter::new(metadata.payload, 4096)
            .next()
            .unwrap()
            .unwrap();
        let xml = MoRecordIter::new(group.payload, 4096)
            .map(Result::unwrap)
            .find(|r| r.header.record_type == 0x0201_002c)
            .unwrap();
        let decoded =
            super::super::transform::decode_xz_record(&xml, 16 * 1024 * 1024, 64 * 1024).unwrap();
        let catalog = parse_catalog(borrowed(&decoded).payload, 100_000).unwrap();
        let setups = parse_setups(setup, TelemetryLimits::default()).unwrap();
        let stream = *root
            .iter()
            .find(|r| r.header.record_type == STREAM)
            .unwrap();
        let decoded = decode_stream(stream, &setups, &catalog, TelemetryLimits::default()).unwrap();
        assert_eq!(decoded.systems.len(), 1);
        assert_eq!(decoded.systems[0].segments.len(), 4);
        assert!(
            decoded.systems[0]
                .segments
                .iter()
                .all(|s| s.channels.len() == 196)
        );
        assert_eq!(
            decoded.systems[0]
                .segments
                .iter()
                .flat_map(|s| s.channels.iter())
                .map(|c| c.samples.len())
                .sum::<usize>(),
            595_297
        );
    }
}

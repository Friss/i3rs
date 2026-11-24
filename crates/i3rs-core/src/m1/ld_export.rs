//! Fresh engineering-unit `.ld` serialization for explicitly prepared grids.
//!
//! `encode_channels` serializes already prepared uniform grids. `encode_segment`
//! applies the explicit common-origin interpolation/hold policy below.
//! Private signed-enum attributes make this i3rs extended LD output;
//! interoperability with other applications has not been validated.

use std::collections::BTreeMap;

use crate::Session;

use super::{
    catalog::ValueType,
    telemetry::{DecodedSegment, TelemetryChannel, TimestampedSample},
};

const HEADER_SIZE: usize = 0x6e2;
const CHANNEL_BASE_SIZE: usize = 124;
const MAX_SAMPLES: usize = 20_000_000;
pub const SIGNED_ENUM_ATTRIBUTE_KIND: u32 = u32::from_le_bytes(*b"i3rs");
pub const RESAMPLING_POLICY: &str = "Common segment t0, per-channel k/frequency grids through the last segment sample. Floating engineering values use linear interpolation only inside source brackets no wider than 1.5 periods. Synchronized GPS coordinate pairs hold the previous pair at transitions to or from (0, 0). Integer/state values use previous-value hold for at most 1.5 periods. Missing coverage is NaN. Exact matches use 16 machine-epsilon relative time tolerance. Source timestamps must be strictly increasing; channels are never shifted to their own first sample.";

#[derive(Debug, Clone)]
pub struct OutputChannel {
    pub name: String,
    pub unit: String,
    pub frequency_hz: f64,
    pub values: Vec<f64>,
    pub enum_labels: BTreeMap<i32, String>,
    pub display_decimals: u8,
}

/// Resample onto an explicit common-origin policy, recorded in conversion
/// manifests through [`RESAMPLING_POLICY`]. Source values are engineering units.
pub fn encode_segment(
    segment: &DecodedSegment,
    session: &Session,
    max_output_bytes: usize,
) -> Result<Vec<u8>, String> {
    validate_session(session)?;
    if segment.channels.is_empty() || segment.channels.len() > usize::from(u16::MAX) {
        return Err("LD output requires one to 65535 channels".into());
    }
    let mut duration = 0f64;
    let mut source_count = 0usize;
    for channel in &segment.channels {
        validate_fields(
            &channel.name,
            &channel.unit,
            channel.frequency_hz,
            channel.display_decimals,
        )?;
        source_count = source_count
            .checked_add(channel.samples.len())
            .filter(|count| *count <= MAX_SAMPLES)
            .ok_or("LD source exceeds 20 million samples")?;
        let mut previous = None;
        for sample in &channel.samples {
            if !sample.time_seconds.is_finite()
                || sample.time_seconds < 0.
                || previous.is_some_and(|previous| sample.time_seconds <= previous)
            {
                return Err(format!(
                    "Channel {} has invalid or non-increasing source timestamps",
                    channel.name
                ));
            }
            previous = Some(sample.time_seconds);
            duration = duration.max(sample.time_seconds);
        }
    }
    if source_count == 0 {
        return Err("LD segment has no timestamped samples".into());
    }
    // Preflight the complete serialized size before allocating output grids.
    let mut size = HEADER_SIZE;
    grow(
        &mut size,
        segment
            .channels
            .len()
            .checked_mul(CHANNEL_BASE_SIZE)
            .ok_or("LD metadata size overflow")?,
        max_output_bytes,
    )?;
    let mut counts = Vec::new();
    counts
        .try_reserve_exact(segment.channels.len())
        .map_err(|_| "Could not allocate LD grid layout")?;
    let mut sample_count = 0usize;
    for channel in &segment.channels {
        let scaled = duration * channel.frequency_hz;
        if !scaled.is_finite() || scaled >= MAX_SAMPLES as f64 {
            return Err("LD output grid exceeds 20 million samples".into());
        }
        let count = (scaled + time_tolerance(scaled, 0.)).floor() as usize + 1;
        sample_count = sample_count
            .checked_add(count)
            .filter(|count| *count <= MAX_SAMPLES)
            .ok_or("LD output exceeds 20 million samples")?;
        counts.push(count);
        let extra = match enum_encoding(&channel.enum_labels)? {
            EnumEncoding::None => 0,
            EnumEncoding::Legacy(length) => {
                grow(&mut size, length, max_output_bytes)?;
                10
            }
            EnumEncoding::Private(length) => 8 + length,
        };
        grow(
            &mut size,
            6 + 8 + 8 + channel.name.len() + 1 + extra,
            max_output_bytes,
        )?;
    }
    let padding = (8 - size % 8) % 8;
    grow(&mut size, padding, max_output_bytes)?;
    grow(
        &mut size,
        sample_count
            .checked_mul(8)
            .ok_or("LD sample size overflow")?,
        max_output_bytes,
    )?;
    let mut channels = Vec::new();
    channels
        .try_reserve_exact(segment.channels.len())
        .map_err(|_| "Could not allocate LD output channels")?;
    let gps_pair = gps_coordinate_pair(&segment.channels);
    for (channel_index, (channel, count)) in segment.channels.iter().zip(counts).enumerate() {
        let gps_peer = gps_pair.and_then(|(lat, lon)| match channel_index {
            i if i == lat => Some(segment.channels[lon].samples.as_slice()),
            i if i == lon => Some(segment.channels[lat].samples.as_slice()),
            _ => None,
        });
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| "Could not allocate bounded LD output grid")?;
        let mut cursor = 0;
        for index in 0..count {
            values.push(resampled_value(
                channel,
                index as f64 / channel.frequency_hz,
                &mut cursor,
                gps_peer,
            ));
        }
        channels.push(OutputChannel {
            name: channel.name.clone(),
            unit: channel.unit.clone(),
            frequency_hz: channel.frequency_hz,
            values,
            enum_labels: channel.enum_labels.clone(),
            display_decimals: channel.display_decimals,
        });
    }
    encode_channels(&channels, session, max_output_bytes)
}

fn gps_coordinate_pair(channels: &[TelemetryChannel]) -> Option<(usize, usize)> {
    let find = |name: &str| {
        channels
            .iter()
            .position(|channel| channel.data_type == ValueType::I32E7 && channel.name == name)
    };
    let lat = find("GPS Latitude")?;
    let lon = find("GPS Longitude")?;
    let latitude = &channels[lat];
    let longitude = &channels[lon];
    // Compare by index only when both axes were logged at the same instants.
    (latitude.frequency_hz == longitude.frequency_hz
        && latitude.samples.len() == longitude.samples.len()
        && latitude
            .samples
            .iter()
            .zip(&longitude.samples)
            .all(|(a, b)| a.time_seconds == b.time_seconds))
    .then_some((lat, lon))
}

fn time_tolerance(a: f64, b: f64) -> f64 {
    f64::EPSILON * 16. * a.abs().max(b.abs()).max(1.)
}

fn resampled_value(
    channel: &TelemetryChannel,
    time: f64,
    cursor: &mut usize,
    gps_peer: Option<&[TimestampedSample]>,
) -> f64 {
    let samples = &channel.samples;
    while *cursor < samples.len()
        && samples[*cursor].time_seconds
            < time - time_tolerance(time, samples[*cursor].time_seconds)
    {
        *cursor += 1;
    }
    if let Some(sample) = samples.get(*cursor)
        && (sample.time_seconds - time).abs() <= time_tolerance(time, sample.time_seconds)
    {
        return sample.value;
    }
    let Some(left) = cursor.checked_sub(1).and_then(|index| samples.get(index)) else {
        return f64::NAN;
    };
    let period = 1. / channel.frequency_hz;
    let max_gap = 1.5 * period;
    let state = matches!(
        channel.data_type,
        ValueType::Enum | ValueType::I32 | ValueType::U32
    ) || !channel.enum_labels.is_empty()
        || crate::is_state_channel(&channel.name);
    if state {
        let age = time - left.time_seconds;
        return if age >= 0. && age <= max_gap + time_tolerance(age, max_gap) {
            left.value
        } else {
            f64::NAN
        };
    }
    let Some(right) = samples.get(*cursor) else {
        return f64::NAN;
    };
    let gap = right.time_seconds - left.time_seconds;
    if gap > max_gap + time_tolerance(gap, max_gap)
        || !left.value.is_finite()
        || !right.value.is_finite()
    {
        return f64::NAN;
    }
    if let Some(peer) = gps_peer {
        let left_no_fix = left.value == 0. && peer[*cursor - 1].value == 0.;
        let right_no_fix = right.value == 0. && peer[*cursor].value == 0.;
        if left_no_fix != right_no_fix {
            return left.value;
        }
    }
    let weight = (time - left.time_seconds) / gap;
    left.value * (1. - weight) + right.value * weight
}

/// Encode supplied grids as little-endian f64 samples in engineering units.
/// Frequencies must be integral 1–65535 Hz. Missing values must already be NaN;
/// there is no interpolation, padding or change to the supplied sample counts.
/// Full names use pointer-linked attributes; units must fit their 12-byte field.
pub fn encode_channels(
    channels: &[OutputChannel],
    session: &Session,
    max_output_bytes: usize,
) -> Result<Vec<u8>, String> {
    validate_session(session)?;
    if channels.is_empty() || channels.len() > usize::from(u16::MAX) {
        return Err("LD output requires one to 65535 channels".into());
    }
    let mut size = HEADER_SIZE;
    grow(
        &mut size,
        channels
            .len()
            .checked_mul(CHANNEL_BASE_SIZE)
            .ok_or("LD metadata size overflow")?,
        max_output_bytes,
    )?;
    let mut layouts = Vec::new();
    layouts
        .try_reserve_exact(channels.len())
        .map_err(|_| "Could not allocate LD metadata layout")?;
    let mut samples = 0usize;
    for (index, channel) in channels.iter().enumerate() {
        validate_channel(channel)?;
        samples = samples
            .checked_add(channel.values.len())
            .filter(|count| *count <= MAX_SAMPLES)
            .ok_or("LD output exceeds 20 million samples")?;
        let extension = size;
        grow(&mut size, 6, max_output_bytes)?;
        let attributes = size;
        let enum_encoding = enum_encoding(&channel.enum_labels)?;
        let enum_attribute_bytes = match enum_encoding {
            EnumEncoding::None => 0,
            EnumEncoding::Legacy(_) => 10,
            EnumEncoding::Private(length) => 8 + length,
        };
        let attribute_len = 8usize
            .checked_add(8)
            .and_then(|n| n.checked_add(channel.name.len() + 1))
            .and_then(|n| n.checked_add(enum_attribute_bytes))
            .ok_or("LD attribute size overflow")?;
        grow(&mut size, attribute_len, max_output_bytes)?;
        layouts.push(ChannelLayout {
            base: HEADER_SIZE + index * CHANNEL_BASE_SIZE,
            extension,
            attributes,
            enumeration: None,
            enum_encoding,
            data: 0,
        });
    }
    for (index, layout) in layouts.iter_mut().enumerate() {
        if let EnumEncoding::Legacy(table_len) = layout.enum_encoding {
            layout.enumeration = Some((size, (index + 1) as u16));
            grow(&mut size, table_len, max_output_bytes)?;
        }
    }
    // Keep sample arrays naturally aligned. Enum tables remain before this
    // header pointer, matching the reader's bounded enum-scan region.
    let padding = (8 - size % 8) % 8;
    grow(&mut size, padding, max_output_bytes)?;
    let first_data = size;
    for (channel, layout) in channels.iter().zip(&mut layouts) {
        layout.data = size;
        grow(
            &mut size,
            channel
                .values
                .len()
                .checked_mul(8)
                .ok_or("LD sample size overflow")?,
            max_output_bytes,
        )?;
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "Could not allocate bounded LD output")?;
    bytes.resize(size, 0);
    bytes[0] = 0x40;
    put_u32(&mut bytes, 8, HEADER_SIZE as u32);
    put_u32(&mut bytes, 12, first_data as u32);
    put_u32(&mut bytes, 0x42, 1_000_000);
    put_u32(&mut bytes, 0x46, session.device_serial);
    fixed_text(&mut bytes, 0x4a, 8, &session.device_type);
    put_u16(&mut bytes, 0x52, session.device_version);
    put_u32(&mut bytes, 0x56, channels.len() as u32);
    for (offset, width, text) in session_strings(session) {
        fixed_text(&mut bytes, offset, width, text);
    }
    for (index, (channel, layout)) in channels.iter().zip(&layouts).enumerate() {
        let base = layout.base;
        put_u32(
            &mut bytes,
            base,
            if index == 0 {
                0
            } else {
                layouts[index - 1].base as u32
            },
        );
        put_u32(
            &mut bytes,
            base + 4,
            layouts.get(index + 1).map_or(0, |next| next.base as u32),
        );
        put_u32(&mut bytes, base + 8, layout.data as u32);
        put_u32(&mut bytes, base + 12, channel.values.len() as u32);
        put_u16(&mut bytes, base + 16, (index + 1) as u16);
        for (offset, value) in [
            (18, 8),
            (20, 8),
            (22, channel.frequency_hz as u16),
            (24, 0),
            (26, 1),
            (28, 1),
            (30, 0),
        ] {
            put_u16(&mut bytes, base + offset, value);
        }
        // The mandatory legacy field carries a UTF-8 boundary-safe prefix;
        // attribute kind 4 always retains the complete, unmodified name.
        let mut prefix_len = channel.name.len().min(32);
        while !channel.name.is_char_boundary(prefix_len) {
            prefix_len -= 1;
        }
        fixed_text(&mut bytes, base + 32, 32, &channel.name[..prefix_len]);
        fixed_text(&mut bytes, base + 72, 12, &channel.unit);
        bytes[base + 100] = channel.display_decimals;
        bytes[base + 101] = 3;
        put_u32(&mut bytes, base + 120, layout.extension as u32);
        put_u16(&mut bytes, layout.extension, 0);
        put_u32(&mut bytes, layout.extension + 2, layout.attributes as u32);
        put_u32(&mut bytes, layout.attributes, 1);
        put_u32(
            &mut bytes,
            layout.attributes + 4,
            if channel.enum_labels.is_empty() { 1 } else { 2 },
        );
        let mut at = layout.attributes + 8;
        put_u32(&mut bytes, at, 4);
        put_u16(&mut bytes, at + 6, (channel.name.len() + 1) as u16);
        at += 8;
        fixed_text(&mut bytes, at, channel.name.len() + 1, &channel.name);
        at += channel.name.len() + 1;
        if let Some((table, id)) = layout.enumeration {
            put_u32(&mut bytes, at, 5);
            put_u16(&mut bytes, at + 6, 2);
            put_u16(&mut bytes, at + 8, id);
            write_enum(&mut bytes, table, id, &channel.enum_labels);
        } else if let EnumEncoding::Private(length) = layout.enum_encoding {
            put_u32(&mut bytes, at, SIGNED_ENUM_ATTRIBUTE_KIND);
            put_u16(&mut bytes, at + 6, length as u16);
            write_private_enum(&mut bytes, at + 8, &channel.enum_labels);
        }
        for (sample_index, value) in channel.values.iter().enumerate() {
            let at = layout.data + sample_index * 8;
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(bytes)
}

struct ChannelLayout {
    base: usize,
    extension: usize,
    attributes: usize,
    enumeration: Option<(usize, u16)>,
    enum_encoding: EnumEncoding,
    data: usize,
}

fn grow(size: &mut usize, additional: usize, limit: usize) -> Result<(), String> {
    *size = size
        .checked_add(additional)
        .filter(|size| *size <= limit && u32::try_from(*size).is_ok())
        .ok_or("LD output exceeds the byte limit or 32-bit format address space")?;
    Ok(())
}

fn valid_text(text: &str, limit: usize, field: &str, allow_empty: bool) -> Result<(), String> {
    if text.len() > limit
        || text.contains('\0')
        || text.trim() != text
        || (!allow_empty && text.is_empty())
    {
        return Err(format!(
            "LD {field} must fit {limit} UTF-8 bytes without NUL or surrounding whitespace"
        ));
    }
    Ok(())
}

fn session_strings(session: &Session) -> [(usize, usize, &str); 6] {
    [
        (0x5e, 16, &session.date),
        (0x7e, 16, &session.time),
        (0x9e, 64, &session.driver),
        (0xde, 64, &session.vehicle_id),
        (0x15e, 64, &session.venue),
        (0x624, 64, &session.short_comment),
    ]
}

fn validate_session(session: &Session) -> Result<(), String> {
    valid_text(&session.device_type, 8, "device type", true)?;
    for (_, width, text) in session_strings(session) {
        valid_text(text, width, "session detail", true)?;
    }
    Ok(())
}

fn validate_channel(channel: &OutputChannel) -> Result<(), String> {
    validate_fields(
        &channel.name,
        &channel.unit,
        channel.frequency_hz,
        channel.display_decimals,
    )?;
    if channel.values.is_empty() || u32::try_from(channel.values.len()).is_err() {
        return Err("LD channels require a nonempty 32-bit sample count".into());
    }
    Ok(())
}

fn validate_fields(
    name: &str,
    unit: &str,
    frequency_hz: f64,
    display_decimals: u8,
) -> Result<(), String> {
    valid_text(name, usize::from(u16::MAX) - 1, "channel name", false)?;
    // The established reader replaces dots in displayed channel names.
    if name.contains('.') {
        return Err(
            "LD channel names containing dots cannot round-trip through the current reader".into(),
        );
    }
    valid_text(unit, 12, "channel unit", true)?;
    if !frequency_hz.is_finite()
        || !(1. ..=65535.).contains(&frequency_hz)
        || frequency_hz.fract() != 0.
    {
        return Err("LD channel frequencies must be integral 1–65535 Hz".into());
    }
    if display_decimals > 9 {
        return Err(
            "LD display precision above 9 decimals cannot round-trip through the current reader"
                .into(),
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum EnumEncoding {
    None,
    Legacy(usize),
    Private(usize),
}

fn enum_encoding(labels: &BTreeMap<i32, String>) -> Result<EnumEncoding, String> {
    if labels.is_empty() {
        return Ok(EnumEncoding::None);
    }
    if labels.len() > 500 {
        return Err("LD enum tables support at most 500 labels".into());
    }
    if labels
        .keys()
        .all(|value| (-2..=i32::from(i16::MAX)).contains(value))
    {
        return enum_size(labels).map(EnumEncoding::Legacy);
    }
    let mut size = 4usize;
    for label in labels.values() {
        valid_text(label, usize::from(u16::MAX), "private enum label", false)?;
        size = size
            .checked_add(6 + label.len())
            .filter(|size| *size <= usize::from(u16::MAX))
            .ok_or("Private signed-enum attribute exceeds its 65535-byte limit")?;
    }
    Ok(EnumEncoding::Private(size))
}

fn write_private_enum(bytes: &mut [u8], start: usize, labels: &BTreeMap<i32, String>) {
    put_u16(bytes, start, 1);
    put_u16(bytes, start + 2, labels.len() as u16);
    let mut at = start + 4;
    for (value, label) in labels {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put_u16(bytes, at + 4, label.len() as u16);
        bytes[at + 6..at + 6 + label.len()].copy_from_slice(label.as_bytes());
        at += 6 + label.len();
    }
}

fn enum_size(labels: &BTreeMap<i32, String>) -> Result<usize, String> {
    if labels.len() > 500 {
        return Err("LD enum tables support at most 500 labels".into());
    }
    let mut size = 6usize;
    for (value, label) in labels {
        // The established reader recognizes negative sentinels -1 and -2 only;
        // reject other negatives until their format semantics are established.
        if *value < -2 || *value > i32::from(i16::MAX) {
            return Err(format!(
                "Enum value {value} cannot round-trip through the current LD reader"
            ));
        }
        valid_text(label, 198, "enum label", false)?;
        size = size
            .checked_add(11 + label.len())
            .filter(|size| *size <= 20_000)
            .ok_or("LD enum table exceeds the reader's 20000-byte scan window")?;
    }
    Ok(size)
}

fn write_enum(bytes: &mut [u8], start: usize, id: u16, labels: &BTreeMap<i32, String>) {
    put_u16(bytes, start, labels.len() as u16);
    put_u16(bytes, start + 2, 2);
    put_u16(bytes, start + 4, id);
    let mut at = start + 6;
    for (value, label) in labels {
        put_u16(bytes, at, 3);
        put_u16(bytes, at + 6, (label.len() + 3) as u16);
        put_u16(bytes, at + 8, *value as i16 as u16);
        fixed_text(bytes, at + 10, label.len() + 1, label);
        at += 11 + label.len();
    }
}

fn fixed_text(bytes: &mut [u8], start: usize, width: usize, text: &str) {
    debug_assert!(text.len() <= width);
    bytes[start..start + width].fill(0);
    bytes[start..start + text.len()].copy_from_slice(text.as_bytes());
}

fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::super::telemetry::TimestampedSample;
    use super::*;
    use crate::{DataType, LdFile};

    fn session() -> Session {
        Session {
            date: "25/01/2024".into(),
            time: "18:04:56".into(),
            driver: "Driver".into(),
            vehicle_id: "Vehicle".into(),
            venue: "Test venue".into(),
            short_comment: "Native grid".into(),
            device_serial: 42,
            device_type: "M1".into(),
            device_version: 100,
            num_channels_header: 0,
        }
    }

    fn channel(name: &str, frequency_hz: f64, values: Vec<f64>) -> OutputChannel {
        OutputChannel {
            name: name.into(),
            unit: String::new(),
            frequency_hz,
            values,
            enum_labels: BTreeMap::new(),
            display_decimals: 3,
        }
    }

    #[test]
    fn round_trips_fresh_multifrequency_f64_grids_holes_and_u32_precision() {
        let mut fast = channel(
            "Long channel name preserving Unicode température and its complete suffix",
            2.,
            vec![0.25, f64::NAN, 4_294_967_295.],
        );
        fast.unit = "ratio".into();
        let slow = channel("Slow", 1., vec![-2., 1. / 3.]);
        let source = session();
        let bytes = encode_channels(&[fast.clone(), slow], &source, 100_000).unwrap();
        assert_eq!(&bytes[0x42..0x46], &1_000_000u32.to_le_bytes());
        let ld = LdFile::from_bytes(bytes).unwrap();
        assert_eq!(ld.channels.len(), 2);
        assert_eq!(ld.session.num_channels_header, 2);
        assert_eq!(ld.session.driver, source.driver);
        assert_eq!(ld.session.device_serial, source.device_serial);
        assert_eq!(ld.channels[0].name, fast.name);
        assert_eq!(ld.channels[0].unit, "ratio");
        assert_eq!(ld.channels[0].data_type, DataType::Float64);
        assert_eq!((ld.channels[0].freq, ld.channels[1].freq), (2., 1.));
        let values = ld.read_channel_data(&ld.channels[0]).unwrap();
        assert_eq!((values[0], values[2]), (0.25, 4_294_967_295.));
        assert!(values[1].is_nan());
        assert_eq!(
            ld.read_channel_data(&ld.channels[1]).unwrap(),
            [-2., 1. / 3.]
        );
    }

    #[test]
    fn round_trips_signed_sentinel_enum_labels_without_fallback_names() {
        let mut state = channel("Custom state", 1., vec![-2., -1., 0., 32767.]);
        state.display_decimals = 0;
        state.enum_labels = [
            (-2, "Unknown".into()),
            (-1, "Reverse".into()),
            (0, "Néutral".into()),
            (32767, "Upper state".into()),
        ]
        .into();
        let expected = state.enum_labels.clone();
        let ld =
            LdFile::from_bytes(encode_channels(&[state], &session(), 100_000).unwrap()).unwrap();
        let labels = &ld.channels[0].enum_labels;
        for (value, label) in expected {
            assert_eq!(labels.get(&i64::from(value)), Some(&label));
        }
        assert_eq!(
            ld.read_channel_data(&ld.channels[0]).unwrap(),
            [-2., -1., 0., 32767.]
        );
    }

    #[test]
    fn rejects_size_limits_fractional_clocks_and_unrepresentable_text_or_enums() {
        let valid = channel("Test", 1., vec![1.]);
        let encoded = encode_channels(std::slice::from_ref(&valid), &session(), 100_000).unwrap();
        assert!(
            encode_channels(std::slice::from_ref(&valid), &session(), encoded.len() - 1).is_err()
        );
        for frequency in [0., 1.5, 65_536., f64::NAN] {
            let mut invalid = valid.clone();
            invalid.frequency_hz = frequency;
            assert!(encode_channels(&[invalid], &session(), 100_000).is_err());
        }
        let mut invalid = valid.clone();
        invalid.unit = "1234567890123".into();
        assert!(encode_channels(&[invalid], &session(), 100_000).is_err());
        let mut invalid = valid.clone();
        invalid.enum_labels.insert(-3, "x".repeat(65_536));
        assert!(encode_channels(&[invalid], &session(), 100_000).is_err());
        let mut invalid = valid;
        invalid.name = " Name with whitespace ".into();
        assert!(encode_channels(&[invalid], &session(), 100_000).is_err());
    }

    #[test]
    fn round_trips_private_signed_i32_enum_attribute_without_legacy_table() {
        let mut channel = channel("Signed state", 1., vec![-4., f64::from(i32::MAX)]);
        channel.enum_labels = [
            (i32::MIN, "Lower".into()),
            (-4, "Bus fault".into()),
            (i32::MAX, "Upper".into()),
        ]
        .into();
        let bytes = encode_channels(std::slice::from_ref(&channel), &session(), 100_000).unwrap();
        assert!(bytes.windows(4).any(|window| window == b"i3rs"));
        let ld = LdFile::from_bytes(bytes).unwrap();
        for (value, label) in &channel.enum_labels {
            assert_eq!(
                ld.channels[0].enum_labels.get(&i64::from(*value)),
                Some(label)
            );
        }
    }

    fn timestamped(
        name: &str,
        data_type: ValueType,
        frequency_hz: f64,
        values: &[(f64, f64)],
    ) -> TelemetryChannel {
        TelemetryChannel {
            name: name.into(),
            unit: String::new(),
            base: 1,
            address: 0,
            data_type,
            enum_labels: BTreeMap::new(),
            display_decimals: 3,
            frequency_hz,
            samples: values
                .iter()
                .map(|(time_seconds, value)| TimestampedSample {
                    time_seconds: *time_seconds,
                    value: *value,
                    source_word: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn common_grid_preserves_phase_with_bounded_interpolation_hold_and_nan_gaps() {
        let fast = timestamped(
            "Continuous",
            ValueType::F32,
            2.,
            &[(0.1, 1.), (0.6, 6.), (1.1, 11.), (2.1, 21.)],
        );
        let integer = timestamped(
            "Counter",
            ValueType::U32,
            1.,
            &[(0.25, 4_294_967_295.), (1.25, 4_294_967_294.)],
        );
        let segment = DecodedSegment {
            index: 0,
            start_ticks: 0,
            end_ticks: 1,
            channels: vec![fast, integer],
        };
        let bytes = encode_segment(&segment, &session(), 100_000).unwrap();
        let ld = LdFile::from_bytes(bytes).unwrap();
        let fast = ld.read_channel_data(&ld.channels[0]).unwrap();
        assert_eq!(fast.len(), 5);
        assert!(fast[0].is_nan());
        assert!((fast[1] - 5.).abs() < 1e-12);
        assert!((fast[2] - 10.).abs() < 1e-12);
        assert!(fast[3].is_nan() && fast[4].is_nan());
        let integer = ld.read_channel_data(&ld.channels[1]).unwrap();
        assert_eq!(integer.len(), 3);
        assert!(integer[0].is_nan());
        assert_eq!(&integer[1..], &[4_294_967_295., 4_294_967_294.]);
    }

    #[test]
    fn gps_fix_boundaries_do_not_interpolate_but_single_zero_coordinates_do() {
        let latitude = timestamped(
            "GPS Latitude",
            ValueType::I32E7,
            2.,
            &[(0.25, 0.), (0.75, 0.), (1.25, 2.), (1.75, 0.), (2.25, 0.)],
        );
        let longitude = timestamped(
            "GPS Longitude",
            ValueType::I32E7,
            2.,
            &[(0.25, 0.), (0.75, 1.), (1.25, 0.), (1.75, 0.), (2.25, 0.)],
        );
        let segment = DecodedSegment {
            index: 0,
            start_ticks: 0,
            end_ticks: 1,
            channels: vec![latitude, longitude],
        };
        let bytes = encode_segment(&segment, &session(), 100_000).unwrap();
        let ld = LdFile::from_bytes(bytes).unwrap();
        let latitude = ld.read_channel_data(&ld.channels[0]).unwrap();
        let longitude = ld.read_channel_data(&ld.channels[1]).unwrap();
        assert!(latitude[0].is_nan() && longitude[0].is_nan());
        assert_eq!(&latitude[1..], &[0., 1., 2., 0.]);
        assert_eq!(&longitude[1..], &[0., 0.5, 0., 0.]);
    }

    #[test]
    fn resampling_rejects_invalid_timestamps_and_oversized_grids_before_allocation() {
        for times in [
            vec![(0., 1.), (0., 2.)],
            vec![(1., 1.), (0., 2.)],
            vec![(-1., 1.)],
            vec![(f64::NAN, 1.)],
            vec![(1e20, 1.)],
        ] {
            let source = timestamped("Source", ValueType::F32, 1., &times);
            let segment = DecodedSegment {
                index: 0,
                start_ticks: 0,
                end_ticks: 1,
                channels: vec![source],
            };
            assert!(encode_segment(&segment, &session(), 100_000).is_err());
        }
    }

    #[test]
    fn exact_matches_survive_nan_neighbors_and_state_hold_expires() {
        let channel = timestamped(
            "Continuous",
            ValueType::F32,
            10.,
            &[(0.2, f64::NAN), (0.3, 7.), (0.4, f64::NAN)],
        );
        let mut cursor = 0;
        assert_eq!(resampled_value(&channel, 0.1 + 0.2, &mut cursor, None), 7.);
        let state = timestamped("State", ValueType::Enum, 1., &[(0.2, -4.), (3., 1.)]);
        let mut cursor = 0;
        assert!(resampled_value(&state, 0., &mut cursor, None).is_nan());
        assert_eq!(resampled_value(&state, 1., &mut cursor, None), -4.);
        assert!(resampled_value(&state, 2., &mut cursor, None).is_nan());
    }
}

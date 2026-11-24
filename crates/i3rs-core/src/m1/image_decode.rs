//! Native decoding of the supported plain M1 logged-data-image layout.
//!
//! A setup's metadata GUID (`02010044`) selects its TuneConfig catalog; this
//! differs from the setup GUID used by the telemetry packet stream. Images
//! requiring different catalogs for different setups are explicitly unsupported.

use std::collections::{BTreeMap, BTreeSet};

use crate::Session;

use super::catalog::{Catalog, parse_catalog};
use super::log_image::{
    DEFAULT_METADATA_RECORD_LIMIT, MO_HEADER_SIZE, MetadataGroupContents, MetadataProperty,
    MoRecord, MoRecordIter, RawLogImage, STRING_RECORD_TYPE,
};
use super::telemetry::{
    DecodedTelemetry, TIMESTAMP_POLICY, TelemetryLimits, decode_stream_with_capture_start,
    parse_setups,
};
use super::transform::{XZ_RECORD_ENCODING, decode_xz_record};

const SETUPS: u32 = 0x0200_0020;
const SETUP: u32 = 0x0200_0022;
const STREAM: u32 = 0x0101_0002;
const METADATA_LIST: u32 = 0x0200_0046;
const METADATA: u32 = 0x0200_0047;
const METADATA_GUID: u32 = 0x0201_0044;
const CONFIG_XML: u32 = 0x0201_002c;
const VARIANT_VALUE: u32 = 0x0101_0008;
const MAX_SESSION_TEXT_BYTES: usize = 64 * 1024;
const MAX_LD_SESSION_FIELD_BYTES: usize = 64;
const MAX_INDIVIDUAL_UNIT_WARNINGS: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct ImageDecodeLimits {
    pub max_image_bytes: usize,
    /// Maximum full decoded Mo record size for the selected TuneConfig.
    pub max_decoded_bytes: usize,
    pub max_catalog_items: usize,
    pub max_xz_memory_kib: u32,
    pub telemetry: TelemetryLimits,
}

impl Default for ImageDecodeLimits {
    fn default() -> Self {
        Self {
            max_image_bytes: 512 * 1024 * 1024,
            max_decoded_bytes: 64 * 1024 * 1024,
            max_catalog_items: 20_000,
            max_xz_memory_kib: 64 * 1024,
            telemetry: TelemetryLimits::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub telemetry: DecodedTelemetry,
    pub session: Session,
    /// Zero-based logging systems with numeric packets but no decodable context.
    pub omitted_systems: Vec<u8>,
    /// Explicit limitations, including unvalidated permissions and clock fallback.
    pub warnings: Vec<String>,
}

/// Decode one supported uncompressed image, without writing any files.
///
/// Exactly one plain setup list, metadata list and binary stream are required.
/// Every setup must reference the same metadata GUID, and that GUID must have
/// exactly one matching catalog entry. Unrelated metadata entries are not used.
/// Image version 3 and the observed tag-2 little-endian scalar representation
/// are supported. Dates are left blank: retrieval time is not recording time.
/// A circular image can start mid-recording. Leading numeric packets are
/// checked for corruption and reported if skipped while waiting for context.
pub fn decode_image(bytes: &[u8], limits: &ImageDecodeLimits) -> Result<DecodedImage, String> {
    if bytes.len() > limits.max_image_bytes {
        return Err(format!(
            "M1 image has {} bytes, exceeding its {}-byte limit",
            bytes.len(),
            limits.max_image_bytes
        ));
    }
    validate_limits(limits)?;
    let image = RawLogImage::parse(bytes).map_err(|error| error.to_string())?;
    if image.header.encoding != 0 {
        return Err("transformed M1 image roots are unsupported".into());
    }
    let (session, session_warnings) = session_metadata(&image, limits.telemetry.max_records)?;
    let mut budget = HeaderBudget(limits.telemetry.max_records);
    budget.take()?; // Outer image header.
    let mut setup_record = None;
    let mut stream_record = None;
    let mut metadata_record = None;
    let mut unrelated_roots = 0usize;
    for record in records(image.payload, &mut budget)? {
        match record.header.record_type {
            SETUPS => unique(&mut setup_record, record, "logging setup list")?,
            STREAM => unique(&mut stream_record, record, "binary logging stream")?,
            METADATA_LIST => unique(&mut metadata_record, record, "logging metadata list")?,
            0x0100_0009 | 0x0100_000e | 0x0201_0012 => {}
            _ => unrelated_roots += 1,
        }
    }
    let setup_record = setup_record.ok_or("M1 image has no logging setup list")?;
    let stream_record = stream_record.ok_or("M1 image has no binary logging stream")?;
    let metadata_record = metadata_record.ok_or("M1 image has no logging metadata list")?;
    let mut decoded = decode_records(
        setup_record,
        metadata_record,
        stream_record,
        session,
        limits,
        &mut budget,
        true,
        false,
    )?;
    decoded.warnings.extend(session_warnings);
    if unrelated_roots != 0 {
        decoded.warnings.push(format!(
            "{unrelated_roots} unrecognized root record(s) were left uninterpreted."
        ));
    }
    Ok(decoded)
}

/// Decode separately acquired setup, metadata and binary-stream records.
///
/// This shares the image decoder's GUID/catalog/telemetry checks without
/// asserting that the source is an official `.m1dli`. A capture adapter may
/// borrow its bounded joined sectors as the plain stream record's payload.
/// All three complete record spans together must fit `max_image_bytes`.
/// Supplied session fields are caller metadata; this does not interpret any
/// retrieval timestamp as a recording date or validate image permissions.
pub fn decode_parts(
    setups: MoRecord<'_>,
    metadata: MoRecord<'_>,
    stream: MoRecord<'_>,
    session: Session,
    limits: &ImageDecodeLimits,
) -> Result<DecodedImage, String> {
    decode_parts_with_capture_start(setups, metadata, stream, session, limits, false, false)
}

/// As [`decode_parts`], while allowing circular-buffer streams that begin
/// mid-recording before the first setup and high-timestamp markers.
pub fn decode_parts_allowing_mid_stream(
    setups: MoRecord<'_>,
    metadata: MoRecord<'_>,
    stream: MoRecord<'_>,
    session: Session,
    limits: &ImageDecodeLimits,
) -> Result<DecodedImage, String> {
    decode_parts_with_capture_start(setups, metadata, stream, session, limits, true, false)
}

pub(super) fn decode_parts_with_capture_start(
    setups: MoRecord<'_>,
    metadata: MoRecord<'_>,
    stream: MoRecord<'_>,
    session: Session,
    limits: &ImageDecodeLimits,
    allow_mid_stream_start: bool,
    from_raw_capture: bool,
) -> Result<DecodedImage, String> {
    validate_limits(limits)?;
    let mut total = 0usize;
    for record in [setups, metadata, stream] {
        total = total
            .checked_add(MO_HEADER_SIZE)
            .and_then(|length| length.checked_add(record.payload.len()))
            .filter(|length| *length <= limits.max_image_bytes)
            .ok_or("M1 record parts exceed the combined input byte limit")?;
    }
    for text in [
        &session.date,
        &session.time,
        &session.driver,
        &session.vehicle_id,
        &session.venue,
        &session.short_comment,
        &session.device_type,
    ] {
        if text.len() > MAX_SESSION_TEXT_BYTES || text.contains('\0') {
            return Err("M1 session metadata exceeds its text limit or contains NUL".into());
        }
    }
    let mut budget = HeaderBudget(limits.telemetry.max_records);
    for _ in 0..3 {
        budget.take()?;
    }
    decode_records(
        setups,
        metadata,
        stream,
        session,
        limits,
        &mut budget,
        allow_mid_stream_start,
        from_raw_capture,
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_records(
    setup_record: MoRecord<'_>,
    metadata_record: MoRecord<'_>,
    stream_record: MoRecord<'_>,
    session: Session,
    limits: &ImageDecodeLimits,
    budget: &mut HeaderBudget,
    allow_mid_stream_start: bool,
    from_raw_capture: bool,
) -> Result<DecodedImage, String> {
    if setup_record.header.record_type != SETUPS
        || metadata_record.header.record_type != METADATA_LIST
        || stream_record.header.record_type != STREAM
    {
        return Err(
            "M1 parts require setup-list, metadata-list and binary-stream record types".into(),
        );
    }
    plain(setup_record, "logging setup list")?;
    plain(stream_record, "binary logging stream")?;
    plain(metadata_record, "logging metadata list")?;
    let required_guid = setup_metadata_guid(setup_record, budget)?;
    let entries = metadata_entries(metadata_record, budget)?;
    let config = entries
        .get(&required_guid)
        .ok_or("logging setups have no matching metadata GUID in the image")?;
    if config.header.encoding != XZ_RECORD_ENCODING || config.header.flags != 0 {
        return Err(
            "logging metadata XML requires the supported encoding-31/index-0 layout".into(),
        );
    }
    // Validate packet layouts before allocating or decompressing their catalog.
    let setups = parse_setups(setup_record, limits.telemetry).map_err(|error| error.to_string())?;
    let max_xml_record_bytes = limits
        .max_decoded_bytes
        .min(super::catalog::MAX_XML_BYTES + MO_HEADER_SIZE);
    let decoded_config = decode_xz_record(config, max_xml_record_bytes, limits.max_xz_memory_kib)?;
    let catalog = parse_catalog(&decoded_config[MO_HEADER_SIZE..], limits.max_catalog_items)?;
    let (telemetry, skipped, omitted_systems) = decode_stream_with_capture_start(
        stream_record,
        &setups,
        &catalog,
        limits.telemetry,
        allow_mid_stream_start,
    )
    .map_err(|error| error.to_string())?;
    let mut warnings = Vec::new();
    if !from_raw_capture {
        warnings.push("Image enable/permission records remain opaque; conversion permissions and image signatures were not validated.".into());
        warnings.push("Recording calendar timestamps are not interpreted; source retrieval time is not used as recording time.".into());
    }
    warnings.push(TIMESTAMP_POLICY.into());
    for (system, count) in skipped.into_iter().enumerate() {
        if count != 0 {
            warnings.push(format!(
                "Logging system {} omitted {count} leading numeric packets before the logging setup and high-timestamp markers were both available.",
                system + 1
            ));
        }
    }
    for &system in &omitted_systems {
        warnings.push(format!(
            "Logging system {} had only leading numeric packets without setup and high-timestamp markers and was omitted.",
            system + 1
        ));
    }
    append_catalog_warnings(&catalog, &telemetry, &mut warnings);
    if telemetry.used_default_clock {
        warnings.push(
            "Using Tune's 125 MHz fallback clock; the board-specific clock has not been established."
                .into(),
        );
    }
    if entries.len() > 1 {
        warnings.push("Unreferenced logging metadata catalogs were not decoded.".into());
    }
    Ok(DecodedImage {
        telemetry,
        session,
        omitted_systems,
        warnings,
    })
}

fn append_catalog_warnings(
    catalog: &Catalog,
    telemetry: &DecodedTelemetry,
    warnings: &mut Vec<String>,
) {
    let selected: BTreeSet<_> = telemetry
        .systems
        .iter()
        .flat_map(|system| &system.segments)
        .flat_map(|segment| &segment.channels)
        .map(|channel| (channel.base, channel.address))
        .collect();
    let matched: Vec<_> = selected
        .iter()
        .filter_map(|address| catalog.unit_conversion_warnings.get(address))
        .collect();
    for warning in matched.iter().take(MAX_INDIVIDUAL_UNIT_WARNINGS) {
        warnings.push((*warning).clone());
    }
    if matched.len() > MAX_INDIVIDUAL_UNIT_WARNINGS {
        warnings.push(format!(
            "And {} additional channel(s) retained physical units due to unsupported display conversions.",
            matched.len() - MAX_INDIVIDUAL_UNIT_WARNINGS
        ));
    }
}

fn validate_limits(limits: &ImageDecodeLimits) -> Result<(), String> {
    if limits.max_decoded_bytes < MO_HEADER_SIZE
        || limits.max_catalog_items == 0
        || limits.telemetry.max_records == 0
        || limits.max_xz_memory_kib == 0
        || limits.max_xz_memory_kib == u32::MAX
    {
        return Err("invalid M1 image decode limits".into());
    }
    Ok(())
}

fn unique<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("M1 image has duplicate {name}"));
    }
    Ok(())
}

fn plain(record: MoRecord<'_>, name: &str) -> Result<(), String> {
    if usize::try_from(record.header.payload_len).ok() != Some(record.payload.len()) {
        return Err(format!(
            "{name} payload length does not match its supplied span"
        ));
    }
    if record.header.encoding != 0 {
        return Err(format!("transformed {name} is unsupported"));
    }
    Ok(())
}

struct HeaderBudget(usize);
impl HeaderBudget {
    fn take(&mut self) -> Result<(), String> {
        self.0 = self.0.checked_sub(1).ok_or("image record limit exceeded")?;
        Ok(())
    }
}

fn records<'a>(bytes: &'a [u8], budget: &mut HeaderBudget) -> Result<Vec<MoRecord<'a>>, String> {
    let mut result = Vec::new();
    for record in MoRecordIter::new(bytes, budget.0) {
        let record = record.map_err(|error| error.to_string())?;
        budget.take()?;
        result.push(record);
    }
    Ok(result)
}

fn guid(record: MoRecord<'_>) -> Result<[u8; 16], String> {
    plain(record, "logging metadata GUID")?;
    if record.header.flags != 0 {
        return Err("indexed logging metadata GUIDs are unsupported".into());
    }
    record
        .payload
        .try_into()
        .map_err(|_| "logging metadata GUID must contain exactly 16 bytes".into())
}

fn setup_metadata_guid(
    record: MoRecord<'_>,
    budget: &mut HeaderBudget,
) -> Result<[u8; 16], String> {
    let mut required = None;
    for setup in records(record.payload, budget)? {
        plain(setup, "logging setup")?;
        if setup.header.record_type != SETUP {
            return Err("unexpected record in logging setup list".into());
        }
        let mut setup_guid = None;
        for field in records(setup.payload, budget)? {
            if field.header.record_type == METADATA_GUID
                && setup_guid.replace(guid(field)?).is_some()
            {
                return Err("logging setup has duplicate metadata GUID records".into());
            }
        }
        let setup_guid = setup_guid.ok_or("logging setup has no metadata GUID")?;
        if required.is_some_and(|previous| previous != setup_guid) {
            return Err(
                "multiple setup metadata GUIDs require catalog switching, which is unsupported"
                    .into(),
            );
        }
        required = Some(setup_guid);
    }
    required.ok_or_else(|| "M1 image has no logging setups".into())
}

fn metadata_entries<'a>(
    record: MoRecord<'a>,
    budget: &mut HeaderBudget,
) -> Result<BTreeMap<[u8; 16], MoRecord<'a>>, String> {
    let mut result = BTreeMap::new();
    for metadata in records(record.payload, budget)? {
        plain(metadata, "logging metadata entry")?;
        if metadata.header.record_type != METADATA {
            return Err("unexpected record in logging metadata list".into());
        }
        let mut entry_guid = None;
        let mut xml = None;
        for field in records(metadata.payload, budget)? {
            match field.header.record_type {
                METADATA_GUID => {
                    if entry_guid.replace(guid(field)?).is_some() {
                        return Err("logging metadata entry has duplicate GUID records".into());
                    }
                }
                CONFIG_XML => {
                    unique(&mut xml, field, "logging metadata XML")?;
                }
                _ => return Err("unexpected record in logging metadata entry".into()),
            }
        }
        let entry_guid = entry_guid.ok_or("logging metadata entry has no GUID")?;
        let xml = xml.ok_or("logging metadata entry has no TuneConfig XML")?;
        if result.insert(entry_guid, xml).is_some() {
            return Err("duplicate logging metadata catalog GUID".into());
        }
    }
    Ok(result)
}

fn truncate_session_field(text: &str, field: &str, warnings: &mut Vec<String>) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= MAX_LD_SESSION_FIELD_BYTES {
        return trimmed.to_owned();
    }
    let mut end = MAX_LD_SESSION_FIELD_BYTES;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    warnings.push(format!(
        "Image session field {field} exceeded {MAX_LD_SESSION_FIELD_BYTES} bytes and was truncated for LD header compatibility."
    ));
    trimmed[..end].trim_end().to_owned()
}

fn session_metadata(
    image: &RawLogImage<'_>,
    max_records: usize,
) -> Result<(Session, Vec<String>), String> {
    let metadata = image
        .inspect_metadata_with_limit(max_records.min(DEFAULT_METADATA_RECORD_LIMIT))
        .map_err(|error| error.to_string())?;
    let mut version = None;
    let mut serial = None;
    let mut strings = BTreeMap::new();
    let mut warnings = Vec::new();
    for group in &metadata.groups {
        if !matches!(group.record.header.flags, 0 | 1) {
            continue;
        }
        let MetadataGroupContents::Plain { properties, .. } = &group.contents else {
            return Err("transformed image session properties are unsupported".into());
        };
        for property in properties {
            let Some(name) = property.name() else {
                continue;
            };
            if group.record.header.flags == 0 {
                match name {
                    b"version" => unique(
                        &mut version,
                        variant_u32(property, "image version")?,
                        "image version property",
                    )?,
                    b"device.serial" => unique(
                        &mut serial,
                        variant_u32(property, "device serial")?,
                        "device serial property",
                    )?,
                    _ => {}
                }
            } else if matches!(
                name,
                b"Driver" | b"Venue" | b"Vehicle Id" | b"Short Comment"
            ) {
                let value = property_value(property)?;
                if value.header.record_type != STRING_RECORD_TYPE || value.header.encoding != 0 {
                    return Err("image session text must be a plain string record".into());
                }
                if value.payload.len() > MAX_SESSION_TEXT_BYTES || value.payload.contains(&0) {
                    return Err(
                        "image session text is oversized or contains an embedded NUL".into(),
                    );
                }
                let text = std::str::from_utf8(value.payload)
                    .map_err(|_| "image session text is not UTF-8")?;
                let field_name = std::str::from_utf8(name).unwrap_or("Session");
                let cleaned = truncate_session_field(text, field_name, &mut warnings);
                if strings.insert(name, cleaned).is_some() {
                    return Err("duplicate image session text property".into());
                }
            }
        }
    }
    if version != Some(3) {
        return Err(format!(
            "unsupported M1 image version {version:?}; expected version 3"
        ));
    }
    Ok((
        Session {
            date: String::new(),
            time: String::new(),
            driver: strings.remove(b"Driver".as_slice()).unwrap_or_default(),
            vehicle_id: strings.remove(b"Vehicle Id".as_slice()).unwrap_or_default(),
            venue: strings.remove(b"Venue".as_slice()).unwrap_or_default(),
            short_comment: strings
                .remove(b"Short Comment".as_slice())
                .unwrap_or_default(),
            device_serial: serial.unwrap_or_default(),
            device_type: "M1".into(),
            device_version: 0, // Image format version is not ECU firmware version.
            num_channels_header: 0, // Each exported segment determines its own count.
        },
        warnings,
    ))
}

fn property_value<'a>(property: &MetadataProperty<'a>) -> Result<MoRecord<'a>, String> {
    if property.field_records().count() != 2 {
        return Err("image session property must contain exactly a name and a value".into());
    }
    property
        .value()
        .copied()
        .ok_or_else(|| "image session property has no value".into())
}

fn variant_u32(property: &MetadataProperty<'_>, name: &str) -> Result<u32, String> {
    let value = property_value(property)?;
    if value.header.record_type != VARIANT_VALUE
        || value.header.encoding != 0
        || value.payload.len() != 5
        || value.payload[0] != 2
    {
        return Err(format!("unsupported {name} scalar representation"));
    }
    Ok(u32::from_le_bytes(value.payload[1..5].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::super::log_image::{
        LOG_IMAGE_RECORD_TYPE, PROPERTIES_RECORD_TYPE, PROPERTY_RECORD_TYPE,
    };
    use super::*;

    // Synthetic checked XZ: plain0201002c with one f32 Engine Speed channel
    // at1:0x100, unit rpm, logging event0 at1Hz. No private fixture bytes.
    const CATALOG_XZ: &[u8] = &[
        0x00, 0x00, 0x01, 0x36, 0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01, 0x69, 0x22, 0xde,
        0x36, 0x02, 0x00, 0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x37, 0x27, 0x97, 0xd6, 0xe0, 0x01,
        0x35, 0x00, 0xd4, 0x5d, 0x00, 0x26, 0x9b, 0xbc, 0x18, 0x02, 0x00, 0x7f, 0xe7, 0x63, 0x9c,
        0x9f, 0xd7, 0x83, 0x6c, 0x7a, 0x46, 0xc6, 0xd3, 0xb2, 0x52, 0x6f, 0x13, 0x4d, 0x47, 0x58,
        0x66, 0xf9, 0xce, 0xa8, 0xca, 0x57, 0x1c, 0xc7, 0x01, 0x5b, 0x53, 0x17, 0x18, 0x61, 0x5a,
        0x70, 0x77, 0x36, 0x82, 0x96, 0x4d, 0xf3, 0xa0, 0x39, 0x03, 0xdd, 0xe8, 0xed, 0xa3, 0x6c,
        0xca, 0x7b, 0x35, 0x66, 0xbd, 0xc3, 0xbc, 0x48, 0x3f, 0x0a, 0xbd, 0xc0, 0xa2, 0x81, 0x9b,
        0x87, 0x68, 0x19, 0x12, 0xdc, 0xed, 0x57, 0xd2, 0x5b, 0x83, 0x7a, 0x25, 0x65, 0xe0, 0xad,
        0x5c, 0xbe, 0xe2, 0x13, 0xca, 0x81, 0xe5, 0x6a, 0x20, 0xb5, 0xc0, 0xb0, 0xec, 0xa7, 0x88,
        0x1e, 0x34, 0x32, 0x48, 0x01, 0xb8, 0x55, 0x0a, 0x05, 0xcd, 0x53, 0xb9, 0x14, 0x06, 0x72,
        0x98, 0xc1, 0xab, 0x3b, 0xaf, 0x26, 0xfe, 0x03, 0xdc, 0xfa, 0x91, 0xa9, 0x90, 0x9e, 0x97,
        0xff, 0x13, 0xe2, 0x47, 0x4e, 0xcc, 0xec, 0x8e, 0xba, 0x83, 0xb4, 0xed, 0x27, 0x0c, 0xfd,
        0x71, 0x90, 0x39, 0xd6, 0x69, 0x0a, 0xc9, 0x0b, 0xcd, 0xdc, 0xc9, 0x9e, 0xd1, 0x5b, 0x8e,
        0xe9, 0xdb, 0x20, 0x35, 0x5b, 0x5c, 0xf2, 0xec, 0xe1, 0xf9, 0x4d, 0x2d, 0xf1, 0xa3, 0x5c,
        0x13, 0x91, 0x22, 0xc1, 0x91, 0x5a, 0xae, 0x6f, 0xb6, 0x54, 0xf2, 0xa5, 0x17, 0x55, 0x39,
        0x80, 0x78, 0xe5, 0x91, 0x43, 0xe7, 0xc4, 0xf5, 0x1b, 0xd0, 0xd9, 0x87, 0xbc, 0xde, 0x0e,
        0x6c, 0xde, 0x77, 0xa9, 0xbb, 0x57, 0x20, 0x00, 0xf5, 0xa9, 0xb9, 0x63, 0x00, 0x01, 0xec,
        0x01, 0xb6, 0x02, 0x00, 0x00, 0xbc, 0xe3, 0x2e, 0xa4, 0x3e, 0x30, 0x0d, 0x8b, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x01, 0x59, 0x5a,
    ];

    fn record(kind: u32, flags: u8, encoding: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"Mo".to_vec();
        bytes.extend_from_slice(&[flags, encoding]);
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    fn joined(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            bytes.extend_from_slice(record);
        }
        bytes
    }
    fn property(name: &[u8], kind: u32, payload: &[u8]) -> Vec<u8> {
        record(
            PROPERTY_RECORD_TYPE,
            0,
            0,
            &joined(&[
                record(STRING_RECORD_TYPE, 0, 0, name),
                record(kind, 0, 0, payload),
            ]),
        )
    }
    fn properties() -> Vec<Vec<u8>> {
        vec![
            record(
                PROPERTIES_RECORD_TYPE,
                0,
                0,
                &joined(&[
                    property(b"version", VARIANT_VALUE, &[2, 3, 0, 0, 0]),
                    property(b"device.serial", VARIANT_VALUE, &[2, 42, 0, 0, 0]),
                ]),
            ),
            record(
                PROPERTIES_RECORD_TYPE,
                1,
                0,
                &joined(&[
                    property(b"Driver", STRING_RECORD_TYPE, "Test Driver Ω".as_bytes()),
                    property(b"Vehicle Id", STRING_RECORD_TYPE, b"Test Vehicle"),
                    property(b"Venue", STRING_RECORD_TYPE, b"Test Circuit"),
                    property(b"Short Comment", STRING_RECORD_TYPE, b"Synthetic"),
                ]),
            ),
        ]
    }
    fn setup(system: u8, metadata_guid: [u8; 16]) -> Vec<u8> {
        let mut info = vec![0; 16];
        info.extend_from_slice(&[1; 16]);
        let group_info = [1u32, 7816, 0, 1, 4]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        let entry = [1u32, 0x100, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        record(
            SETUP,
            system,
            0,
            &joined(&[
                record(0x0201_0023, 0, 0, &info),
                record(METADATA_GUID, 0, 0, &metadata_guid),
                record(
                    0x0200_0024,
                    0,
                    0,
                    &joined(&[
                        record(0x0201_0025, 0, 0, &group_info),
                        record(0x0201_0026, 0, 0, &entry),
                    ]),
                ),
            ]),
        )
    }
    fn metadata(guid: [u8; 16], payload: &[u8]) -> Vec<u8> {
        record(
            METADATA,
            0,
            0,
            &joined(&[
                record(METADATA_GUID, 0, 0, &guid),
                record(CONFIG_XML, 0, XZ_RECORD_ENCODING, payload),
            ]),
        )
    }
    fn numeric_packet(ticks: u32, value: f32) -> Vec<u8> {
        let mut payload = ticks.to_be_bytes().to_vec();
        payload.extend_from_slice(&value.to_bits().to_be_bytes());
        payload.extend_from_slice(&crc32fast::hash(&payload).to_be_bytes());
        record(0x0201_002b, 0, 0, &payload)
    }
    fn stream(leading_packet: Option<Vec<u8>>) -> Vec<u8> {
        let mut packets = vec![record(0x0201_004d, 0, 0, &0u32.to_be_bytes())];
        packets.extend(leading_packet);
        packets.extend([
            record(0x0201_0039, 0, 0, &[1; 16]),
            record(0x0201_0029, 0, 0, &0u32.to_be_bytes()),
            numeric_packet(0, 1000.),
            numeric_packet(125_000_000, 2000.),
        ]);
        record(STREAM, 0, 0, &record(0x0200_004c, 0, 0, &joined(&packets)))
    }
    fn image(mut children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut roots = properties();
        roots.append(&mut children);
        record(LOG_IMAGE_RECORD_TYPE, 0, 0, &joined(&roots))
    }
    fn data_records() -> Vec<Vec<u8>> {
        vec![
            record(SETUPS, 0, 0, &setup(0, [2; 16])),
            record(METADATA_LIST, 0, 0, &metadata([2; 16], CATALOG_XZ)),
            stream(None),
            record(0x0201_0012, 0, 10, b"opaque enables"),
        ]
    }

    #[test]
    fn synthetic_image_preserves_samples_session_and_explicit_limits() {
        let decoded = decode_image(&image(data_records()), &ImageDecodeLimits::default()).unwrap();
        assert_eq!(decoded.session.driver, "Test Driver Ω");
        assert_eq!(decoded.session.vehicle_id, "Test Vehicle");
        assert_eq!(decoded.session.venue, "Test Circuit");
        assert_eq!(decoded.session.short_comment, "Synthetic");
        assert_eq!(decoded.session.device_serial, 42);
        assert_eq!(decoded.session.device_version, 0);
        assert!(decoded.session.date.is_empty() && decoded.session.time.is_empty());
        assert_eq!(decoded.telemetry.systems.len(), 1);
        let segment = &decoded.telemetry.systems[0].segments[0];
        assert_eq!(segment.channels.len(), 1);
        let channel = &segment.channels[0];
        assert_eq!(channel.name, "Engine Speed");
        assert_eq!(channel.unit, "rpm");
        assert_eq!(
            channel.samples.iter().map(|s| s.value).collect::<Vec<_>>(),
            [1000., 2000.]
        );
        assert_eq!(channel.samples[1].time_seconds, 1.);
        assert!(decoded.telemetry.used_default_clock);
        assert!(decoded.warnings.iter().any(|w| w.contains("125 MHz")));
        assert!(
            decoded
                .warnings
                .iter()
                .any(|w| w.contains("permissions") && w.contains("not validated"))
        );
    }

    #[test]
    fn circular_image_recovers_leading_packets_only_after_crc_validation() {
        let mut records = data_records();
        let mut leading = numeric_packet(0, 42.);
        records[2] = stream(Some(leading.clone()));
        let limits = ImageDecodeLimits::default();
        let decoded = decode_image(&image(records.clone()), &limits).unwrap();
        let samples = &decoded.telemetry.systems[0].segments[0].channels[0].samples;
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].value, 1000.);
        assert!(
            decoded
                .warnings
                .iter()
                .any(|w| w.contains("omitted 1 leading numeric packets"))
        );

        *leading.last_mut().unwrap() ^= 1;
        records[2] = stream(Some(leading));
        assert!(
            decode_image(&image(records), &limits)
                .unwrap_err()
                .contains("CRC")
        );
    }

    #[test]
    fn selects_exact_metadata_guid_without_decoding_unreferenced_first_entry() {
        let mut records = data_records();
        records[1] = record(
            METADATA_LIST,
            0,
            0,
            &joined(&[
                metadata([0; 16], b"deliberately invalid but unreferenced XZ"),
                metadata([2; 16], CATALOG_XZ),
            ]),
        );
        let decoded = decode_image(&image(records), &ImageDecodeLimits::default()).unwrap();
        assert_eq!(
            decoded.telemetry.systems[0].segments[0].channels[0].name,
            "Engine Speed"
        );
        assert!(decoded.warnings.iter().any(|w| w.contains("Unreferenced")));
    }

    #[test]
    fn separately_acquired_parts_share_decoder_and_validate_spans_and_combined_cap() {
        let roots = data_records();
        let borrowed = |bytes| MoRecordIter::new(bytes, 1).next().unwrap().unwrap();
        let setups = borrowed(roots[0].as_slice());
        let metadata = borrowed(roots[1].as_slice());
        let stream = borrowed(roots[2].as_slice());
        let bytes = image(vec![]);
        let (session, _) = session_metadata(&RawLogImage::parse(&bytes).unwrap(), 100).unwrap();
        let limits = ImageDecodeLimits::default();
        let decoded = decode_parts(setups, metadata, stream, session.clone(), &limits).unwrap();
        assert_eq!(
            decoded.telemetry.systems[0].segments[0].channels[0]
                .samples
                .len(),
            2
        );
        let mut malformed = stream;
        malformed.header.payload_len += 1;
        assert!(
            decode_parts(setups, metadata, malformed, session.clone(), &limits)
                .unwrap_err()
                .contains("supplied span")
        );
        malformed = stream;
        malformed.header.record_type = SETUPS;
        assert!(
            decode_parts(setups, metadata, malformed, session.clone(), &limits)
                .unwrap_err()
                .contains("record types")
        );
        let limits = ImageDecodeLimits {
            max_image_bytes: roots[0].len() + roots[1].len() + roots[2].len() - 1,
            ..limits
        };
        assert!(
            decode_parts(setups, metadata, stream, session, &limits)
                .unwrap_err()
                .contains("combined input")
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_multiple_metadata_associations() {
        let mut missing = data_records();
        missing[1] = record(METADATA_LIST, 0, 0, &metadata([3; 16], CATALOG_XZ));
        assert!(
            decode_image(&image(missing), &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("matching metadata GUID")
        );
        let mut duplicate = data_records();
        duplicate[1] = record(
            METADATA_LIST,
            0,
            0,
            &joined(&[metadata([2; 16], CATALOG_XZ), metadata([2; 16], CATALOG_XZ)]),
        );
        assert!(
            decode_image(&image(duplicate), &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("duplicate logging metadata")
        );
        let mut multiple = data_records();
        multiple[0] = record(
            SETUPS,
            0,
            0,
            &joined(&[setup(0, [2; 16]), setup(1, [3; 16])]),
        );
        assert!(
            decode_image(&image(multiple), &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("catalog switching")
        );
    }

    #[test]
    fn bounded_image_catalog_record_and_decoded_output_limits() {
        let bytes = image(data_records());
        let mut limits = ImageDecodeLimits {
            max_image_bytes: bytes.len() - 1,
            ..ImageDecodeLimits::default()
        };
        assert!(
            decode_image(&bytes, &limits)
                .unwrap_err()
                .contains("byte limit")
        );
        limits = ImageDecodeLimits::default();
        limits.max_decoded_bytes = 12;
        assert!(
            decode_image(&bytes, &limits)
                .unwrap_err()
                .contains("output limit")
        );
        limits = ImageDecodeLimits::default();
        limits.telemetry.max_records = 2;
        assert!(decode_image(&bytes, &limits).is_err());
        limits = ImageDecodeLimits::default();
        limits.max_catalog_items = 1;
        assert!(
            decode_image(&bytes, &limits)
                .unwrap_err()
                .contains("budget")
        );
    }

    #[test]
    fn rejects_duplicate_transformed_or_missing_required_root_records() {
        for index in 0..3 {
            let mut roots = data_records();
            roots.push(roots[index].clone());
            assert!(
                decode_image(&image(roots), &ImageDecodeLimits::default())
                    .unwrap_err()
                    .contains("duplicate")
            );
            let mut roots = data_records();
            roots[index][3] = 10;
            assert!(
                decode_image(&image(roots), &ImageDecodeLimits::default())
                    .unwrap_err()
                    .contains("transformed")
            );
            let mut roots = data_records();
            roots.remove(index);
            assert!(
                decode_image(&image(roots), &ImageDecodeLimits::default())
                    .unwrap_err()
                    .contains("has no")
            );
        }
        let mut bytes = image(data_records());
        bytes[3] = 31;
        assert!(
            decode_image(&bytes, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("roots")
        );
    }

    #[test]
    fn session_properties_reject_unknown_versions_duplicate_and_invalid_text() {
        let roots = properties();
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &joined(&roots));
        let parsed = RawLogImage::parse(&bytes).unwrap();
        assert_eq!(session_metadata(&parsed, 100).unwrap().0.device_serial, 42);
        for payload in [vec![2, 4, 0, 0, 0], vec![1, 3, 0, 0, 0]] {
            let group = record(
                PROPERTIES_RECORD_TYPE,
                0,
                0,
                &property(b"version", VARIANT_VALUE, &payload),
            );
            let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &group);
            assert!(session_metadata(&RawLogImage::parse(&bytes).unwrap(), 100).is_err());
        }
        for value in [vec![0xff], vec![0], vec![b'x'; MAX_SESSION_TEXT_BYTES + 1]] {
            let mut roots = properties();
            roots[1] = record(
                PROPERTIES_RECORD_TYPE,
                1,
                0,
                &property(b"Driver", STRING_RECORD_TYPE, &value),
            );
            let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &joined(&roots));
            assert!(session_metadata(&RawLogImage::parse(&bytes).unwrap(), 100).is_err());
        }
        let mut roots = properties();
        roots.push(roots[1].clone());
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &joined(&roots));
        assert!(
            session_metadata(&RawLogImage::parse(&bytes).unwrap(), 100)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn unit_warnings_only_include_selected_addresses_once() {
        let mut decoded =
            decode_image(&image(data_records()), &ImageDecodeLimits::default()).unwrap();
        let channel = &decoded.telemetry.systems[0].segments[0].channels[0];
        let address = (channel.base, channel.address);
        let catalog = Catalog {
            unit_conversion_warnings: BTreeMap::from([
                (address, "Selected physical unit retained".into()),
                ((99, 99), "Unused physical unit retained".into()),
            ]),
            ..Catalog::default()
        };
        let repeated = decoded.telemetry.systems[0].segments[0].clone();
        decoded.telemetry.systems[0].segments.push(repeated);
        let mut warnings = Vec::new();
        append_catalog_warnings(&catalog, &decoded.telemetry, &mut warnings);
        assert_eq!(warnings, ["Selected physical unit retained"]);
        assert!(
            decoded
                .warnings
                .iter()
                .any(|warning| warning == TIMESTAMP_POLICY)
        );

        // Many unsupported display unit warnings are summarized so conversion
        // manifests stay within their warning-count bound.
        let mut many_warnings = BTreeMap::new();
        let mut many_channels = Vec::new();
        for index in 0..150u32 {
            many_warnings.insert((1, index), format!("Channel {index}: retained"));
            many_channels.push(super::super::telemetry::TelemetryChannel {
                data_type: super::super::catalog::ValueType::F32,
                base: 1,
                address: index,
                name: format!("Ch {index}"),
                unit: String::new(),
                enum_labels: Default::default(),
                display_decimals: 1,
                frequency_hz: 1.,
                samples: Vec::new(),
            });
        }
        let catalog = Catalog {
            unit_conversion_warnings: many_warnings,
            ..Catalog::default()
        };
        let telemetry = DecodedTelemetry {
            systems: vec![super::super::telemetry::DecodedSystem {
                system_id: 0,
                segments: vec![super::super::telemetry::DecodedSegment {
                    index: 0,
                    start_ticks: 0,
                    end_ticks: 0,
                    channels: many_channels,
                }],
            }],
            clock_hz: 125_000_000.,
            used_default_clock: true,
        };
        let mut summarized = Vec::new();
        append_catalog_warnings(&catalog, &telemetry, &mut summarized);
        assert_eq!(summarized.len(), MAX_INDIVIDUAL_UNIT_WARNINGS + 1);
        assert!(summarized.last().unwrap().contains("118 additional"));
    }

    #[test]
    fn local_numeric_image_if_requested() {
        let Some(path) = std::env::var_os("I3RS_M1_TELEMETRY_FIXTURE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let decoded = decode_image(&bytes, &ImageDecodeLimits::default()).unwrap();
        assert_eq!(decoded.telemetry.systems.len(), 1);
        let system = &decoded.telemetry.systems[0];
        assert_eq!(system.segments.len(), 4);
        assert!(
            system
                .segments
                .iter()
                .all(|segment| segment.channels.len() == 196)
        );
        assert_eq!(
            system
                .segments
                .iter()
                .flat_map(|s| &s.channels)
                .map(|c| c.samples.len())
                .sum::<usize>(),
            595_297
        );
        let channels = &system.segments[0].channels;
        let engine = channels
            .iter()
            .find(|channel| channel.name == "Engine Speed")
            .unwrap();
        assert_eq!(engine.unit, "rpm");
        assert!((engine.samples[0].value - 1041.0945638020833).abs() < 1e-6);
        let throttle = channels
            .iter()
            .find(|channel| channel.name == "Throttle Position")
            .unwrap();
        assert_eq!(throttle.unit, "%");
        assert!((throttle.samples[0].value - 4.004716128110886).abs() < 1e-8);
        for name in ["Engine Efficiency", "Gear Ratio", "Launch Engine Speed"] {
            assert!(channels.iter().any(|channel| channel.name == name));
        }
        assert!(
            !decoded
                .warnings
                .iter()
                .any(|warning| warning.contains("unsupported display conversion"))
        );
        assert!(decoded.session.date.is_empty() && decoded.session.time.is_empty());
        assert!(decoded.session.device_serial > 0);
        assert!(
            decoded
                .warnings
                .iter()
                .any(|warning| warning.contains("125 MHz"))
        );
    }
}

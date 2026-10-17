//! Memory-mapped .ld file parser.
//!
//! Opens a MoTeC .ld log file via memory-mapping (no full file read),
//! parses header and channel metadata eagerly, and provides on-demand
//! access to channel sample data.

use half::f16;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use crate::state_labels::state_channel_labels;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HEAD_SIZE: usize = 0x6E2;
const CHAN_META_SIZE: usize = 120;
const MAGIC_BYTE: u8 = 0x40;

// ---------------------------------------------------------------------------
// Binary read helpers (little-endian)
// ---------------------------------------------------------------------------

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_i8(data: &[u8], offset: usize) -> i8 {
    data[offset] as i8
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_f64(data: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn decode_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn read_string(data: &[u8], offset: usize, len: usize) -> String {
    if offset + len > data.len() {
        return String::new();
    }
    decode_string(&data[offset..offset + len])
}

pub fn normalize_channel_name(name: &str) -> String {
    name.to_ascii_lowercase().replace(['.', '_'], " ")
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Data type of channel samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Float32,
    Float16,
    Int8,
    Int16,
    Int32,
    Float64,
    Unknown(u16, u16),
}

impl DataType {
    fn from_codes(dtype_a: u16, dtype_code: u16) -> Self {
        match (dtype_a, dtype_code) {
            (0x07, 4) => DataType::Float32,
            (0x07, 2) => DataType::Float16,
            (0x08, 0x08) => DataType::Float64,
            // For integer channels `dtype_code` is the sample width in bytes, and
            // `dtype_a` only picks the family. 1-byte channels carry
            // low-cardinality state values (Gear, Marker); 0x06 shows up on
            // counters and indices (GPS Satellites, Lap Beacon Ticks).
            (0x00 | 0x03 | 0x05 | 0x06, 1) => DataType::Int8,
            (0x00 | 0x03 | 0x05 | 0x06, 2) => DataType::Int16,
            (0x00 | 0x03 | 0x05 | 0x06, 4) => DataType::Int32,
            _ => DataType::Unknown(dtype_a, dtype_code),
        }
    }

    /// Bytes per sample for this data type.
    pub fn bytes_per_sample(self) -> Option<usize> {
        match self {
            DataType::Float32 => Some(4),
            DataType::Float16 => Some(2),
            DataType::Int8 => Some(1),
            DataType::Int16 => Some(2),
            DataType::Int32 => Some(4),
            DataType::Float64 => Some(8),
            DataType::Unknown(_, _) => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DataType::Float32 => "float32",
            DataType::Float16 => "float16",
            DataType::Int8 => "int8",
            DataType::Int16 => "int16",
            DataType::Int32 => "int32",
            DataType::Float64 => "float64",
            DataType::Unknown(_, _) => "unknown",
        }
    }
}

/// Session metadata from the file header.
#[derive(Debug, Clone)]
pub struct Session {
    pub date: String,
    pub time: String,
    pub driver: String,
    pub vehicle_id: String,
    pub venue: String,
    pub short_comment: String,
    pub device_serial: u32,
    pub device_type: String,
    pub device_version: u16,
    pub num_channels_header: u32,
}

/// Extended metadata from the event/venue/vehicle pointer chain.
#[derive(Debug, Clone, Default)]
pub struct Event {
    pub event_name: String,
    pub session: String,
    pub comment: String,
    pub venue_detail: String,
    pub vehicle_id: String,
    pub vehicle_weight: u32,
    pub vehicle_type: String,
    pub vehicle_comment: String,
}

/// Channel metadata. Does not hold sample data — use `LdFile::read_channel_data()`.
#[derive(Debug, Clone)]
pub struct ChannelMeta {
    pub index: usize,
    pub name: String,
    pub short_name: String,
    pub unit: String,
    /// Physical samples per second, including legacy ADL clock scaling.
    pub freq: f64,
    pub n_data: u32,
    pub data_type: DataType,
    pub shift: i16,
    pub mul: i16,
    pub scale: i16,
    pub dec_places: i16,
    /// Decimal digits for display, independent of the sample scaling exponent.
    pub display_decimals: u8,
    data_ptr: u32,
    legacy_name: String,
    /// Half-open ranges of samples marked invalid by the logger.
    invalid_ranges: Vec<std::ops::Range<usize>>,
    /// Enum/state value labels parsed from the file (value → label).
    /// Empty for non-enum channels. Wrapped in Arc for cheap cloning.
    pub enum_labels: Arc<HashMap<i64, String>>,
}

impl ChannelMeta {
    fn mask_invalid(&self, values: &mut [f64], start: usize) {
        let end = start + values.len();
        let first = self
            .invalid_ranges
            .partition_point(|range| range.end <= start);
        for range in &self.invalid_ranges[first..] {
            if range.start >= end {
                break;
            }
            values[range.start.max(start) - start..range.end.min(end) - start].fill(f64::NAN);
        }
    }

    /// Duration of this channel's data in seconds.
    pub fn duration_secs(&self) -> f64 {
        if self.freq > 0.0 {
            self.n_data as f64 / self.freq
        } else {
            0.0
        }
    }
}

// ---------------------------------------------------------------------------
// LdFile — the main entry point
// ---------------------------------------------------------------------------

/// A memory-mapped MoTeC .ld log file.
///
/// Header and channel metadata are parsed on open. Channel sample data
/// is decoded on-demand from the memory map.
pub struct LdFile {
    backing: LdBacking,
    recording_id: std::sync::OnceLock<String>,
    pub session: Session,
    pub event: Event,
    pub channels: Vec<ChannelMeta>,
    normalized_name_index: HashMap<String, usize>,
    #[allow(dead_code)]
    chan_meta_ptr: u32,
}

enum LdBacking {
    Mmap(Mmap),
    Bytes(Box<[u8]>),
}

impl LdBacking {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mmap(mmap) => mmap,
            Self::Bytes(bytes) => bytes,
        }
    }
}

impl LdFile {
    pub(crate) fn source_bytes(&self) -> &[u8] {
        self.backing.as_slice()
    }

    /// Physical-to-nominal sample rate ratio, also used to migrate saved timelines.
    pub fn sample_clock_scale(&self) -> f64 {
        sample_clock_scale(self.data())
    }

    /// Stable content identity, computed lazily when recording-scoped analysis is used.
    /// FNV-1a 128 is an identity fingerprint, not a cryptographic integrity check.
    pub fn recording_id(&self) -> &str {
        self.recording_id.get_or_init(|| {
            let mut hash = 0x6c62272e07bb014262b821756295c58du128;
            for byte in self.data() {
                hash = (hash ^ *byte as u128).wrapping_mul(0x1000000000000000000013bu128);
            }
            format!("ld:{hash:032x}")
        })
    }

    /// Open and parse a .ld file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let file = File::open(path.as_ref()).map_err(|e| format!("Failed to open file: {}", e))?;

        // Safety: we treat the file as read-only and never modify it.
        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| format!("Failed to mmap file: {}", e))?;

        Self::parse_backing(LdBacking::Mmap(mmap))
    }

    /// Parse a .ld file from in-memory bytes.
    ///
    /// This is primarily intended for web uploads and remote object fetches
    /// where a filesystem path is unavailable.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        Self::parse_backing(LdBacking::Bytes(bytes.into_boxed_slice()))
    }

    fn parse_backing(backing: LdBacking) -> Result<Self, String> {
        let data = backing.as_slice();

        if data.len() < HEAD_SIZE {
            return Err(format!(
                "File too small ({} bytes, need >= {})",
                data.len(),
                HEAD_SIZE
            ));
        }
        if data[0] != MAGIC_BYTE {
            return Err(format!("Bad magic byte: {:#x} (expected 0x40)", data[0]));
        }

        let session = parse_session(data);
        let chan_meta_ptr = read_u32(data, 0x08);
        let chan_data_ptr = read_u32(data, 0x0C);
        let event_ptr = read_u32(data, 0x24);
        let event = parse_event(data, event_ptr);
        let enum_tables = parse_enum_tables(data, chan_data_ptr as usize);
        let channels = parse_channel_metadata(data, chan_meta_ptr, &enum_tables);
        let mut normalized_name_index = HashMap::new();
        for channel in &channels {
            normalized_name_index
                .entry(normalize_channel_name(&channel.name))
                .or_insert(channel.index);
        }
        // Keep saved workspaces using the old truncated spelling readable.
        // Canonical names above always take precedence over legacy aliases.
        for channel in &channels {
            normalized_name_index
                .entry(normalize_channel_name(&channel.legacy_name))
                .or_insert(channel.index);
        }

        Ok(LdFile {
            backing,
            recording_id: std::sync::OnceLock::new(),
            session,
            event,
            channels,
            normalized_name_index,
            chan_meta_ptr,
        })
    }

    /// File size in bytes.
    pub fn file_size(&self) -> usize {
        self.data().len()
    }

    /// First finest-grid boundary after the latest recorded sample timestamp.
    /// Include channels that outlast the fastest channel without adding a full
    /// slow-channel period of padding.
    pub fn duration_secs(&self) -> f64 {
        let channels = self
            .channels
            .iter()
            .filter(|ch| ch.freq > 0.0 && ch.n_data > 0);
        let max_freq = channels.clone().map(|ch| ch.freq).fold(0.0_f64, f64::max);
        if max_freq <= 0.0 {
            return 0.0;
        }
        let end_tick = channels
            .map(|ch| {
                let tick = (ch.n_data - 1) as f64 * (max_freq / ch.freq);
                // Avoid rounding a sample already on the finest grid down one tick.
                (tick + 1e-8).floor() + 1.0
            })
            .fold(0.0_f64, f64::max);
        end_tick / max_freq
    }

    pub fn find_channel_by_name(&self, name: &str) -> Option<&ChannelMeta> {
        if let Some(channel) = self.channels.iter().find(|channel| channel.name == name) {
            return Some(channel);
        }

        let normalized = normalize_channel_name(name);
        self.normalized_name_index
            .get(&normalized)
            .and_then(|idx| self.channels.get(*idx))
    }

    /// Read and decode all sample data for a channel, applying MoTeC scaling.
    /// Returns scaled f64 values, or None if the data type is unknown.
    pub fn read_channel_data(&self, channel: &ChannelMeta) -> Option<Vec<f64>> {
        let bps = channel.data_type.bytes_per_sample()?;
        let offset = channel.data_ptr as usize;
        let count = channel.n_data as usize;
        let data = self.data();

        let available = if offset < data.len() {
            (data.len() - offset) / bps
        } else {
            0
        };
        let actual_count = count.min(available);
        if actual_count == 0 {
            return Some(vec![]);
        }

        let mut vals = self.read_raw_samples(offset, actual_count, channel.data_type);
        apply_scaling(&mut vals, channel);
        channel.mask_invalid(&mut vals, 0);
        Some(vals)
    }

    /// Read a range of samples for a channel (by sample indices), with scaling.
    /// Useful for on-demand access to a visible time window.
    pub fn read_channel_range(
        &self,
        channel: &ChannelMeta,
        start_sample: usize,
        end_sample: usize,
    ) -> Option<Vec<f64>> {
        let bps = channel.data_type.bytes_per_sample()?;
        let count = channel.n_data as usize;
        let start = start_sample.min(count);
        let end = end_sample.min(count);
        if start >= end {
            return Some(vec![]);
        }

        let offset = channel.data_ptr as usize + start * bps;
        let n = end - start;
        let data = self.data();

        let available = if offset < data.len() {
            (data.len() - offset) / bps
        } else {
            0
        };
        let actual = n.min(available);
        if actual == 0 {
            return Some(vec![]);
        }

        let mut vals = self.read_raw_samples(offset, actual, channel.data_type);
        apply_scaling(&mut vals, channel);
        channel.mask_invalid(&mut vals, start);
        Some(vals)
    }

    /// Look up a text label for a state/enum channel value.
    /// Returns `None` if the channel has no enum labels or the value isn't mapped.
    pub fn format_channel_value<'a>(
        &self,
        channel: &'a ChannelMeta,
        value: f64,
    ) -> Option<&'a str> {
        if !value.is_finite() || channel.enum_labels.is_empty() {
            return None;
        }
        let v = value.round() as i64;
        channel.enum_labels.get(&v).map(|s| s.as_str())
    }

    fn read_raw_samples(&self, offset: usize, count: usize, dtype: DataType) -> Vec<f64> {
        let data = self.data();
        let mut vals = Vec::with_capacity(count);
        let bps = dtype.bytes_per_sample().unwrap();

        for i in 0..count {
            let pos = offset + i * bps;
            let v = match dtype {
                DataType::Float32 => read_f32(data, pos) as f64,
                DataType::Float16 => {
                    let bits = read_u16(data, pos);
                    f16::from_bits(bits).to_f64()
                }
                DataType::Int8 => read_i8(data, pos) as f64,
                DataType::Int16 => read_i16(data, pos) as f64,
                DataType::Int32 => read_i32(data, pos) as f64,
                DataType::Float64 => read_f64(data, pos),
                DataType::Unknown(_, _) => 0.0,
            };
            vals.push(v);
        }
        vals
    }

    fn data(&self) -> &[u8] {
        self.backing.as_slice()
    }
}

// ---------------------------------------------------------------------------
// Internal parsers
// ---------------------------------------------------------------------------

/// Convert raw samples to engineering units:
///
/// ```text
/// value = raw / scale * 10^(-dec_places) * mul + shift
/// ```
///
/// `shift` is already in engineering units, so it is applied *after* `mul` — see
/// `docs/ld-file-format.md` for why the order is load-bearing on ADL logs.
///
/// Scales in place: a channel can hold ~190k samples, so this avoids allocating a
/// second buffer the size of the one just decoded.
fn apply_scaling(values: &mut [f64], channel: &ChannelMeta) {
    let scale_f = channel.scale as f64;
    if scale_f == 0.0 {
        return;
    }
    let shift_f = channel.shift as f64;
    let mul_f = channel.mul as f64;
    let dec_factor = 10.0_f64.powi(-channel.dec_places as i32);

    for v in values {
        *v = *v / scale_f * dec_factor * mul_f + shift_f;
    }
}

fn parse_session(data: &[u8]) -> Session {
    Session {
        date: read_string(data, 0x5E, 16),
        time: read_string(data, 0x7E, 16),
        driver: read_string(data, 0x9E, 64),
        vehicle_id: read_string(data, 0xDE, 64),
        venue: read_string(data, 0x15E, 64),
        short_comment: read_string(data, 0x624, 64),
        device_serial: read_u32(data, 0x46),
        device_type: read_string(data, 0x4A, 8),
        device_version: read_u16(data, 0x52),
        num_channels_header: read_u32(data, 0x56),
    }
}

fn parse_event(data: &[u8], event_ptr: u32) -> Event {
    let mut e = Event::default();
    let off = event_ptr as usize;

    if event_ptr == 0 || off + 1154 > data.len() {
        return e;
    }

    e.event_name = read_string(data, off, 64);
    e.session = read_string(data, off + 64, 64);
    e.comment = read_string(data, off + 128, 1024);

    let venue_ptr = read_u32(data, off + 1152) as usize;
    if venue_ptr == 0 || venue_ptr + 1100 > data.len() {
        return e;
    }
    e.venue_detail = read_string(data, venue_ptr, 64);

    let vehicle_ptr = read_u32(data, venue_ptr + 1098) as usize;
    if vehicle_ptr == 0 || vehicle_ptr + 260 > data.len() {
        return e;
    }
    e.vehicle_id = read_string(data, vehicle_ptr, 64);
    e.vehicle_weight = read_u32(data, vehicle_ptr + 192);
    e.vehicle_type = read_string(data, vehicle_ptr + 196, 32);
    e.vehicle_comment = read_string(data, vehicle_ptr + 228, 32);

    e
}

/// M1 records have a 124-byte base and pointer-linked optional blocks. They are
/// not fixed-size: validity transitions can precede the extended attributes.
fn channel_attributes(data: &[u8], off: usize) -> (Option<String>, Option<u16>) {
    let mut name = None;
    let mut enum_id = None;
    let Some(base) = data.get(off..).filter(|b| b.len() >= 124) else {
        return (name, enum_id);
    };
    let ext = read_u32(base, 120) as usize;
    let Some(extension) = data.get(ext..).filter(|b| ext != 0 && b.len() >= 6) else {
        return (name, enum_id);
    };
    let ptr = read_u32(extension, 2) as usize;
    let Some(attrs) = data.get(ptr..).filter(|b| ptr != 0 && b.len() >= 8) else {
        return (name, enum_id);
    };
    if read_u32(attrs, 0) != 1 {
        return (name, enum_id);
    }
    let count = read_u32(attrs, 4) as usize;
    let mut pos = 8;
    // Each attribute has an eight-byte header and a length-delimited payload.
    if count > (attrs.len() - 8) / 8 {
        return (name, enum_id);
    }
    for _ in 0..count {
        let Some(header) = attrs.get(pos..).and_then(|rest| rest.get(..8)) else {
            break;
        };
        let kind = read_u32(header, 0);
        let len = read_u16(header, 6) as usize;
        pos += 8;
        let Some(payload) = attrs.get(pos..).and_then(|rest| rest.get(..len)) else {
            break;
        };
        match kind {
            4 if payload.last() == Some(&0) => {
                let full_name = decode_string(payload);
                if !full_name.is_empty() {
                    name = Some(full_name);
                }
            }
            5 if len == 2 => enum_id = Some(read_u16(payload, 0)),
            _ => {}
        }
        pos += len;
    }
    (name, enum_id)
}

fn channel_invalid_ranges(
    data: &[u8],
    off: usize,
    freq: u16,
    sample_count: u32,
) -> Vec<std::ops::Range<usize>> {
    let ptr = read_u32(data, off + 116) as usize;
    let Some(block) = data.get(ptr..).filter(|b| ptr != 0 && b.len() >= 12) else {
        return Vec::new();
    };
    if read_u16(block, 0) != 0 || read_u16(block, 2) != freq {
        return Vec::new();
    }
    let count = read_u32(block, 4) as usize;
    let records_ptr = read_u32(block, 8) as usize;
    let Some(records) = data.get(records_ptr..).filter(|_| records_ptr != 0) else {
        return Vec::new();
    };
    if count > records.len() / 6 {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut invalid_start = None;
    let mut previous = 0;
    for record in records[..count * 6].as_chunks::<6>().0 {
        let flag = read_u16(record, 0);
        let index = read_u32(record, 2);
        // Reject the whole malformed block, never partially mask channel data.
        if flag > 1 || index < previous || index > sample_count {
            return Vec::new();
        }
        if flag == 0 {
            invalid_start.get_or_insert(index as usize);
        } else if let Some(start) = invalid_start.take()
            && start < index as usize
        {
            ranges.push(start..index as usize);
        }
        previous = index;
    }
    if let Some(start) = invalid_start
        && start < sample_count as usize
    {
        ranges.push(start..sample_count as usize);
    }
    ranges
}

fn sample_clock_scale(data: &[u8]) -> f64 {
    if read_u32(data, 0x42) == 1_024_000 {
        1.024
    } else {
        1.0
    }
}

fn parse_channel_metadata(
    data: &[u8],
    mut meta_ptr: u32,
    enum_tables: &HashMap<u16, Arc<HashMap<i64, String>>>,
) -> Vec<ChannelMeta> {
    let mut channels = Vec::new();
    // Older ADL files use the 1,024,000 clock value at header +0x42,
    // rather than the modern 1,000,000. Keep samples on their native grid.
    let clock_scale = sample_clock_scale(data);
    let mut visited = HashSet::new();

    while meta_ptr != 0 && !visited.contains(&meta_ptr) {
        visited.insert(meta_ptr);
        let off = meta_ptr as usize;
        if data
            .get(off..)
            .is_none_or(|rest| rest.len() < CHAN_META_SIZE)
        {
            break;
        }

        let next_addr = read_u32(data, off + 4);
        let data_ptr = read_u32(data, off + 8);
        let n_data = read_u32(data, off + 12);
        let dtype_a = read_u16(data, off + 18);
        let dtype_code = read_u16(data, off + 20);
        let rec_freq = read_u16(data, off + 22);
        let shift = read_i16(data, off + 24);
        let mul = read_i16(data, off + 26);
        let scale = read_i16(data, off + 28);
        let dec_places = read_i16(data, off + 30);

        let raw_name = &data[off + 32..off + 64];
        let raw_short = &data[off + 64..off + 72];
        let raw_unit = &data[off + 72..off + 84];

        let mut name = decode_string(raw_name);
        let mut short_name = decode_string(raw_short);
        let unit_str = decode_string(raw_unit);

        // Compatibility alias only: never display this old overflow heuristic.
        let mut legacy_name = name.clone();
        if !raw_name.contains(&0)
            && !short_name.is_empty()
            && short_name.len() <= 2
            && raw_name[31].is_ascii_alphanumeric()
            && short_name.as_bytes()[0].is_ascii_alphanumeric()
        {
            legacy_name.push_str(&short_name);
        }
        let legacy_name = legacy_name.replace('.', " ");

        let (full_name, enum_id) = channel_attributes(data, off);
        if let Some(full_name) = full_name {
            name = full_name;
        }

        // Replace dots with spaces for display (e.g. "Engine.Speed" → "Engine Speed")
        name = name.replace('.', " ");

        // Determine unit: prefer unit field, fall back to short_name
        let unit = if !unit_str.is_empty() {
            unit_str
        } else if !short_name.is_empty() {
            let u = short_name.clone();
            short_name = String::new();
            u
        } else {
            String::new()
        };

        // Skip terminator entries (n_data == 0 with empty name)
        if n_data == 0 && name.is_empty() {
            meta_ptr = next_addr;
            continue;
        }

        let data_type = DataType::from_codes(dtype_a, dtype_code);

        let mut enum_labels = enum_id
            .and_then(|id| enum_tables.get(&id).cloned())
            .unwrap_or_default();
        let invalid_ranges = channel_invalid_ranges(data, off, rec_freq, n_data);
        // Byte +100 is the display precision, +101 is its format (integer/float).
        // Older producers leave both zero: retain scaling precision as fallback.
        let display_decimals = if matches!(data[off + 101], 1 | 3) {
            data[off + 100].min(9)
        } else {
            dec_places.clamp(0, 9) as u8
        };
        if enum_labels.is_empty()
            && let Some(fallback_labels) = state_channel_labels(&name)
        {
            enum_labels = Arc::new(fallback_labels);
        }

        channels.push(ChannelMeta {
            index: channels.len(),
            name,
            short_name,
            unit,
            freq: rec_freq as f64 * clock_scale,
            n_data,
            data_type,
            shift,
            mul,
            scale,
            dec_places,
            display_decimals,
            invalid_ranges,
            data_ptr,
            legacy_name,
            enum_labels,
        });

        meta_ptr = next_addr;
    }

    channels
}

// ---------------------------------------------------------------------------
// Enum table parsing — extracts value→label mappings from the ECU config section
// ---------------------------------------------------------------------------

fn enum_scan_bounds(chan_data_ptr: usize, file_len: usize) -> (usize, usize) {
    let file_end = file_len.saturating_sub(12);
    let search_end = if chan_data_ptr == 0 {
        file_end
    } else {
        chan_data_ptr.min(file_end)
    };
    let search_start = HEAD_SIZE.min(search_end);
    (search_start, search_end)
}

/// Parse all enum tables embedded in the file.
/// Returns a map from enum_id → (value → label).
fn parse_enum_tables(data: &[u8], chan_data_ptr: usize) -> HashMap<u16, Arc<HashMap<i64, String>>> {
    let mut tables: HashMap<u16, Arc<HashMap<i64, String>>> = HashMap::new();

    // Enum tables are in the ECU config section, after the raw sample data region.
    // Each table starts with: [u16 count] [u16 type=2] [u16 enum_id]
    // Followed by entries with marker bytes 03.
    // We search for the pattern: [xx xx] [02 00] [xx xx] [03 00 00 00]
    let (search_start, search_end) = enum_scan_bounds(chan_data_ptr, data.len());

    let mut pos = search_start;
    while pos < search_end {
        // Check for enum header: type field = 0x0002
        let type_val = read_u16(data, pos + 2);
        if type_val != 2 {
            pos += 1;
            continue;
        }

        // Check that it's followed by a 03-marker
        if pos + 10 >= data.len() || data[pos + 6] != 0x03 || data[pos + 7] != 0x00 {
            pos += 1;
            continue;
        }

        let count = read_u16(data, pos) as usize;
        let enum_id = read_u16(data, pos + 4);

        if count == 0 || count > 500 {
            pos += 1;
            continue;
        }

        // Scan forward to collect entries with 03-marker
        if let Some((entries, next_pos)) = scan_enum_entries(data, pos + 6, count) {
            tables.entry(enum_id).or_insert_with(|| Arc::new(entries));
            pos = next_pos;
            continue;
        }

        pos += 1;
    }

    tables
}

/// Scan forward from `start` collecting up to `expected` enum entries.
/// Each entry is preceded by a 6-byte marker starting with 0x03.
fn scan_enum_entries(
    data: &[u8],
    start: usize,
    expected: usize,
) -> Option<(HashMap<i64, String>, usize)> {
    let mut entries = HashMap::new();
    let end = (start + 20000).min(data.len());
    let mut pos = start;

    while entries.len() < expected && pos + 10 < end {
        if data[pos] == 0x06 && data[pos + 1] == 0x00 {
            pos += 2;
            continue;
        }

        if data[pos] == 0x03 && data[pos + 1] == 0x00 {
            // Entry: [03 00 xx xx xx xx] [u16 size] [u16 value] [null-terminated string]
            let entry_size = read_u16(data, pos + 6) as usize;
            let entry_value = read_u16(data, pos + 8) as i64;
            // Handle special MoTeC sentinel values
            let entry_value = if entry_value == 0xFFFF {
                -1 // Map 65535 to -1 for display
            } else if entry_value == 0xFFFE {
                -2
            } else {
                entry_value
            };
            let str_len = entry_size.wrapping_sub(2);
            let next_pos = pos + 8 + entry_size;
            if str_len > 0 && str_len < 200 && next_pos <= data.len() {
                let raw = &data[pos + 10..pos + 10 + str_len];
                let label = decode_string(raw);
                if !label.is_empty() {
                    entries.insert(entry_value, label);
                    pos = next_pos;
                    continue;
                }
            }
        }
        pos += 1;
    }

    (entries.len() == expected).then_some((entries, pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn legacy_adl_clock_preserves_native_samples_and_validity() {
        let modern = LdFile::from_bytes(extended_fixture()).unwrap();
        let mut bytes = extended_fixture();
        put_u32(&mut bytes, 0x42, 1_024_000);
        let legacy = LdFile::from_bytes(bytes).unwrap();
        assert_eq!(legacy.channels[0].freq, 2.048);
        assert!((legacy.duration_secs() * 1.024 - modern.duration_secs()).abs() < 1e-9);
        let a = modern.read_channel_data(&modern.channels[0]).unwrap();
        let b = legacy.read_channel_data(&legacy.channels[0]).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(
            a.iter()
                .zip(&b)
                .all(|(a, b)| a == b || a.is_nan() && b.is_nan())
        );
        let mut bytes = extended_fixture();
        put_u32(&mut bytes, 0x42, 1_000_000);
        assert_eq!(LdFile::from_bytes(bytes).unwrap().channels[0].freq, 2.0);
    }

    #[test]
    fn saved_nominal_analysis_migrates_once_without_losing_edits() {
        use crate::corrections::{GpsShift, WheelLockCorrection};
        use crate::ranges::*;
        let mut bytes = extended_fixture();
        put_u32(&mut bytes, 0x42, 1_024_000);
        let ld = LdFile::from_bytes(bytes).unwrap();
        let mut analysis = RecordingAnalysis::from_laps(ld.recording_id().into(), 4.0, &[]);
        analysis.ranges = vec![AnalysisRange {
            id: "run".into(),
            name: "Edited run".into(),
            start: 1.024,
            end: 4.0,
            kind: RangeKind::Run,
            trust: Trust::Trusted,
            comment: "Keep me".into(),
            reaction_time: Some(0.4),
        }];
        analysis.selected_range = Some("run".into());
        analysis.markers = vec![
            RangeMarker {
                id: "marker".into(),
                time: 2.048,
                name: "Edited marker".into(),
                kind: MarkerKind::User,
                trust: Trust::Untrusted,
                distance_m: Some(100.),
            },
            RangeMarker {
                id: "end".into(),
                time: 4.0,
                name: "End".into(),
                kind: MarkerKind::User,
                trust: Trust::Unknown,
                distance_m: None,
            },
        ];
        analysis.details.driver = "Saved driver".into();
        analysis
            .details
            .custom
            .insert("Tyres".into(), "Soft".into());
        analysis.drag_setup.ratios = vec![1.6, 1.28, 1.];
        analysis.drag_runs.insert(
            "run".into(),
            crate::drag::RunDetails {
                elapsed: [Some(0.5), Some(1.), Some(1.5), Some(2.), Some(2.5)],
            },
        );
        analysis.corrections.gps_shift = GpsShift::Manual(1.024);
        analysis.corrections.wheel_lock = Some(WheelLockCorrection {
            acceleration_channel: "G Force Long".into(),
            lag_seconds: 0.512,
            braking_threshold_mps2: -1.,
            speed_threshold_mps: 0.5,
        });
        analysis.corrections.track_length_m = Some(5000.);
        analysis.gps_generation = Some(GpsGeneration {
            gate: [[1., 1.], [2., 2.]],
            minimum_interval: 30.72,
            kind: RangeKind::Lap,
        });
        let original = analysis.clone();
        assert!(analysis.migrate_sample_clock(&ld).unwrap());
        assert_eq!(analysis.duration, ld.duration_secs());
        assert_eq!(analysis.ranges[0].start, 1.);
        assert_eq!(analysis.ranges[0].end, analysis.duration);
        assert_eq!(analysis.ranges[0].id, "run");
        assert_eq!(analysis.ranges[0].name, "Edited run");
        assert_eq!(analysis.ranges[0].comment, "Keep me");
        assert_eq!(analysis.ranges[0].trust, Trust::Trusted);
        assert_eq!(analysis.ranges[0].reaction_time, Some(0.4));
        assert_eq!(analysis.markers[0].time, 2.);
        assert_eq!(analysis.markers[1].time, analysis.duration);
        assert_eq!(analysis.markers[0].distance_m, Some(100.));
        assert_eq!(analysis.details, original.details);
        assert_eq!(analysis.selected_range, original.selected_range);
        assert_eq!(analysis.drag_setup, original.drag_setup);
        assert_eq!(analysis.drag_runs, original.drag_runs);
        assert_eq!(analysis.corrections.gps_shift, GpsShift::Manual(1.));
        assert_eq!(
            analysis
                .corrections
                .wheel_lock
                .as_ref()
                .unwrap()
                .lag_seconds,
            0.5
        );
        assert_eq!(analysis.corrections.track_length_m, Some(5000.));
        assert!((analysis.gps_generation.as_ref().unwrap().minimum_interval - 30.).abs() < 1e-9);
        let json = serde_json::to_string(&analysis).unwrap();
        let mut reopened: RecordingAnalysis = serde_json::from_str(&json).unwrap();
        assert!(!reopened.migrate_sample_clock(&ld).unwrap());
        assert_eq!(serde_json::to_string(&reopened).unwrap(), json);
    }

    #[test]
    fn analysis_clock_migration_rejects_other_recordings_clocks_and_invalid_data() {
        use crate::ranges::*;
        let modern = LdFile::from_bytes(extended_fixture()).unwrap();
        let mut bytes = extended_fixture();
        put_u32(&mut bytes, 0x42, 1_024_000);
        let legacy = LdFile::from_bytes(bytes).unwrap();
        let valid = RecordingAnalysis::from_laps(legacy.recording_id().into(), 4., &[]);
        for mut bad in [
            RecordingAnalysis {
                recording: "other".into(),
                ..valid.clone()
            },
            RecordingAnalysis {
                duration: 4.1,
                ..valid.clone()
            },
            RecordingAnalysis {
                version: 2,
                ..valid.clone()
            },
        ] {
            let before = serde_json::to_string(&bad).unwrap();
            assert!(bad.migrate_sample_clock(&legacy).is_err());
            assert_eq!(serde_json::to_string(&bad).unwrap(), before);
        }
        let mut wrong_clock = RecordingAnalysis::from_laps(
            modern.recording_id().into(),
            modern.duration_secs() * 1.024,
            &[],
        );
        assert!(wrong_clock.migrate_sample_clock(&modern).is_err());
    }

    fn extended_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 4096];
        bytes[0] = MAGIC_BYTE;
        put_u32(&mut bytes, 8, 1800);
        put_u32(&mut bytes, 12, 4000);
        let off = 1800;
        put_u32(&mut bytes, off + 8, 4000);
        put_u32(&mut bytes, off + 12, 8);
        put_u16(&mut bytes, off + 18, 7);
        put_u16(&mut bytes, off + 20, 4);
        put_u16(&mut bytes, off + 22, 2);
        put_u16(&mut bytes, off + 26, 1);
        put_u16(&mut bytes, off + 28, 1);
        bytes[off + 32..off + 64].copy_from_slice(b"A channel name filling 32 bytes!");
        bytes[off + 64] = b'G';
        bytes[off + 100] = 3;
        bytes[off + 101] = 3;
        put_u32(&mut bytes, off + 116, 2000);
        put_u16(&mut bytes, 2002, 2);
        put_u32(&mut bytes, 2004, 4);
        put_u32(&mut bytes, 2008, 2044);
        for (i, (flag, index)) in [(0, 0), (1, 2), (0, 4), (1, 6)].iter().enumerate() {
            put_u16(&mut bytes, 2044 + i * 6, *flag);
            put_u32(&mut bytes, 2046 + i * 6, *index);
        }
        // Attributes follow a validity block, so no fixed +208 enum offset.
        put_u32(&mut bytes, off + 120, 2200);
        put_u32(&mut bytes, 2202, 2270);
        put_u32(&mut bytes, 2270, 1);
        put_u32(&mut bytes, 2274, 2);
        put_u32(&mut bytes, 2278, 5);
        put_u16(&mut bytes, 2284, 2);
        put_u16(&mut bytes, 2286, 137);
        let name = b"A channel name filling 32 bytes! with its full suffix\0";
        put_u32(&mut bytes, 2288, 4);
        put_u16(&mut bytes, 2294, name.len() as u16);
        bytes[2296..2296 + name.len()].copy_from_slice(name);
        for i in 0..8 {
            bytes[4000 + i * 4..4004 + i * 4].copy_from_slice(&(i as f32 + 0.125).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn validity_masks_full_and_partial_reads_without_changing_valid_measurements() {
        let ld = LdFile::from_bytes(extended_fixture()).unwrap();
        let ch = &ld.channels[0];
        let full = ld.read_channel_data(ch).unwrap();
        for (i, value) in full.iter().enumerate() {
            if [0, 1, 4, 5].contains(&i) {
                assert!(value.is_nan());
            } else {
                assert_eq!(*value, i as f64 + 0.125);
            }
        }
        let partial = ld.read_channel_range(ch, 3, 7).unwrap();
        assert_eq!(partial[0], 3.125);
        assert!(partial[1].is_nan() && partial[2].is_nan());
        assert_eq!(partial[3], 6.125);
    }

    #[test]
    fn linked_attributes_preserve_names_units_precision_and_enum_references() {
        let bytes = extended_fixture();
        let (name, enum_id) = channel_attributes(&bytes, 1800);
        assert_eq!(enum_id, Some(137));
        let ld = LdFile::from_bytes(bytes).unwrap();
        let ch = &ld.channels[0];
        assert_eq!(ch.name, name.unwrap());
        assert!(ch.name.ends_with("with its full suffix"));
        assert_eq!(ch.unit, "G");
        assert_eq!(ch.display_decimals, 3);
        assert_eq!(ch.dec_places, 0);
    }

    #[test]
    fn invalid_optional_pointers_counts_and_nonmonotonic_ranges_are_ignored() {
        for (position, value) in [
            (1916, u32::MAX),
            (2004, u32::MAX),
            (2008, u32::MAX),
            (2064, 1),
        ] {
            let mut bytes = extended_fixture();
            put_u32(&mut bytes, position, value);
            let ld = LdFile::from_bytes(bytes).unwrap();
            assert!(
                ld.read_channel_data(&ld.channels[0])
                    .unwrap()
                    .iter()
                    .all(|v| v.is_finite())
            );
        }
        let mut bytes = extended_fixture();
        put_u32(&mut bytes, 1920, u32::MAX);
        let ld = LdFile::from_bytes(bytes).unwrap();
        assert_eq!(ld.channels[0].name, "A channel name filling 32 bytes!");
        assert_eq!(ld.channels[0].unit, "G");
    }

    #[test]
    fn session_endpoint_uses_finest_grid_instead_of_slow_channel_padding() {
        let mut ld = LdFile::from_bytes(extended_fixture()).unwrap();
        ld.channels[0].freq = 100.0;
        ld.channels[0].n_data = 401;
        let mut slow = ld.channels[0].clone();
        slow.freq = 1.0;
        slow.n_data = 5;
        ld.channels.push(slow);
        assert_eq!(ld.duration_secs(), 4.01);

        // A different sample grid may end within the final finest-grid period.
        // Round to the next boundary rather than adding another whole period.
        let mut beacon = ld.channels[0].clone();
        beacon.freq = 68.0;
        beacon.n_data = 266439; // last sample at 3918.205882... seconds
        ld.channels[0].n_data = 391821;
        ld.channels.push(beacon);
        assert_eq!(ld.duration_secs(), 3918.21);
    }

    #[test]
    fn session_endpoint_includes_channels_that_outlast_the_fastest_channel() {
        let mut ld = LdFile::from_bytes(extended_fixture()).unwrap();
        ld.channels[0].freq = 100.0;
        ld.channels[0].n_data = 1000;
        let mut slow = ld.channels[0].clone();
        slow.freq = 1.0;
        slow.n_data = 100;
        ld.channels.push(slow);
        assert_eq!(ld.duration_secs(), 99.01);

        // Empty and zero-frequency channels cannot supply a timestamp or grid.
        ld.channels[0].n_data = 0;
        assert_eq!(ld.duration_secs(), 100.0);
        ld.channels[1].freq = 0.0;
        assert_eq!(ld.duration_secs(), 0.0);
    }

    #[test]
    fn enum_scan_bounds_stay_out_of_header_and_sample_data() {
        assert_eq!(enum_scan_bounds(4_000, 5_000), (HEAD_SIZE, 4_000));
        assert_eq!(enum_scan_bounds(0, 5_000), (HEAD_SIZE, 4_988));
    }

    #[test]
    fn scan_enum_entries_reports_end_of_table() {
        let mut data = Vec::new();

        data.extend_from_slice(&[0x03, 0x00, 0, 0, 0, 0]);
        data.extend_from_slice(&(6u16).to_le_bytes());
        data.extend_from_slice(&(1u16).to_le_bytes());
        data.extend_from_slice(b"Off\0");

        data.extend_from_slice(&[0x03, 0x00, 0, 0, 0, 0]);
        data.extend_from_slice(&(5u16).to_le_bytes());
        data.extend_from_slice(&(2u16).to_le_bytes());
        data.extend_from_slice(b"On\0");

        let (entries, next_pos) = scan_enum_entries(&data, 0, 2).expect("expected valid table");
        assert_eq!(entries.get(&1).map(String::as_str), Some("Off"));
        assert_eq!(entries.get(&2).map(String::as_str), Some("On"));
        assert_eq!(next_pos, data.len());
    }

    fn scaling_channel(shift: i16, mul: i16, scale: i16, dec_places: i16) -> ChannelMeta {
        ChannelMeta {
            index: 0,
            name: String::new(),
            short_name: String::new(),
            unit: String::new(),
            freq: 10.0,
            n_data: 0,
            data_type: DataType::Int16,
            shift,
            mul,
            scale,
            dec_places,
            data_ptr: 0,
            legacy_name: String::new(),
            display_decimals: 0,
            invalid_ranges: Vec::new(),
            enum_labels: Arc::new(HashMap::new()),
        }
    }

    fn scaled(chan: &ChannelMeta, raw: &[f64]) -> Vec<f64> {
        let mut vals = raw.to_vec();
        apply_scaling(&mut vals, chan);
        vals
    }

    /// A sim-exported ADL log packs 0–100 % into int16 via `shift=50, mul=2,
    /// dec_places=3`. Folding `shift` in before `mul` reports 50–150 % instead.
    #[test]
    fn shift_is_applied_after_the_multiplier() {
        let out = scaled(&scaling_channel(50, 2, 1, 3), &[-25_000.0, 0.0, 25_000.0]);
        assert!((out[0] - 0.0).abs() < 1e-9, "got {}", out[0]);
        assert!((out[1] - 50.0).abs() < 1e-9, "got {}", out[1]);
        assert!((out[2] - 100.0).abs() < 1e-9, "got {}", out[2]);
    }

    #[test]
    fn scaling_uses_scale_and_dec_places() {
        // Steering Wheel Position: shift=-6, mul=250, scale=9, dec_places=3.
        let out = scaled(&scaling_channel(-6, 250, 9, 3), &[216.0]);
        assert!(out[0].abs() < 1e-9, "got {}", out[0]);
    }

    #[test]
    fn scaling_is_unchanged_when_mul_is_one() {
        // Vast majority of M1 channels: the operator order must not matter here.
        let out = scaled(&scaling_channel(32_760, 1, 1, 0), &[-32_760.0, -30_000.0]);
        assert!(out[0].abs() < 1e-9, "got {}", out[0]);
        assert!((out[1] - 2_760.0).abs() < 1e-9, "got {}", out[1]);
    }

    #[test]
    fn zero_scale_passes_raw_values_through() {
        let out = scaled(&scaling_channel(50, 2, 0, 3), &[7.0, -3.0]);
        assert_eq!(out, vec![7.0, -3.0]);
    }

    /// For integer channels `dtype_code` is the sample width in bytes and
    /// `dtype_a` only picks the family. Two families were missing: 1-byte widths
    /// (Gear, Marker) and the whole of `0x06` (GPS Satellites, Lap Beacon
    /// Ticks) — both decoded as Unknown and failed to load, the latter even in
    /// the checked-in VIR_LAP fixture.
    #[test]
    fn data_type_codes_map_as_expected() {
        for family in [0x00, 0x03, 0x05, 0x06] {
            assert_eq!(DataType::from_codes(family, 1), DataType::Int8);
            assert_eq!(DataType::from_codes(family, 2), DataType::Int16);
            assert_eq!(DataType::from_codes(family, 4), DataType::Int32);
        }
        assert_eq!(DataType::from_codes(0x07, 4), DataType::Float32);
        assert_eq!(DataType::from_codes(0x07, 2), DataType::Float16);
        assert_eq!(DataType::from_codes(0x08, 0x08), DataType::Float64);
        assert_eq!(DataType::Int8.bytes_per_sample(), Some(1));
        // Float16 must keep winning over Int8 for the 0x07 family.
        assert_eq!(
            DataType::from_codes(0x07, 1),
            DataType::Unknown(0x07, 1),
            "unmapped codes should stay Unknown rather than silently decoding"
        );
    }

    /// Int8 is signed: MoTeC logs reverse gear as -1.
    #[test]
    fn int8_samples_decode_as_signed() {
        let mut file = vec![0u8; HEAD_SIZE];
        file[0] = 0x40; // magic
        let data_ptr = file.len();
        file.extend_from_slice(&[0x00, 0x01, 0x07, 0xFF]); // 0, 1, 7, -1

        let mut chan = scaling_channel(0, 1, 1, 0);
        chan.data_type = DataType::Int8;
        chan.n_data = 4;
        chan.data_ptr = data_ptr as u32;

        let ld = LdFile::from_bytes(file).expect("synthetic file should open");
        let vals = ld.read_channel_data(&chan).expect("Int8 should decode");
        assert_eq!(vals, vec![0.0, 1.0, 7.0, -1.0]);
    }
}

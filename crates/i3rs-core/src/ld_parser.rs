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

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Data type of channel samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Float32,
    Float16,
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
            (0x00 | 0x03 | 0x05, 2) => DataType::Int16,
            (0x00 | 0x03 | 0x05, 4) => DataType::Int32,
            (0x08, 0x08) => DataType::Float64,
            _ => DataType::Unknown(dtype_a, dtype_code),
        }
    }

    /// Bytes per sample for this data type.
    pub fn bytes_per_sample(self) -> Option<usize> {
        match self {
            DataType::Float32 => Some(4),
            DataType::Float16 => Some(2),
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
    pub freq: u16,
    pub n_data: u32,
    pub data_type: DataType,
    pub shift: i16,
    pub mul: i16,
    pub scale: i16,
    pub dec_places: i16,
    data_ptr: u32,
    /// Enum/state value labels parsed from the file (value → label).
    /// Empty for non-enum channels. Wrapped in Arc for cheap cloning.
    pub enum_labels: Arc<HashMap<i64, String>>,
}

impl ChannelMeta {
    /// Duration of this channel's data in seconds.
    pub fn duration_secs(&self) -> f64 {
        if self.freq > 0 {
            self.n_data as f64 / self.freq as f64
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
    mmap: Mmap,
    pub session: Session,
    pub event: Event,
    pub channels: Vec<ChannelMeta>,
    #[allow(dead_code)]
    chan_meta_ptr: u32,
}

impl LdFile {
    /// Open and parse a .ld file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let file = File::open(path.as_ref()).map_err(|e| format!("Failed to open file: {}", e))?;

        // Safety: we treat the file as read-only and never modify it.
        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| format!("Failed to mmap file: {}", e))?;

        if mmap.len() < HEAD_SIZE {
            return Err(format!(
                "File too small ({} bytes, need >= {})",
                mmap.len(),
                HEAD_SIZE
            ));
        }
        if mmap[0] != MAGIC_BYTE {
            return Err(format!("Bad magic byte: {:#x} (expected 0x40)", mmap[0]));
        }

        let session = parse_session(&mmap);
        let chan_meta_ptr = read_u32(&mmap, 0x08);
        let chan_data_ptr = read_u32(&mmap, 0x0C);
        let event_ptr = read_u32(&mmap, 0x24);
        let event = parse_event(&mmap, event_ptr);
        let enum_tables = parse_enum_tables(&mmap, chan_data_ptr as usize);
        let channels = parse_channel_metadata(&mmap, chan_meta_ptr, &enum_tables);

        Ok(LdFile {
            mmap,
            session,
            event,
            channels,
            chan_meta_ptr,
        })
    }

    /// File size in bytes.
    pub fn file_size(&self) -> usize {
        self.mmap.len()
    }

    /// Estimated session duration in seconds (from the longest channel).
    pub fn duration_secs(&self) -> f64 {
        self.channels
            .iter()
            .map(|ch| ch.duration_secs())
            .fold(0.0_f64, f64::max)
    }

    /// Read and decode all sample data for a channel, applying MoTeC scaling.
    /// Returns scaled f64 values, or None if the data type is unknown.
    pub fn read_channel_data(&self, channel: &ChannelMeta) -> Option<Vec<f64>> {
        let bps = channel.data_type.bytes_per_sample()?;
        let offset = channel.data_ptr as usize;
        let count = channel.n_data as usize;

        let available = if offset < self.mmap.len() {
            (self.mmap.len() - offset) / bps
        } else {
            0
        };
        let actual_count = count.min(available);
        if actual_count == 0 {
            return Some(vec![]);
        }

        let raw = self.read_raw_samples(offset, actual_count, channel.data_type);
        Some(self.apply_scaling(&raw, channel))
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

        let available = if offset < self.mmap.len() {
            (self.mmap.len() - offset) / bps
        } else {
            0
        };
        let actual = n.min(available);
        if actual == 0 {
            return Some(vec![]);
        }

        let raw = self.read_raw_samples(offset, actual, channel.data_type);
        Some(self.apply_scaling(&raw, channel))
    }

    /// Look up a text label for a state/enum channel value.
    /// Returns `None` if the channel has no enum labels or the value isn't mapped.
    pub fn format_channel_value<'a>(
        &self,
        channel: &'a ChannelMeta,
        value: f64,
    ) -> Option<&'a str> {
        if channel.enum_labels.is_empty() {
            return None;
        }
        let v = value.round() as i64;
        channel.enum_labels.get(&v).map(|s| s.as_str())
    }

    fn read_raw_samples(&self, offset: usize, count: usize, dtype: DataType) -> Vec<f64> {
        let data = &self.mmap;
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
                DataType::Int16 => read_i16(data, pos) as f64,
                DataType::Int32 => read_i32(data, pos) as f64,
                DataType::Float64 => read_f64(data, pos),
                DataType::Unknown(_, _) => 0.0,
            };
            vals.push(v);
        }
        vals
    }

    fn apply_scaling(&self, raw: &[f64], channel: &ChannelMeta) -> Vec<f64> {
        let scale_f = channel.scale as f64;
        let shift_f = channel.shift as f64;
        let mul_f = channel.mul as f64;
        let dec_factor = 10.0_f64.powi(-channel.dec_places as i32);

        if scale_f == 0.0 {
            raw.to_vec()
        } else {
            raw.iter()
                .map(|v| (v / scale_f * dec_factor + shift_f) * mul_f)
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Internal parsers
// ---------------------------------------------------------------------------

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

/// Extended channel record size (120 base + 92 extended metadata).
const CHAN_RECORD_SIZE: usize = 212;

fn parse_channel_metadata(
    data: &[u8],
    mut meta_ptr: u32,
    enum_tables: &HashMap<u16, Arc<HashMap<i64, String>>>,
) -> Vec<ChannelMeta> {
    let mut channels = Vec::new();
    let mut visited = HashSet::new();

    while meta_ptr != 0 && !visited.contains(&meta_ptr) {
        visited.insert(meta_ptr);
        let off = meta_ptr as usize;
        if off + CHAN_META_SIZE > data.len() {
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

        // Name overflow heuristic for M1 ECU channel names > 32 chars
        let name_overflowed = !raw_name.contains(&0u8);
        if name_overflowed && !short_name.is_empty() {
            let last_char = raw_name[31];
            let first_short = short_name.as_bytes()[0];
            if last_char.is_ascii_alphanumeric()
                && first_short.is_ascii_alphanumeric()
                && short_name.len() <= 2
            {
                name.push_str(&short_name);
                short_name = String::new();
            }
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

        // Extract enum reference from extended metadata:
        //   +208: u16 enum_type (0x0002 = uses enum table)
        //   +210: u16 enum_id   (indexes into parsed enum tables)
        let enum_labels =
            if off + CHAN_RECORD_SIZE <= data.len() && read_u16(data, off + 208) == 0x0002 {
                let enum_id = read_u16(data, off + 210);
                enum_tables
                    .get(&enum_id)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(HashMap::new()))
            } else {
                Arc::new(HashMap::new())
            };

        channels.push(ChannelMeta {
            index: channels.len(),
            name,
            short_name,
            unit,
            freq: rec_freq,
            n_data,
            data_type,
            shift,
            mul,
            scale,
            dec_places,
            data_ptr,
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
}

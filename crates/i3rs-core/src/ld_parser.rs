//! Memory-mapped .ld file parser.
//!
//! Opens a MoTeC .ld log file via memory-mapping (no full file read),
//! parses header and channel metadata eagerly, and provides on-demand
//! access to channel sample data.

use half::f16;
use memmap2::Mmap;
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

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
        let event_ptr = read_u32(&mmap, 0x24);
        let event = parse_event(&mmap, event_ptr);
        let channels = parse_channel_metadata(&mmap, chan_meta_ptr);

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

fn parse_channel_metadata(data: &[u8], mut meta_ptr: u32) -> Vec<ChannelMeta> {
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
        });

        meta_ptr = next_addr;
    }

    channels
}

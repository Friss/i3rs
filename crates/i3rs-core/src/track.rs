//! GPS track extraction, normalization, color mapping, and sector timing.

use serde::{Deserialize, Serialize};

use crate::Lap;
use crate::ld_parser::{LdFile, normalize_channel_name};

/// Normalized GPS track data ready for rendering.
pub struct TrackData {
    /// Normalized X coordinates (from longitude, scaled by cos(mean_lat)).
    pub x: Vec<f64>,
    /// Normalized Y coordinates (from latitude).
    pub y: Vec<f64>,
    /// Time in seconds for each GPS sample.
    pub time: Vec<f64>,
    /// GPS sample frequency (Hz).
    pub freq: u16,
    pub distance_stretch_tolerance: Option<f64>,
    spatial_index: Option<TrackSpatialIndex>,
    origin: Option<(f64, f64, f64)>,
    pub distance: Option<(Vec<f64>, u16)>,
    /// Corrected speed in metres per second, shared with distance integration.
    pub corrected_speed: Option<(Vec<f64>, u16)>,
}

impl TrackData {
    pub fn geographic_fixes(&self) -> Option<Vec<[f64; 2]>> {
        let (lat, lon, cos) = self.origin?;
        if cos.abs() < 1e-9 {
            return None;
        }
        Some(
            self.x
                .iter()
                .zip(&self.y)
                .map(|(x, y)| [y + lat, x / cos + lon])
                .collect(),
        )
    }

    pub fn from_normalized_parts(x: Vec<f64>, y: Vec<f64>, time: Vec<f64>, freq: u16) -> Self {
        let spatial_index = TrackSpatialIndex::build(&x, &y);
        Self {
            x,
            y,
            time,
            freq,
            distance_stretch_tolerance: Some(DISTANCE_STRETCH_TOLERANCE),
            spatial_index,
            origin: None,
            distance: None,
            corrected_speed: None,
        }
    }
}

struct TrackSpatialIndex {
    min_x: f64,
    min_y: f64,
    cell_width: f64,
    cell_height: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl TrackSpatialIndex {
    fn build(x: &[f64], y: &[f64]) -> Option<Self> {
        if x.is_empty() || y.is_empty() {
            return None;
        }

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for (&xv, &yv) in x.iter().zip(y.iter()) {
            if xv.is_finite() && yv.is_finite() {
                min_x = min_x.min(xv);
                max_x = max_x.max(xv);
                min_y = min_y.min(yv);
                max_y = max_y.max(yv);
            }
        }

        if !min_x.is_finite() || !min_y.is_finite() {
            return None;
        }

        let grid_dim = ((x.len() as f64 / 64.0).sqrt().ceil() as usize).clamp(4, 128);
        let span_x = (max_x - min_x).max(1e-9);
        let span_y = (max_y - min_y).max(1e-9);
        let cell_width = span_x / grid_dim as f64;
        let cell_height = span_y / grid_dim as f64;
        let cell_count = grid_dim * grid_dim;
        let mut cells = vec![Vec::new(); cell_count];

        let index = Self {
            min_x,
            min_y,
            cell_width,
            cell_height,
            cols: grid_dim,
            rows: grid_dim,
            cells: Vec::new(),
        };

        for (sample_idx, (&xv, &yv)) in x.iter().zip(y.iter()).enumerate() {
            if !xv.is_finite() || !yv.is_finite() {
                continue;
            }
            let (col, row) = index.cell_coords(xv, yv);
            cells[index.cell_index(col, row)].push(sample_idx);
        }

        Some(Self { cells, ..index })
    }

    fn find_nearest(&self, track: &TrackData, x: f64, y: f64) -> usize {
        let (base_col, base_row) = self.cell_coords(x, y);
        let max_ring = self.cols.max(self.rows);
        let mut best_idx = 0usize;
        let mut best_dist = f64::INFINITY;

        for ring in 0..max_ring {
            self.for_each_ring_cell(base_col, base_row, ring, |col, row| {
                for &sample_idx in &self.cells[self.cell_index(col, row)] {
                    let dx = track.x[sample_idx] - x;
                    let dy = track.y[sample_idx] - y;
                    let dist = dx * dx + dy * dy;
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = sample_idx;
                    }
                }
            });

            if best_dist.is_finite() {
                let next_ring = ring + 1;
                if next_ring >= max_ring {
                    break;
                }

                let mut next_ring_min_dist = f64::INFINITY;
                self.for_each_ring_cell(base_col, base_row, next_ring, |col, row| {
                    let dist = self.cell_rect_distance2(col, row, x, y);
                    next_ring_min_dist = next_ring_min_dist.min(dist);
                });

                if !next_ring_min_dist.is_finite() || best_dist <= next_ring_min_dist {
                    break;
                }
            }
        }

        best_idx
    }

    fn for_each_ring_cell(
        &self,
        base_col: usize,
        base_row: usize,
        ring: usize,
        mut visit: impl FnMut(usize, usize),
    ) {
        let min_col = base_col.saturating_sub(ring);
        let max_col = (base_col + ring).min(self.cols.saturating_sub(1));
        let min_row = base_row.saturating_sub(ring);
        let max_row = (base_row + ring).min(self.rows.saturating_sub(1));

        if ring == 0 {
            visit(base_col, base_row);
            return;
        }

        for col in min_col..=max_col {
            visit(col, min_row);
            if max_row != min_row {
                visit(col, max_row);
            }
        }

        if max_row > min_row + 1 {
            for row in (min_row + 1)..max_row {
                visit(min_col, row);
                if max_col != min_col {
                    visit(max_col, row);
                }
            }
        }
    }

    fn cell_coords(&self, x: f64, y: f64) -> (usize, usize) {
        let col = ((x - self.min_x) / self.cell_width).floor() as isize;
        let row = ((y - self.min_y) / self.cell_height).floor() as isize;
        (
            col.clamp(0, self.cols.saturating_sub(1) as isize) as usize,
            row.clamp(0, self.rows.saturating_sub(1) as isize) as usize,
        )
    }

    fn cell_index(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    fn cell_rect_distance2(&self, col: usize, row: usize, x: f64, y: f64) -> f64 {
        let x0 = self.min_x + col as f64 * self.cell_width;
        let x1 = x0 + self.cell_width;
        let y0 = self.min_y + row as f64 * self.cell_height;
        let y1 = y0 + self.cell_height;

        let dx = if x < x0 {
            x0 - x
        } else if x > x1 {
            x - x1
        } else {
            0.0
        };
        let dy = if y < y0 {
            y0 - y
        } else if y > y1 {
            y - y1
        } else {
            0.0
        };

        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SectionKind {
    Corner,
    Straight,
    #[default]
    Other,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct SectionSemantics {
    pub id: String,
    pub kind: SectionKind,
    pub categories: std::collections::BTreeSet<String>,
}

impl SectionSemantics {
    fn with_identity(&self, fallback: String) -> Self {
        let mut semantics = self.clone();
        if semantics.id.is_empty() {
            semantics.id = fallback;
        }
        semantics
    }
}

/// A track sector defined by GPS sample index boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sector {
    #[serde(default)]
    pub semantics: SectionSemantics,
    pub name: String,
    /// Index into GPS data where this sector starts.
    pub start_index: usize,
    /// Index into GPS data where this sector ends (exclusive).
    pub end_index: usize,
    #[serde(default)]
    pub distance: Option<DistanceSection>,
}

/// Timing result for one sector in one lap.
#[derive(Clone, Debug)]
pub struct SectorTime {
    pub sector_name: String,
    pub lap_name: String,
    /// Time in seconds spent in this sector during this lap.
    pub time_secs: f64,
}

/// Extract GPS track data from a loaded .ld file.
///
/// Searches for channels matching "GPS Latitude" and "GPS Longitude" (case-insensitive,
/// underscore/space/dot normalized). Returns None if either channel is missing.
pub fn extract_gps_track(ld: &LdFile) -> Option<TrackData> {
    let (lat_ch, lon_ch) = gps_channels(ld)?;
    let lat_data = ld.read_channel_data(lat_ch)?;
    let lon_data = ld.read_channel_data(lon_ch)?;
    let mut track = gps_track_from_samples(&lat_data, lat_ch.freq, &lon_data, lon_ch.freq)?;
    attach_corrected_distance(&mut track, ld);
    Some(track)
}

/// The latitude and longitude channels, when the log has both.
pub fn gps_channels(ld: &LdFile) -> Option<(&crate::ChannelMeta, &crate::ChannelMeta)> {
    Some((
        find_gps_channel(ld, &["latitude", "lat"])?,
        find_gps_channel(ld, &["longitude", "lon", "long"])?,
    ))
}

/// Build a normalized track from decoded GPS samples, resampling longitude to
/// the latitude rate.
pub fn gps_track_from_samples(
    lat: &[f64],
    lat_freq: u16,
    lon: &[f64],
    lon_freq: u16,
) -> Option<TrackData> {
    if lat_freq == 0 || lon_freq == 0 {
        return None;
    }
    let lon_at_lat_rate: Vec<f64> = (0..lat.len())
        .map(|i| {
            crate::analysis::sample_at_time(lon, lon_freq, i as f64 / lat_freq as f64)
                .unwrap_or(f64::NAN)
        })
        .collect();
    normalize_gps_samples(lat, &lon_at_lat_rate, lat_freq)
}

/// Attach corrected speed and the distance integrated from it.
pub fn attach_corrected_distance(track: &mut TrackData, ld: &LdFile) {
    track.corrected_speed = crate::distance::corrected_speed_from_ld(ld);
    track.distance = track.corrected_speed.as_ref().and_then(|(speed, freq)| {
        crate::distance::integrate_speed_series(speed, *freq, "m/s").map(|d| (d, *freq))
    });
}

fn normalize_gps_samples(lat: &[f64], lon: &[f64], freq: u16) -> Option<TrackData> {
    let valid = |a: f64, b: f64| {
        a.is_finite() && b.is_finite() && a.abs() <= 90.0 && b.abs() <= 180.0
            // A paired zero is the receiver's no-fix sentinel. Either coordinate
            // may legitimately be zero on its own (equator/prime meridian).
            && (a != 0.0 || b != 0.0)
    };
    let mut sum_lat = 0.0;
    let mut sum_lon = 0.0;
    let mut count = 0;
    for (&a, &b) in lat.iter().zip(lon) {
        if valid(a, b) {
            sum_lat += a;
            sum_lon += b;
            count += 1;
        }
    }
    if count == 0 || freq == 0 {
        return None;
    }
    let mean_lat = sum_lat / count as f64;
    let mean_lon = sum_lon / count as f64;
    let cos_lat = mean_lat.to_radians().cos();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut time = Vec::new();
    for (i, (&a, &b)) in lat.iter().zip(lon).enumerate() {
        // Preserve timestamps and sample indices without drawing a false line
        // through a lost fix or making a missing fix selectable on the map.
        x.push(if valid(a, b) {
            (b - mean_lon) * cos_lat
        } else {
            f64::NAN
        });
        y.push(if valid(a, b) { a - mean_lat } else { f64::NAN });
        time.push(i as f64 / freq as f64);
    }
    let mut track = TrackData::from_normalized_parts(x, y, time, freq);
    track.origin = Some((mean_lat, mean_lon, cos_lat));
    Some(track)
}

/// Distance boundaries are stored as integer millimetres for stable cache keys.
/// Laps within 5% of the model length are stretched to that length; other laps
/// retain their measured distance. Wrapping sections include both sides of the beacon.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DistanceSection {
    pub start_mm: u32,
    pub end_mm: u32,
    pub track_length_mm: u32,
}

impl DistanceSection {
    /// Both boundaries lie on a non-empty model and differ from each other.
    pub fn is_valid(&self) -> bool {
        self.track_length_mm > 0
            && self.start_mm <= self.track_length_mm
            && self.end_mm <= self.track_length_mm
            && self.start_mm != self.end_mm
    }
}

/// Laps whose measured length is within this fraction of a distance model's
/// length are stretched to the model length.
pub const DISTANCE_STRETCH_TOLERANCE: f64 = 0.05;

/// Approximate metres per degree of latitude (normalized GPS coordinates are degrees).
const METERS_PER_DEGREE: f64 = 111_320.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DistanceTrackDefinition {
    pub version: u32,
    pub name: String,
    pub length_m: f64,
    pub sections: Vec<DistanceSectionDefinition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DistanceSectionDefinition {
    #[serde(default)]
    pub semantics: SectionSemantics,
    pub name: String,
    pub start_m: f64,
    pub end_m: f64,
}

pub fn vir_full_definition() -> DistanceTrackDefinition {
    serde_json::from_str(include_str!("../data/vir-full-sections.json"))
        .expect("bundled VIR definition must be valid")
}

/// Resolve distance boundaries onto a selected lap for map markers, retaining
/// the exact distance boundary for timing on every other lap.
pub fn sectors_from_distance_definition(
    track: &TrackData,
    lap: &Lap,
    definition: &DistanceTrackDefinition,
) -> Result<Vec<Sector>, String> {
    if track.time.is_empty()
        || track.x.len() != track.time.len()
        || track.y.len() != track.time.len()
        || !(1..=2).contains(&definition.version)
        || !definition.length_m.is_finite()
        || definition.length_m <= 0.0
        || definition.length_m > 1_000_000.0
        || definition.sections.len() > 100
    {
        return Err("Invalid distance track definition".into());
    }
    definition
        .sections
        .iter()
        .map(|s| {
            if s.name.trim().is_empty()
                || ![s.start_m, s.end_m]
                    .iter()
                    .all(|d| d.is_finite() && *d >= 0.0 && *d <= definition.length_m)
                || s.start_m == s.end_m
            {
                return Err("Invalid distance section boundary".into());
            }
            let distance = DistanceSection {
                start_mm: (s.start_m * 1000.0).round() as u32,
                end_mm: (s.end_m * 1000.0).round() as u32,
                track_length_mm: (definition.length_m * 1000.0).round() as u32,
            };
            let (a, b) = distance_section_interval(&distance, lap, track)
                .ok_or("Select a complete lap that visits all section boundaries")?;
            let index = |t: f64| {
                track
                    .time
                    .partition_point(|v| *v < t)
                    .min(track.time.len().saturating_sub(1))
            };
            let (start_index, end_index) = (index(a), index(b));
            if ![
                track.x[start_index],
                track.y[start_index],
                track.x[end_index],
                track.y[end_index],
            ]
            .iter()
            .all(|v| v.is_finite())
            {
                return Err("GPS is missing at a section boundary".into());
            }
            Ok(Sector {
                semantics: s.semantics.with_identity(format!(
                    "distance-{}-{}-{}",
                    distance.start_mm, distance.end_mm, distance.track_length_mm
                )),
                name: s.name.clone(),
                start_index,
                end_index,
                distance: Some(distance),
            })
        })
        .collect()
}

fn distance_section_interval(
    section: &DistanceSection,
    lap: &Lap,
    track: &TrackData,
) -> Option<(f64, f64)> {
    let (data, freq) = track.distance.as_ref()?;
    let start = crate::sample_at_time(data, *freq, lap.start_time)?;
    let end = crate::sample_at_time(data, *freq, lap.end_time)?;
    let length = end - start;
    let model = section.track_length_mm as f64 / 1000.0;
    if length <= 0.0 || !section.is_valid() {
        return None;
    }
    let scale = if track
        .distance_stretch_tolerance
        .is_some_and(|t| ((length - model) / model).abs() <= t)
    {
        length / model
    } else {
        1.0
    };
    let crossing = |mm: u32| {
        if mm == 0 {
            return Some(lap.start_time);
        }
        let target = start + mm as f64 / 1000.0 * scale;
        if target > end {
            return None;
        }
        let i = data.partition_point(|d| *d < target);
        if i == 0 {
            return Some(0.0);
        }
        if i >= data.len() {
            return None;
        }
        let width = data[i] - data[i - 1];
        if width <= 0.0 {
            return None;
        }
        Some(((i - 1) as f64 + (target - data[i - 1]) / width) / *freq as f64)
    };
    Some((crossing(section.start_mm)?, crossing(section.end_mm)?))
}

/// Portable section definitions use geographic positions, never sample indices
/// from a different recording. The same file can be applied to another session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackDefinition {
    pub version: u32,
    pub name: String,
    pub sections: Vec<GeographicSection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeographicSection {
    #[serde(default)]
    pub semantics: SectionSemantics,
    pub name: String,
    /// Latitude, longitude in degrees.
    pub start: [f64; 2],
    pub end: [f64; 2],
    #[serde(default)]
    pub distance: Option<DistanceSection>,
}

impl TrackDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=2).contains(&self.version) {
            return Err("Unsupported track definition version".into());
        }
        if self.sections.len() > 100 {
            return Err("Track definitions support up to 100 sections".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for s in &self.sections {
            if s.name.trim().is_empty()
                || s.start.iter().chain(s.end.iter()).any(|v| !v.is_finite())
                || s.start[0].abs() > 90.
                || s.end[0].abs() > 90.
                || s.start[1].abs() > 180.
                || s.end[1].abs() > 180.
                || s.start == s.end
            {
                return Err("Invalid geographic section boundary or name".into());
            }
            if s.distance.as_ref().is_some_and(|d| !d.is_valid()) {
                return Err("Invalid distance section boundary".into());
            }
            if !s.semantics.id.is_empty() && !ids.insert(&s.semantics.id) {
                return Err("Duplicate track section identity".into());
            }
        }
        Ok(())
    }
}

pub fn export_track_definition(
    track: &TrackData,
    sectors: &[Sector],
    name: String,
) -> Option<TrackDefinition> {
    let (lat, lon, scale) = track.origin?;
    if scale.abs() < 1e-9 {
        return None;
    }
    let point = |idx: usize| -> Option<[f64; 2]> {
        let x = *track.x.get(idx)?;
        let y = *track.y.get(idx)?;
        (x.is_finite() && y.is_finite()).then_some([y + lat, x / scale + lon])
    };
    let sections: Option<Vec<_>> = sectors
        .iter()
        .map(|s| {
            Some(GeographicSection {
                semantics: s
                    .semantics
                    .with_identity(format!("gps-{}-{}", s.start_index, s.end_index)),
                name: s.name.clone(),
                start: point(s.start_index)?,
                end: point(s.end_index)?,
                distance: s.distance.clone(),
            })
        })
        .collect();
    Some(TrackDefinition {
        version: 2,
        name,
        sections: sections?,
    })
}

pub fn import_track_definition(
    track: &TrackData,
    definition: &TrackDefinition,
) -> Result<Vec<Sector>, String> {
    definition.validate()?;
    if definition
        .sections
        .iter()
        .any(|s| s.distance.as_ref().is_some_and(|d| !d.is_valid()))
    {
        return Err("Invalid distance section boundary".into());
    }
    if !(1..=2).contains(&definition.version) {
        return Err("Unsupported track definition version".into());
    }
    let (lat, lon, scale) = track.origin.ok_or("GPS origin is unavailable")?;
    let locate = |p: [f64; 2]| -> Result<usize, String> {
        if !p.iter().all(|v| v.is_finite()) || p[0].abs() > 90.0 || p[1].abs() > 180.0 {
            return Err("Invalid geographic section boundary".into());
        }
        let (x, y) = ((p[1] - lon) * scale, p[0] - lat);
        let idx = find_nearest_sample(track, x, y);
        let distance = (track.x[idx] - x).hypot(track.y[idx] - y) * METERS_PER_DEGREE;
        if !distance.is_finite() || distance > 100.0 {
            return Err(
                "Section boundary is more than 100 m from this recording's GPS trace".into(),
            );
        }
        Ok(idx)
    };
    if definition.sections.len() > 100 {
        return Err("Track definitions support up to 100 sections".into());
    }
    definition
        .sections
        .iter()
        .map(|s| {
            if s.name.trim().is_empty() {
                return Err("Sections must have names".into());
            }
            let start_index = locate(s.start)?;
            let end_index = locate(s.end)?;
            if start_index == end_index {
                return Err(format!("{} has identical boundaries", s.name));
            }
            Ok(Sector {
                semantics: s.semantics.with_identity(format!(
                    "geo-{:016x}-{:016x}-{:016x}-{:016x}",
                    s.start[0].to_bits(),
                    s.start[1].to_bits(),
                    s.end[0].to_bits(),
                    s.end[1].to_bits()
                )),
                name: s.name.clone(),
                start_index,
                end_index,
                distance: s.distance.clone(),
            })
        })
        .collect()
}

/// Seed a usable section layout from equal distances on a chosen complete lap.
/// These are generic sections, not inferred corner names.
pub fn split_lap_into_sectors(track: &TrackData, lap: &Lap, count: usize) -> Vec<Sector> {
    let indices: Vec<_> = track
        .time
        .iter()
        .enumerate()
        .filter(|(i, t)| {
            **t >= lap.start_time
                && **t < lap.end_time
                && track.x[*i].is_finite()
                && track.y[*i].is_finite()
        })
        .map(|(i, _)| i)
        .collect();
    if count < 2 || indices.len() < count + 1 {
        return Vec::new();
    }
    let mut distances = vec![0.0];
    for pair in indices.windows(2) {
        let distance = if pair[1] == pair[0] + 1 {
            (track.x[pair[1]] - track.x[pair[0]]).hypot(track.y[pair[1]] - track.y[pair[0]])
        } else {
            0.0
        };
        distances.push(distances.last().unwrap() + distance);
    }
    let total = *distances.last().unwrap();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut boundaries: Vec<_> = (0..count)
        .map(|i| {
            let target = total * i as f64 / count as f64;
            let after = distances
                .partition_point(|d| *d < target)
                .min(indices.len() - 1);
            let before = after.saturating_sub(1);
            indices[if (distances[before] - target).abs() <= (distances[after] - target).abs() {
                before
            } else {
                after
            }]
        })
        .collect();
    boundaries.dedup();
    if boundaries.len() != count {
        return Vec::new();
    }
    (0..count)
        .map(|i| Sector {
            semantics: SectionSemantics {
                id: format!("split-{}-{}", boundaries[i], boundaries[(i + 1) % count]),
                ..Default::default()
            },
            name: format!("Section {}", i + 1),
            start_index: boundaries[i],
            end_index: boundaries[(i + 1) % count],
            distance: None,
        })
        .collect()
}

/// Find a GPS channel by looking for "gps" + one of the given suffixes in the channel name.
fn find_gps_channel<'a>(ld: &'a LdFile, suffixes: &[&str]) -> Option<&'a crate::ChannelMeta> {
    let channels = &ld.channels;
    channels.iter().find(|ch| {
        let name = normalize_channel_name(&ch.name);
        name.contains("gps") && suffixes.iter().any(|s| name.contains(s))
    })
}

/// Find the nearest GPS sample to the given normalized (x, y) coordinate.
/// Returns the sample index.
pub fn find_nearest_sample(track: &TrackData, x: f64, y: f64) -> usize {
    if let Some(index) = &track.spatial_index {
        return index.find_nearest(track, x, y);
    }

    let mut best_idx = 0;
    let mut best_dist = f64::MAX;
    for i in 0..track.x.len() {
        let dx = track.x[i] - x;
        let dy = track.y[i] - y;
        let dist = dx * dx + dy * dy;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i;
        }
    }
    best_idx
}

/// Map channel data to a rainbow color gradient (blue=min, red=max).
///
/// Resamples the color channel to GPS frequency via linear interpolation,
/// then maps each value to an HSV rainbow.
/// Returns one `[u8; 4]` (RGBA) per GPS sample.
/// Compute rainbow colors and the actual value range used.
///
/// Returns `(colors, vmin, vmax)` so the legend matches the rendered colors exactly.
pub fn compute_color_map(
    track: &TrackData,
    channel_data: &[f64],
    channel_freq: u16,
) -> (Vec<[u8; 4]>, f64, f64) {
    let n = track.x.len();
    let mut colors = Vec::with_capacity(n);

    let resampled = resample_to_track(track, channel_data, channel_freq);
    let (vmin, vmax) = color_channel_range(&resampled);
    let range = if (vmax - vmin).abs() < 1e-10 {
        1.0
    } else {
        vmax - vmin
    };

    for &v in &resampled {
        let t = if v.is_finite() {
            ((v - vmin) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let hue = (1.0 - t) * 240.0;
        let (r, g, b) = hsv_to_rgb(hue as f32, 1.0, 1.0);
        colors.push([r, g, b, 255]);
    }

    (colors, vmin, vmax)
}

/// Get the min/max values of the color channel (for legend display).
pub fn color_channel_range(channel_data: &[f64]) -> (f64, f64) {
    let mut vmin = f64::MAX;
    let mut vmax = f64::MIN;
    for &v in channel_data {
        if v.is_finite() {
            if v < vmin {
                vmin = v;
            }
            if v > vmax {
                vmax = v;
            }
        }
    }
    if vmin > vmax {
        (0.0, 0.0)
    } else {
        (vmin, vmax)
    }
}

/// Resample channel data to match GPS track sample times via linear interpolation.
fn resample_to_track(track: &TrackData, data: &[f64], freq: u16) -> Vec<f64> {
    track
        .time
        .iter()
        .map(|&t| {
            let sample_f = t * freq as f64;
            let idx = sample_f.floor() as usize;
            let frac = sample_f - idx as f64;
            if idx + 1 < data.len() {
                data[idx] * (1.0 - frac) + data[idx + 1] * frac
            } else if idx < data.len() {
                data[idx]
            } else if !data.is_empty() {
                data[data.len() - 1]
            } else {
                0.0
            }
        })
        .collect()
}

/// Nearest valid GPS fix in the requested half-open time interval.
/// Used for picking on a selected lap, so overlapping laps cannot steal a click.
pub fn find_nearest_sample_in_range(
    track: &TrackData,
    x: f64,
    y: f64,
    time_range: (f64, f64),
) -> Option<usize> {
    let start = track.time.partition_point(|t| *t < time_range.0);
    let end = track.time.partition_point(|t| *t < time_range.1);
    (start..end.min(track.x.len()))
        .filter(|&i| track.x[i].is_finite() && track.y[i].is_finite())
        .min_by(|&a, &b| {
            let da = (track.x[a] - x).powi(2) + (track.y[a] - y).powi(2);
            let db = (track.x[b] - x).powi(2) + (track.y[b] - y).powi(2);
            da.total_cmp(&db)
        })
}

/// Compute section times from interpolated nearest approaches to the boundaries.
/// Missing crossings yield NaN rather than a plausible-looking zero/full lap.
pub fn compute_sector_times(
    sectors: &[Sector],
    laps: &[Lap],
    track: &TrackData,
) -> Vec<Vec<SectorTime>> {
    laps.iter()
        .map(|lap| {
            sectors
                .iter()
                .map(|sector| {
                    let interval = sector_interval(sector, lap, track);
                    let time_secs = interval.map_or(f64::NAN, |(start, end)| {
                        if end > start {
                            end - start
                        } else {
                            lap.duration() - (start - end)
                        }
                    });
                    SectorTime {
                        sector_name: sector.name.clone(),
                        lap_name: lap.name.clone(),
                        time_secs,
                    }
                })
                .collect()
        })
        .collect()
}

/// Crossing times within a lap; end < start denotes a section across the beacon.
pub fn sector_interval(sector: &Sector, lap: &Lap, track: &TrackData) -> Option<(f64, f64)> {
    if let Some(section) = &sector.distance {
        return distance_section_interval(section, lap, track);
    }
    Some((
        find_crossing_time(track, sector.start_index, lap)?,
        find_crossing_time(track, sector.end_index, lap)?,
    ))
}

fn find_crossing_time(track: &TrackData, target_index: usize, lap: &Lap) -> Option<f64> {
    let target_x = *track.x.get(target_index)?;
    let target_y = *track.y.get(target_index)?;
    if !target_x.is_finite() || !target_y.is_finite() {
        return None;
    }
    let start = track
        .time
        .partition_point(|t| *t < lap.start_time)
        .saturating_sub(1);
    let end = (track.time.partition_point(|t| *t <= lap.end_time) + 1).min(track.x.len());
    let mut best: Option<(f64, f64)> = None;
    for i in start..end.saturating_sub(1) {
        let (x0, y0, x1, y1) = (track.x[i], track.y[i], track.x[i + 1], track.y[i + 1]);
        let (t0, t1) = (track.time[i], track.time[i + 1]);
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) || t1 <= t0 {
            continue;
        }
        let (dx, dy) = (x1 - x0, y1 - y0);
        let length2 = dx * dx + dy * dy;
        if length2 <= f64::EPSILON {
            continue;
        }
        let lo = ((lap.start_time - t0) / (t1 - t0)).max(0.0);
        let hi = ((lap.end_time - t0) / (t1 - t0)).min(1.0);
        if lo > hi {
            continue;
        }
        let f = (((target_x - x0) * dx + (target_y - y0) * dy) / length2).clamp(lo, hi);
        let dist2 = (x0 + f * dx - target_x).powi(2) + (y0 + f * dy - target_y).powi(2);
        if best.is_none_or(|(d, _)| dist2 < d) {
            best = Some((dist2, t0 + f * (t1 - t0)));
        }
    }
    // Normalized coordinates are degrees. Require a passage within 30 m; a
    // partial lap that never visits this section has no time.
    best.filter(|(d, _)| *d <= (30.0 / METERS_PER_DEGREE).powi(2))
        .map(|(_, t)| t)
}

/// Convert HSV to RGB. Hue in [0, 360), saturation and value in [0, 1].
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_sections_interpolate_wrap_and_only_stretch_qualifying_laps() {
        let mut track = TrackData::from_normalized_parts(
            vec![0.; 101],
            vec![0.; 101],
            (0..101).map(|i| i as f64).collect(),
            1,
        );
        track.distance = Some(((0..101).map(|i| i as f64 * 10.).collect(), 1));
        let lap = Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 0.,
            end_time: 100.,
        };
        let mut section = DistanceSection {
            start_mm: 800_000,
            end_mm: 200_000,
            track_length_mm: 1_000_000,
        };
        assert_eq!(
            distance_section_interval(&section, &lap, &track),
            Some((80., 20.))
        );
        let sector = Sector {
            semantics: SectionSemantics::default(),
            name: "Wrapped".into(),
            start_index: 80,
            end_index: 20,
            distance: Some(section.clone()),
        };
        assert_eq!(
            compute_sector_times(&[sector], std::slice::from_ref(&lap), &track)[0][0].time_secs,
            40.
        );
        section.track_length_mm = 1_040_000;
        let (a, b) = distance_section_interval(&section, &lap, &track).unwrap();
        assert!((a - 80. / 1.04).abs() < 1e-9 && (b - 20. / 1.04).abs() < 1e-9);
        section.track_length_mm = 1_100_000;
        assert_eq!(
            distance_section_interval(&section, &lap, &track),
            Some((80., 20.))
        );
        section.start_mm = 1_050_000;
        assert!(distance_section_interval(&section, &lap, &track).is_none());
        section.track_length_mm = 0;
        assert!(distance_section_interval(&section, &lap, &track).is_none());
    }
    #[test]
    fn vir_definition_covers_the_model_without_gaps_or_overlap() {
        let d = vir_full_definition();
        assert_eq!(d.sections.len(), 33);
        assert_eq!(d.length_m, 5337.5);
        let total: f64 = d
            .sections
            .iter()
            .map(|s| {
                if s.end_m > s.start_m {
                    s.end_m - s.start_m
                } else {
                    d.length_m - s.start_m + s.end_m
                }
            })
            .sum();
        assert_eq!(total, d.length_m);
        for i in 0..d.sections.len() {
            assert_eq!(
                d.sections[i].end_m,
                d.sections[(i + 1) % d.sections.len()].start_m
            );
        }
    }

    #[test]
    fn geographic_sections_survive_a_different_recording_origin_and_indices() {
        let lat = [36.0, 36.001, 36.001, 36.0, 36.0];
        let lon = [-79.0, -79.0, -78.999, -78.999, -79.0];
        let first = normalize_gps_samples(&lat, &lon, 1).unwrap();
        let lap = Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 0.0,
            end_time: 4.1,
        };
        let sectors = split_lap_into_sectors(&first, &lap, 4);
        assert_eq!(sectors.len(), 4);
        let total: f64 = compute_sector_times(&sectors, &[lap], &first)[0]
            .iter()
            .map(|s| s.time_secs)
            .sum();
        assert!((total - 4.1).abs() < 1e-9);
        let definition = export_track_definition(&first, &sectors, "Test circuit".into()).unwrap();
        let mut lat2 = vec![0.0, 36.0005];
        lat2.extend(lat);
        let mut lon2 = vec![0.0, -79.0005];
        lon2.extend(lon);
        let second = normalize_gps_samples(&lat2, &lon2, 1).unwrap();
        let restored = import_track_definition(&second, &definition).unwrap();
        assert_eq!(restored.len(), 4);
        for (a, b) in sectors.iter().zip(restored) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.start_index + 2, b.start_index);
            assert_eq!(a.end_index + 2, b.end_index);
        }
        let distant = normalize_gps_samples(&[37.0, 37.001], &[-80.0, -80.001], 1).unwrap();
        assert!(import_track_definition(&distant, &definition).is_err());
    }

    #[test]
    fn decoded_gps_builder_aligns_rates_and_preserves_origin_and_missing_fixes() {
        let track = gps_track_from_samples(
            &[36.0, 36.001, 0.0, 36.003],
            1,
            &[-79.0, -79.0, -78.999, -78.999, 0.0, 0.0, -78.997],
            2,
        )
        .unwrap();
        assert_eq!(track.time, [0.0, 1.0, 2.0, 3.0]);
        assert!(track.x[2].is_nan() && track.y[2].is_nan());
        assert!(track.x[0] < track.x[1] && track.x[1] < track.x[3]);
        let sections = [Sector {
            semantics: SectionSemantics::default(),
            name: "Straight".into(),
            start_index: 0,
            end_index: 3,
            distance: None,
        }];
        let definition = export_track_definition(&track, &sections, "Test".into()).unwrap();
        assert_eq!(definition.sections[0].start, [36.0, -79.0]);
        assert_eq!(definition.sections[0].end, [36.003, -78.997]);
        assert!(gps_track_from_samples(&[36.0], 0, &[-79.0], 1).is_none());
    }

    #[test]
    fn gps_lost_fixes_are_gaps_and_do_not_change_bounds() {
        let track = normalize_gps_samples(
            &[f64::NAN, 36.56, 0.0, 36.57],
            &[f64::NAN, -79.21, 0.0, -79.20],
            20,
        )
        .unwrap();
        assert!(track.x[0].is_nan() && track.x[2].is_nan());
        assert!(track.y[0].is_nan() && track.y[2].is_nan());
        assert_eq!(track.time, [0.0, 0.05, 0.1, 0.15]);
        assert!(track.x[1].abs() < 0.01 && track.y[3].abs() < 0.01);
        assert_eq!(find_nearest_sample(&track, track.x[3], track.y[3]), 3);
        assert!(normalize_gps_samples(&[0.0], &[0.0], 20).is_none());
        assert!(normalize_gps_samples(&[0.0, 1.0], &[1.0, 0.0], 20).is_some());
    }

    #[test]
    fn map_picking_is_scoped_to_the_requested_lap() {
        let track = TrackData::from_normalized_parts(
            vec![f64::NAN, 0.0, 1.0, 0.0, 1.0],
            vec![f64::NAN, 0.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            1,
        );
        assert_eq!(
            find_nearest_sample_in_range(&track, 0.0, 0.0, (3.0, 5.0)),
            Some(3)
        );
        assert_eq!(
            find_nearest_sample_in_range(&track, 0.0, 0.0, (0.0, 1.0)),
            None
        );
    }

    #[test]
    fn section_crossing_interpolates_and_rejects_missing_passages() {
        let track = TrackData::from_normalized_parts(
            vec![0.00005, 0.00015, f64::NAN, 0.0, 0.0001, 0.0002],
            vec![0.0, 0.0, f64::NAN, 0.0, 0.0, 0.0],
            vec![0.0, 0.05, 0.1, 0.15, 0.2, 0.25],
            20,
        );
        let lap = Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 0.15,
            end_time: 0.25,
        };
        let sector = Sector {
            semantics: Default::default(),
            distance: None,
            name: "Turn 1".into(),
            start_index: 0,
            end_index: 1,
        };
        let (start, end) = sector_interval(&sector, &lap, &track).unwrap();
        assert!((start - 0.175).abs() < 1e-9);
        assert!((end - 0.225).abs() < 1e-9);
        let missing = Sector {
            distance: None,
            start_index: 2,
            ..sector
        };
        assert!(
            compute_sector_times(&[missing], &[lap], &track)[0][0]
                .time_secs
                .is_nan()
        );
    }

    #[test]
    fn test_hsv_to_rgb() {
        // Red
        let (r, g, b) = hsv_to_rgb(0.0, 1.0, 1.0);
        assert_eq!((r, g, b), (255, 0, 0));
        // Green
        let (r, g, b) = hsv_to_rgb(120.0, 1.0, 1.0);
        assert_eq!((r, g, b), (0, 255, 0));
        // Blue
        let (r, g, b) = hsv_to_rgb(240.0, 1.0, 1.0);
        assert_eq!((r, g, b), (0, 0, 255));
    }

    #[test]
    fn test_find_nearest_sample() {
        let track = TrackData::from_normalized_parts(
            vec![0.0, 1.0, 2.0, 3.0],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.05, 0.1, 0.15],
            20,
        );
        assert_eq!(find_nearest_sample(&track, 0.1, 0.1), 0);
        assert_eq!(find_nearest_sample(&track, 2.1, 0.9), 2);
        assert_eq!(find_nearest_sample(&track, 3.0, 1.0), 3);
    }

    #[test]
    fn test_resample_to_track() {
        let track =
            TrackData::from_normalized_parts(vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.5], 2);
        // 10 Hz source data
        let data: Vec<f64> = (0..10).map(|i| i as f64 * 10.0).collect();
        let resampled = resample_to_track(&track, &data, 10);
        assert_eq!(resampled.len(), 2);
        assert!((resampled[0] - 0.0).abs() < 1e-10);
        assert!((resampled[1] - 50.0).abs() < 1e-10);
    }
}

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
    spatial_index: Option<TrackSpatialIndex>,
}

impl TrackData {
    pub fn from_normalized_parts(x: Vec<f64>, y: Vec<f64>, time: Vec<f64>, freq: u16) -> Self {
        let spatial_index = TrackSpatialIndex::build(&x, &y);
        Self {
            x,
            y,
            time,
            freq,
            spatial_index,
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

/// A track sector defined by GPS sample index boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sector {
    pub name: String,
    /// Index into GPS data where this sector starts.
    pub start_index: usize,
    /// Index into GPS data where this sector ends (exclusive).
    pub end_index: usize,
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
    let lat_ch = find_gps_channel(ld, &["latitude", "lat"])?;
    let lon_ch = find_gps_channel(ld, &["longitude", "lon", "long"])?;

    let lat_data = ld.read_channel_data(lat_ch)?;
    let lon_data = ld.read_channel_data(lon_ch)?;

    let n = lat_data.len().min(lon_data.len());
    if n == 0 {
        return None;
    }

    let freq = lat_ch.freq;

    // Compute mean latitude for cos correction
    let mean_lat = lat_data.iter().filter(|v| v.is_finite()).sum::<f64>()
        / lat_data.iter().filter(|v| v.is_finite()).count().max(1) as f64;
    let cos_lat = (mean_lat.to_radians()).cos();

    // Center and scale coordinates
    let mean_lon = lon_data.iter().filter(|v| v.is_finite()).sum::<f64>()
        / lon_data.iter().filter(|v| v.is_finite()).count().max(1) as f64;

    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut time = Vec::with_capacity(n);

    for i in 0..n {
        let lat = lat_data[i];
        let lon = lon_data[i];
        if lat.is_finite() && lon.is_finite() {
            x.push((lon - mean_lon) * cos_lat);
            y.push(lat - mean_lat);
            time.push(i as f64 / freq as f64);
        } else {
            // Preserve index alignment — use previous point or zero
            if let (Some(&prev_x), Some(&prev_y)) = (x.last(), y.last()) {
                x.push(prev_x);
                y.push(prev_y);
            } else {
                x.push(0.0);
                y.push(0.0);
            }
            time.push(i as f64 / freq as f64);
        }
    }

    Some(TrackData::from_normalized_parts(x, y, time, freq))
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

/// Compute sector times for each lap.
///
/// Returns a Vec (one per lap) of Vec<SectorTime> (one per sector).
/// Each sector time is computed by finding when the car is nearest to each sector
/// boundary during each lap.
pub fn compute_sector_times(
    sectors: &[Sector],
    laps: &[Lap],
    track: &TrackData,
) -> Vec<Vec<SectorTime>> {
    if sectors.is_empty() || laps.is_empty() || track.x.is_empty() {
        return Vec::new();
    }

    let freq = track.freq as f64;

    laps.iter()
        .map(|lap| {
            // Sample range for this lap
            let lap_start = (lap.start_time * freq).floor() as usize;
            let lap_end = (lap.end_time * freq).ceil() as usize;
            let lap_end = lap_end.min(track.x.len());

            sectors
                .iter()
                .map(|sector| {
                    // Find when the car crosses the sector start and end boundaries during this lap.
                    // Use nearest-approach to the sector boundary points.
                    let start_time =
                        find_crossing_time(track, sector.start_index, lap_start, lap_end);
                    let end_time = find_crossing_time(track, sector.end_index, lap_start, lap_end);

                    let lap_duration = lap.end_time - lap.start_time;
                    let time_secs = if end_time > start_time {
                        end_time - start_time
                    } else {
                        // Sector wraps around lap boundary
                        lap_duration - (start_time - end_time)
                    }
                    .clamp(0.0, lap_duration);

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

/// Find the time at which the car is nearest to a track position (given by sample index)
/// within a lap's sample range.
fn find_crossing_time(
    track: &TrackData,
    target_index: usize,
    lap_start: usize,
    lap_end: usize,
) -> f64 {
    let target_x = track.x.get(target_index).copied().unwrap_or(0.0);
    let target_y = track.y.get(target_index).copied().unwrap_or(0.0);

    let mut best_dist = f64::MAX;
    let mut best_idx = lap_start;

    let search_start = lap_start.min(track.x.len());
    let search_end = lap_end.min(track.x.len());

    for i in search_start..search_end {
        let dx = track.x[i] - target_x;
        let dy = track.y[i] - target_y;
        let dist = dx * dx + dy * dy;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i;
        }
    }

    track.time.get(best_idx).copied().unwrap_or(0.0)
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

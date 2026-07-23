//! Shared application state accessible by all panels.

use eframe::egui;
use i3rs_core::{
    DownsampledPoint, Lap, LdFile, LdxFile, Sector, TrackData, downsample_minmax,
    format_state_value, is_state_channel,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Identifies a channel: either a physical channel from the .ld file or a math channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelId {
    /// Index into `LdFile::channels`.
    Physical(usize),
    /// Index into `SharedState::math_channels`.
    Math(usize),
}

/// Which Y-axis a channel is assigned to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum YAxis {
    Left,
    Right,
}

/// How a channel's display (Y) range is determined.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ScaleMode {
    /// Range is derived automatically from the channel's data min/max.
    #[default]
    Auto,
    /// Range uses the fixed `manual_min`/`manual_max` values.
    Manual,
}

/// A loaded channel's cached display data.
pub struct PlottedChannel {
    pub channel_id: ChannelId,
    pub color: egui::Color32,
    pub data: Arc<[f64]>,
    pub tile_group: usize,
    pub y_axis: YAxis,
    pub display_scale: f64,
    pub display_offset: f64,
    pub display_unit: Option<String>,
    /// How the display (Y) range is determined for this channel.
    pub scale_mode: ScaleMode,
    /// Fixed lower bound (in display units) used when `scale_mode` is `Manual`.
    pub manual_min: f64,
    /// Fixed upper bound (in display units) used when `scale_mode` is `Manual`.
    pub manual_max: f64,
    /// Cached min value (computed once on load).
    pub cached_min: f64,
    /// Cached max value (computed once on load).
    pub cached_max: f64,
    /// Cached average value (computed once on load).
    pub cached_avg: f64,
}

/// Info about a plotted channel, registered by graph panels each frame for the readout panel.
pub struct PlottedChannelInfo {
    pub name: String,
    pub unit: String,
    pub freq: u16,
    pub dec_places: i16,
    pub color: egui::Color32,
    pub data: Arc<[f64]>,
    pub display_scale: f64,
    pub display_offset: f64,
    /// Enum/state labels parsed from the .ld file (value → label).
    pub enum_labels: Arc<HashMap<i64, String>>,
}

impl PlottedChannelInfo {
    /// Returns true when the channel should be treated as discrete state data.
    pub fn uses_discrete_values(&self) -> bool {
        !self.enum_labels.is_empty() || is_state_channel(&self.name)
    }

    /// Format a value using file-parsed enum labels, falling back to hardcoded labels.
    pub fn format_value(&self, value: f64) -> Option<String> {
        if !self.enum_labels.is_empty() {
            let v = value.round() as i64;
            if let Some(label) = self.enum_labels.get(&v) {
                return Some(label.clone());
            }
        }
        format_state_value(&self.name, value)
    }

    pub fn transform_value(&self, value: f64) -> f64 {
        value * self.display_scale + self.display_offset
    }
}

/// A user-defined math channel.
pub struct MathChannelDef {
    pub id: u64,
    pub name: String,
    pub expression: String,
    pub unit: String,
    pub dec_places: i16,
    pub freq: u16,
    /// Cached evaluation result.
    pub data: Option<Arc<[f64]>>,
    /// Parse or evaluation error.
    pub error: Option<String>,
    pub evaluation_state: MathEvaluationState,
    pub cached_min: f64,
    pub cached_max: f64,
    pub cached_avg: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathEvaluationState {
    Queued,
    WaitingForInputs,
    Running,
    Ready,
    Error,
}

impl MathChannelDef {
    pub fn new(id: u64, name: String, expression: String, unit: String, dec_places: i16) -> Self {
        Self {
            id,
            name,
            expression,
            unit,
            dec_places,
            freq: 0,
            data: None,
            error: None,
            evaluation_state: MathEvaluationState::Queued,
            cached_min: 0.0,
            cached_max: 0.0,
            cached_avg: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChannelStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub stddev: f64,
}

pub struct DecodedChannel {
    pub data: Arc<[f64]>,
    pub stats: ChannelStats,
    #[allow(dead_code)]
    pub freq: u16,
    lod_levels: RwLock<Vec<LodLevel>>,
}

#[derive(Clone)]
pub struct LodLevel {
    pub points: Arc<[DownsampledPoint]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DownsampleSeriesKey {
    pub data_ptr: usize,
    pub data_len: usize,
    pub freq: u16,
    pub start_sample: usize,
    pub end_sample: usize,
    pub target_width: usize,
}

#[derive(Clone)]
pub struct DownsampleSeriesRequest {
    pub key: DownsampleSeriesKey,
    pub data: Arc<[f64]>,
    pub freq: u16,
    pub start_sample: usize,
    pub end_sample: usize,
    pub target_width: usize,
}

impl DecodedChannel {
    pub fn best_lod_level_for_view(
        &self,
        visible_sample_count: usize,
        target_width: usize,
    ) -> Option<LodLevel> {
        if visible_sample_count == 0 || target_width == 0 || self.data.is_empty() {
            return None;
        }

        self.ensure_lod_levels();

        let desired_visible_points = target_width.saturating_mul(2) as f64;
        let visible_fraction = visible_sample_count as f64 / self.data.len().max(1) as f64;

        let levels = self.lod_levels.read().ok()?;
        levels
            .iter()
            .filter_map(|level| {
                let estimated_visible_points = level.points.len() as f64 * visible_fraction;
                (estimated_visible_points >= desired_visible_points)
                    .then_some((estimated_visible_points, level.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, level)| level)
    }

    fn ensure_lod_levels(&self) {
        if let Ok(levels) = self.lod_levels.read()
            && !levels.is_empty()
        {
            return;
        }

        if let Ok(mut levels) = self.lod_levels.write() {
            if !levels.is_empty() {
                return;
            }

            for &target_buckets in &[2_048usize, 8_192, 32_768, 131_072] {
                if self.data.len() > target_buckets.saturating_mul(2) {
                    levels.push(LodLevel {
                        points: Arc::from(downsample_minmax(
                            &self.data,
                            self.freq,
                            0,
                            target_buckets,
                        )),
                    });
                }
            }
        }
    }
}

/// Compute min, max, avg, stddev for a slice of finite f64 values.
pub fn compute_channel_stats(data: &[f64]) -> ChannelStats {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut sum = 0.0;
    let mut count = 0u64;

    for &v in data {
        if v.is_finite() {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            sum += v;
            count += 1;
        }
    }

    if count == 0 {
        return ChannelStats::default();
    }

    let avg = sum / count as f64;
    let mut var_sum = 0.0;
    for &v in data {
        if v.is_finite() {
            let diff = v - avg;
            var_sum += diff * diff;
        }
    }
    let stddev = (var_sum / count as f64).sqrt();

    ChannelStats {
        min,
        max,
        avg,
        stddev,
    }
}

/// Graph display mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphMode {
    /// All channels overlaid on one graph.
    Overlay,
    /// Each channel in its own vertically stacked tile.
    Tiled,
}

/// Which domain graph panels use for their X-axis.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphXAxis {
    Time,
    Distance,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelPreference {
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    #[serde(default = "default_display_scale")]
    pub display_scale: f64,
    #[serde(default)]
    pub display_offset: f64,
    #[serde(default)]
    pub display_unit: Option<String>,
    /// Fixed display range `(min, max)` in display units. `None` = auto-scale.
    #[serde(default)]
    pub scale_manual: Option<(f64, f64)>,
}

fn default_display_scale() -> f64 {
    1.0
}

impl Default for ChannelPreference {
    fn default() -> Self {
        Self {
            color: None,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            scale_manual: None,
        }
    }
}

pub fn channel_preference_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

#[derive(Clone)]
pub struct DistanceAxisCache {
    pub data: Arc<[f64]>,
    pub freq: u16,
}

pub const CHANNEL_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(255, 100, 100), // red
    egui::Color32::from_rgb(100, 180, 255), // blue
    egui::Color32::from_rgb(100, 255, 100), // green
    egui::Color32::from_rgb(255, 200, 50),  // yellow
    egui::Color32::from_rgb(200, 100, 255), // purple
    egui::Color32::from_rgb(255, 150, 50),  // orange
    egui::Color32::from_rgb(50, 255, 200),  // cyan
    egui::Color32::from_rgb(255, 100, 200), // pink
];

/// Cached statistics for a single channel (session + per-lap).
pub struct CachedChannelStats {
    pub name: String,
    pub color: egui::Color32,
    pub dec_places: i16,
    /// Full session stats.
    pub session: ChannelStats,
    /// Per-lap stats.
    pub per_lap: Vec<(String, ChannelStats)>,
}

/// Cache for report panel statistics, invalidated when channels or laps change.
pub struct ReportCache {
    /// Fingerprint: (channel_name, data_ptr, data_len, unit, scale_bits, offset_bits).
    fingerprint: Vec<(String, usize, usize, String, u64, u64)>,
    lap_count: usize,
    pub stats: Vec<CachedChannelStats>,
}

impl ReportCache {
    pub fn new() -> Self {
        Self {
            fingerprint: Vec::new(),
            lap_count: 0,
            stats: Vec::new(),
        }
    }

    /// Returns true if the cache is still valid for the current display state.
    pub fn is_valid(&self, registry: &[PlottedChannelInfo], lap_count: usize) -> bool {
        if self.lap_count != lap_count || self.fingerprint.len() != registry.len() {
            return false;
        }
        for (i, info) in registry.iter().enumerate() {
            let ptr = info.data.as_ptr() as usize;
            let (ref name, cached_ptr, cached_len, ref unit, scale_bits, offset_bits) =
                self.fingerprint[i];
            if name != &info.name
                || cached_ptr != ptr
                || cached_len != info.data.len()
                || unit != &info.unit
                || scale_bits != info.display_scale.to_bits()
                || offset_bits != info.display_offset.to_bits()
            {
                return false;
            }
        }
        true
    }

    /// Rebuild the cache from current display state and laps.
    pub fn rebuild(&mut self, registry: &[PlottedChannelInfo], laps: &[Lap]) {
        self.fingerprint = registry
            .iter()
            .map(|info| {
                (
                    info.name.clone(),
                    info.data.as_ptr() as usize,
                    info.data.len(),
                    info.unit.clone(),
                    info.display_scale.to_bits(),
                    info.display_offset.to_bits(),
                )
            })
            .collect();
        self.lap_count = laps.len();
        self.stats.clear();

        for info in registry {
            let mut session = compute_channel_stats(&info.data);
            session.min = info.transform_value(session.min);
            session.max = info.transform_value(session.max);
            if info.display_scale < 0.0 {
                std::mem::swap(&mut session.min, &mut session.max);
            }
            session.avg = info.transform_value(session.avg);
            session.stddev *= info.display_scale.abs();
            let freq = info.freq;
            let mut per_lap = Vec::with_capacity(laps.len());

            for lap in laps {
                let start_sample = (lap.start_time * freq as f64).floor() as usize;
                let end_sample = (lap.end_time * freq as f64).ceil() as usize;
                let start = start_sample.min(info.data.len());
                let end = end_sample.min(info.data.len());
                if start < end {
                    let mut stats = compute_channel_stats(&info.data[start..end]);
                    stats.min = info.transform_value(stats.min);
                    stats.max = info.transform_value(stats.max);
                    if info.display_scale < 0.0 {
                        std::mem::swap(&mut stats.min, &mut stats.max);
                    }
                    stats.avg = info.transform_value(stats.avg);
                    stats.stddev *= info.display_scale.abs();
                    per_lap.push((lap.name.clone(), stats));
                }
            }

            self.stats.push(CachedChannelStats {
                name: info.name.clone(),
                color: info.color,
                dec_places: info.dec_places,
                session,
                per_lap,
            });
        }
    }
}

/// State shared across all panels.
pub struct SharedState {
    pub session_id: u64,
    pub ld_file: Option<Arc<LdFile>>,
    pub ld_path: Option<PathBuf>,
    pub file_name: String,

    // Lap data
    pub laps: Vec<Lap>,
    pub ldx: Option<LdxFile>,
    pub selected_lap: Option<usize>,
    pub show_lap_markers: bool,

    // Cross-panel synchronization
    pub cursor_time: Option<f64>,
    pub zoom_range: Option<(f64, f64)>,
    pub data_duration: Option<f64>,
    /// Set to true when the timeline (or other external control) changes the zoom.
    /// Graphs read this to apply the zoom, then clear it.
    pub zoom_from_timeline: bool,

    pub plotted_channel_registry: Vec<PlottedChannelInfo>,
    pub display_channel_registry: Vec<PlottedChannelInfo>,

    // Channel browser
    pub channel_filter: String,
    pub dragging_channel: Option<ChannelId>,

    // Channels pending addition (set by browser, consumed by graph panels)
    pub pending_toggle_channel: Option<ChannelId>,

    // Math channels
    pub math_channels: Vec<MathChannelDef>,

    // Channel aliases: maps alias name → canonical channel name
    pub channel_aliases: HashMap<String, String>,

    // Global channel display preferences keyed by canonicalized channel name.
    pub channel_preferences: HashMap<String, ChannelPreference>,
    pub channel_preferences_dirty: bool,

    // Report cache
    pub report_cache: ReportCache,

    // Track map: sectors and reference lap
    pub sectors: Vec<Sector>,
    pub reference_lap: Option<usize>,

    // Next panel ID counter
    pub next_panel_id: u64,
    next_math_channel_id: u64,

    // Derived channel caches
    pub distance_axis_cache: Option<DistanceAxisCache>,
    pub decoded_channel_cache: RefCell<HashMap<usize, Arc<DecodedChannel>>>,
    pending_channel_decodes: RefCell<HashSet<usize>>,
    requested_channel_decodes: RefCell<Vec<usize>>,
    pending_math_evaluations: RefCell<HashSet<u64>>,
    requested_math_evaluations: RefCell<Vec<u64>>,
    downsampled_series_cache: RefCell<HashMap<DownsampleSeriesKey, Arc<[DownsampledPoint]>>>,
    pending_downsampled_series: RefCell<HashSet<DownsampleSeriesKey>>,
    requested_downsampled_series: RefCell<Vec<DownsampleSeriesRequest>>,
    track_data_cache: RefCell<Option<Arc<TrackData>>>,
    pending_track_data_build: RefCell<bool>,
    requested_track_data_build: RefCell<bool>,
    resolved_track_data_build: RefCell<bool>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            session_id: 0,
            ld_file: None,
            ld_path: None,
            file_name: String::new(),
            laps: Vec::new(),
            ldx: None,
            selected_lap: None,
            show_lap_markers: true,
            cursor_time: None,
            zoom_range: None,
            data_duration: None,
            zoom_from_timeline: false,
            plotted_channel_registry: Vec::new(),
            display_channel_registry: Vec::new(),
            channel_filter: String::new(),
            dragging_channel: None,
            pending_toggle_channel: None,
            math_channels: Vec::new(),
            channel_aliases: HashMap::new(),
            channel_preferences: HashMap::new(),
            channel_preferences_dirty: false,
            report_cache: ReportCache::new(),
            sectors: Vec::new(),
            reference_lap: None,
            next_panel_id: 1,
            next_math_channel_id: 1,
            distance_axis_cache: None,
            decoded_channel_cache: RefCell::new(HashMap::new()),
            pending_channel_decodes: RefCell::new(HashSet::new()),
            requested_channel_decodes: RefCell::new(Vec::new()),
            pending_math_evaluations: RefCell::new(HashSet::new()),
            requested_math_evaluations: RefCell::new(Vec::new()),
            downsampled_series_cache: RefCell::new(HashMap::new()),
            pending_downsampled_series: RefCell::new(HashSet::new()),
            requested_downsampled_series: RefCell::new(Vec::new()),
            track_data_cache: RefCell::new(None),
            pending_track_data_build: RefCell::new(false),
            requested_track_data_build: RefCell::new(false),
            resolved_track_data_build: RefCell::new(false),
        }
    }

    pub fn invalidate_derived_caches(&mut self) {
        self.distance_axis_cache = None;
        self.downsampled_series_cache.borrow_mut().clear();
        self.pending_downsampled_series.borrow_mut().clear();
        self.requested_downsampled_series.borrow_mut().clear();
    }

    pub fn invalidate_session_caches(&mut self) {
        self.invalidate_derived_caches();
        self.decoded_channel_cache.borrow_mut().clear();
        self.pending_channel_decodes.borrow_mut().clear();
        self.requested_channel_decodes.borrow_mut().clear();
        self.pending_math_evaluations.borrow_mut().clear();
        self.requested_math_evaluations.borrow_mut().clear();
        self.track_data_cache.borrow_mut().take();
        *self.pending_track_data_build.borrow_mut() = false;
        *self.requested_track_data_build.borrow_mut() = false;
        *self.resolved_track_data_build.borrow_mut() = false;
    }

    pub fn downsampled_series_if_ready(
        &self,
        key: &DownsampleSeriesKey,
    ) -> Option<Arc<[DownsampledPoint]>> {
        self.downsampled_series_cache
            .borrow()
            .get(key)
            .map(Arc::clone)
    }

    pub fn request_downsampled_series(&self, request: DownsampleSeriesRequest) {
        if self
            .downsampled_series_cache
            .borrow()
            .contains_key(&request.key)
        {
            return;
        }

        let mut pending = self.pending_downsampled_series.borrow_mut();
        if !pending.insert(request.key.clone()) {
            return;
        }

        self.requested_downsampled_series.borrow_mut().push(request);
    }

    pub fn take_requested_downsampled_series(&self) -> Vec<DownsampleSeriesRequest> {
        let mut requested = self.requested_downsampled_series.borrow_mut();
        std::mem::take(&mut *requested)
    }

    pub fn request_math_channel_evaluation_by_id(&self, math_id: u64) {
        let mut pending = self.pending_math_evaluations.borrow_mut();
        if !pending.insert(math_id) {
            return;
        }
        self.requested_math_evaluations.borrow_mut().push(math_id);
    }

    pub fn take_requested_math_channel_evaluations(&self) -> Vec<u64> {
        let mut requested = self.requested_math_evaluations.borrow_mut();
        std::mem::take(&mut *requested)
    }

    pub fn complete_math_channel_evaluation(&self, math_id: u64) {
        self.pending_math_evaluations.borrow_mut().remove(&math_id);
    }

    pub fn cancel_math_channel_evaluation(&self, math_id: u64) {
        self.pending_math_evaluations.borrow_mut().remove(&math_id);
    }

    pub fn has_pending_math_evaluations(&self) -> bool {
        !self.pending_math_evaluations.borrow().is_empty()
            || !self.requested_math_evaluations.borrow().is_empty()
    }

    pub fn are_math_channels_settled(&self) -> bool {
        !self.has_pending_math_evaluations()
            && self.math_channels.iter().all(|mc| {
                matches!(
                    mc.evaluation_state,
                    MathEvaluationState::Ready | MathEvaluationState::Error
                )
            })
    }

    pub fn create_math_channel_def(
        &mut self,
        name: String,
        expression: String,
        unit: String,
        dec_places: i16,
    ) -> MathChannelDef {
        let id = self.next_math_channel_id;
        self.next_math_channel_id += 1;
        MathChannelDef::new(id, name, expression, unit, dec_places)
    }

    pub fn math_channel_index_by_id(&self, math_id: u64) -> Option<usize> {
        self.math_channels.iter().position(|mc| mc.id == math_id)
    }

    pub fn store_downsampled_series(
        &self,
        key: DownsampleSeriesKey,
        points: Vec<DownsampledPoint>,
    ) {
        self.pending_downsampled_series.borrow_mut().remove(&key);
        self.downsampled_series_cache
            .borrow_mut()
            .insert(key, Arc::from(points));
    }

    pub fn cancel_downsampled_series(&self, key: &DownsampleSeriesKey) {
        self.pending_downsampled_series.borrow_mut().remove(key);
    }

    pub fn track_data_if_ready(&self) -> Option<Arc<TrackData>> {
        self.track_data_cache.borrow().as_ref().map(Arc::clone)
    }

    pub fn request_track_data_build(&self) {
        if self.track_data_cache.borrow().is_some()
            || *self.pending_track_data_build.borrow()
            || *self.resolved_track_data_build.borrow()
        {
            return;
        }

        *self.pending_track_data_build.borrow_mut() = true;
        *self.requested_track_data_build.borrow_mut() = true;
    }

    pub fn take_requested_track_data_build(&self) -> bool {
        let mut requested = self.requested_track_data_build.borrow_mut();
        let was_requested = *requested;
        *requested = false;
        was_requested
    }

    pub fn is_track_data_build_pending(&self) -> bool {
        *self.pending_track_data_build.borrow()
    }

    pub fn store_track_data(&self, track_data: Option<TrackData>) {
        *self.pending_track_data_build.borrow_mut() = false;
        *self.resolved_track_data_build.borrow_mut() = true;
        *self.track_data_cache.borrow_mut() = track_data.map(Arc::new);
    }

    pub fn cancel_track_data_build(&self) {
        *self.pending_track_data_build.borrow_mut() = false;
    }

    pub fn decoded_physical_channel_if_ready(
        &self,
        channel_idx: usize,
    ) -> Option<Arc<DecodedChannel>> {
        self.decoded_channel_cache
            .borrow()
            .get(&channel_idx)
            .map(Arc::clone)
    }

    pub fn request_physical_channel_decode(&self, channel_idx: usize) {
        if self
            .decoded_channel_cache
            .borrow()
            .contains_key(&channel_idx)
        {
            return;
        }

        let mut pending = self.pending_channel_decodes.borrow_mut();
        if !pending.insert(channel_idx) {
            return;
        }

        self.requested_channel_decodes
            .borrow_mut()
            .push(channel_idx);
    }

    pub fn take_requested_physical_channel_decodes(&self) -> Vec<usize> {
        let mut requested = self.requested_channel_decodes.borrow_mut();
        std::mem::take(&mut *requested)
    }

    pub fn store_decoded_physical_channel(
        &self,
        channel_idx: usize,
        data: Vec<f64>,
        stats: ChannelStats,
        freq: u16,
    ) {
        self.pending_channel_decodes
            .borrow_mut()
            .remove(&channel_idx);
        self.decoded_channel_cache.borrow_mut().insert(
            channel_idx,
            Arc::new(DecodedChannel {
                data: Arc::from(data),
                stats,
                freq,
                lod_levels: RwLock::new(Vec::new()),
            }),
        );
    }

    pub fn cancel_physical_channel_decode(&self, channel_idx: usize) {
        self.pending_channel_decodes
            .borrow_mut()
            .remove(&channel_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::SharedState;

    #[test]
    fn canceled_channel_decode_can_be_requested_again() {
        let shared = SharedState::new();

        shared.request_physical_channel_decode(7);
        assert_eq!(shared.take_requested_physical_channel_decodes(), vec![7]);

        shared.cancel_physical_channel_decode(7);
        shared.request_physical_channel_decode(7);

        assert_eq!(shared.take_requested_physical_channel_decodes(), vec![7]);
    }
}

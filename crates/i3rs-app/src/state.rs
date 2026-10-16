//! Shared application state accessible by all panels.

use eframe::egui;
use i3rs_core::{Lap, LdFile, LdxFile, Sector, TrackData};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::panels::utils::{format_enum_value, uses_discrete_values};

/// Identifies a channel: either a physical channel from the .ld file or a math channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelId {
    /// A configured slot whose source is not currently available.
    Unresolved(usize),
    /// Portable reference to a W02 constant or W03 generated channel.
    Analysis(usize),
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
    pub portable_source_unit: Option<String>,
    pub requested_reference: Option<(String, bool)>,
    pub channel_id: ChannelId,
    pub color: egui::Color32,
    pub data: Arc<[f64]>,
    pub tile_group: usize,
    pub y_axis: YAxis,
    pub display_scale: f64,
    pub display_offset: f64,
    pub display_unit: Option<String>,
    pub math_id: Option<u64>,
    pub math_display_signature: Option<(String, Option<String>)>,
    pub recording_display_base: Option<(f64, f64, Option<String>)>,
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
#[derive(Clone)]
pub struct PlottedChannelInfo {
    pub channel_id: ChannelId,
    pub name: String,
    pub unit: String,
    pub freq: u16,
    pub dec_places: i16,
    pub color: egui::Color32,
    pub data: Arc<[f64]>,
    pub display_scale: f64,
    pub display_offset: f64,
    pub interpolation: i3rs_core::channels::Interpolation,
    /// Enum/state labels parsed from the .ld file (value → label).
    pub enum_labels: Arc<HashMap<i64, String>>,
}

impl PlottedChannelInfo {
    /// Returns true when the channel should be treated as discrete state data.
    pub fn uses_discrete_values(&self) -> bool {
        uses_discrete_values(&self.name, &self.enum_labels)
    }

    /// Format a value using file-parsed enum labels, falling back to hardcoded labels.
    pub fn format_value(&self, value: f64) -> Option<String> {
        format_enum_value(&self.name, &self.enum_labels, value)
    }

    pub fn transform_value(&self, value: f64) -> f64 {
        value * self.display_scale + self.display_offset
    }

    /// Export the same values, units and precision used by the readout.
    pub fn export_channel(&self) -> i3rs_core::ExportChannel<'_> {
        i3rs_core::ExportChannel {
            name: &self.name,
            unit: &self.unit,
            data: &self.data,
            freq: self.freq,
            dec_places: self.dec_places,
            display_scale: self.display_scale,
            display_offset: self.display_offset,
        }
    }
}

/// A user-defined math channel.
pub struct MathChannelDef {
    pub id: u64,
    pub name: String,
    pub expression: String,
    pub options: i3rs_core::channels::MathOptions,
    pub evaluation_key: Option<String>,
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
            options: Default::default(),
            evaluation_key: None,
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
}

pub struct DecodedChannel {
    pub data: Arc<[f64]>,
    pub stats: ChannelStats,
}

/// Compute the cached graph overview statistics for finite values.
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
    ChannelStats { min, max, avg }
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

type AnalysisConstantCache = HashMap<usize, (u64, usize, Arc<[f64]>)>;

/// State shared across all panels.
pub struct SharedState {
    analysis_constant_cache: RefCell<AnalysisConstantCache>,
    pub unresolved_references: RefCell<Vec<(String, bool)>>,
    pub comparisons: crate::comparison::Comparisons,
    pub measurement_cursor: Option<f64>,
    pub session_id: u64,
    pub ld_file: Option<Arc<LdFile>>,
    pub ld_path: Option<PathBuf>,
    pub file_name: String,

    // Lap data
    pub recording_analyses:
        std::collections::BTreeMap<String, i3rs_core::ranges::RecordingAnalysis>,
    pub correction_gps_speed: Option<(Arc<[f64]>, u16)>,
    pub correction_speed: Option<(Arc<[f64]>, u16)>,
    pub correction_distance: Option<(Arc<[f64]>, u16)>,
    pub correction_error: Option<String>,
    pub correction_revision: u64,
    pub show_recording_editor: bool,
    pub laps: Vec<Lap>,
    pub ldx: Option<LdxFile>,
    pub selected_lap: Option<usize>,
    pub show_lap_markers: bool,

    // Cross-panel synchronization
    pub cursor_time: Option<f64>,
    pub playback_active: bool,
    pub playback_speed: f64,
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
    pub channel_settings: i3rs_core::channels::ChannelSettings,
    pub analysis_messages: crate::channel_analysis::AnalysisMessages,
    pub channel_errors: RefCell<HashMap<usize, String>>,
    pub values_all_channels: bool,
    pub values_filter: String,
    pub show_channel_status: bool,

    // Global channel display preferences keyed by canonicalized channel name.
    pub channel_preferences: HashMap<String, ChannelPreference>,
    pub channel_preferences_dirty: bool,

    // Track map: sectors and reference lap
    pub sectors: Vec<Sector>,
    pub pending_track_definition: Option<i3rs_core::TrackDefinition>,
    pub track_definition_error: Option<String>,
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
    track_data_cache: RefCell<Option<Arc<TrackData>>>,
    pending_track_data_build: RefCell<bool>,
    requested_track_data_build: RefCell<bool>,
    resolved_track_data_build: RefCell<bool>,
}

impl SharedState {
    pub fn unresolved_channel(&self, name: &str, is_math: bool) -> ChannelId {
        let mut refs = self.unresolved_references.borrow_mut();
        let index = refs
            .iter()
            .position(|r| r.0 == name && r.1 == is_math)
            .unwrap_or_else(|| {
                refs.push((name.into(), is_math));
                refs.len() - 1
            });
        ChannelId::Unresolved(index)
    }
    pub fn analysis_channel(&self, name: &str) -> ChannelId {
        let ChannelId::Unresolved(index) = self.unresolved_channel(name, false) else {
            unreachable!()
        };
        ChannelId::Analysis(index)
    }
    pub fn analysis_series(&self, index: usize) -> Option<(Arc<[f64]>, u16)> {
        let reference = self.unresolved_references.borrow().get(index)?.0.clone();
        match self.resolve_analysis_reference(&reference) {
            i3rs_core::ChannelResolution::Resolved {
                key: i3rs_core::ChannelKey::Constant(i),
                ..
            } => {
                let c = &self.channel_settings.constants[i];
                let len = self.data_duration.unwrap_or(0.).ceil().max(0.) as usize;
                let mut cache = self.analysis_constant_cache.borrow_mut();
                let entry = cache
                    .entry(i)
                    .or_insert_with(|| (c.value.to_bits(), len, Arc::from(vec![c.value; len])));
                if entry.0 != c.value.to_bits() || entry.1 != len {
                    *entry = (c.value.to_bits(), len, Arc::from(vec![c.value; len]));
                }
                Some((entry.2.clone(), 1))
            }
            i3rs_core::ChannelResolution::Resolved {
                key: i3rs_core::ChannelKey::Generated(name),
                ..
            } => {
                self.request_track_data_build();
                match name.as_str() {
                    "Corr Speed" => self.correction_speed.clone(),
                    "Corr GPS Speed" => self.correction_gps_speed.clone(),
                    _ => self.correction_distance.clone(),
                }
            }
            _ => None,
        }
    }
    pub fn resolve_slot(&self, name: &str, is_math: bool) -> Option<ChannelId> {
        if is_math {
            return self.resolved_math_index(name).map(ChannelId::Math);
        }
        match self.resolve_analysis_reference(name) {
            i3rs_core::ChannelResolution::Resolved {
                key: i3rs_core::ChannelKey::Physical(i),
                ..
            } => Some(ChannelId::Physical(i)),
            i3rs_core::ChannelResolution::Resolved {
                key: i3rs_core::ChannelKey::Derived(id),
                ..
            } => self.math_channel_index_by_id(id).map(ChannelId::Math),
            i3rs_core::ChannelResolution::Resolved {
                key: i3rs_core::ChannelKey::Constant(_) | i3rs_core::ChannelKey::Generated(_),
                ..
            } => Some(self.analysis_channel(name)),
            _ => None,
        }
    }

    pub fn new() -> Self {
        Self {
            analysis_constant_cache: Default::default(),
            unresolved_references: Default::default(),
            comparisons: Default::default(),
            measurement_cursor: None,
            session_id: 0,
            recording_analyses: Default::default(),
            correction_gps_speed: None,
            correction_speed: None,
            correction_distance: None,
            correction_error: None,
            correction_revision: 0,
            show_recording_editor: false,
            ld_file: None,
            ld_path: None,
            file_name: String::new(),
            laps: Vec::new(),
            ldx: None,
            selected_lap: None,
            show_lap_markers: true,
            cursor_time: None,
            playback_active: false,
            playback_speed: 1.0,
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
            channel_settings: Default::default(),
            analysis_messages: Default::default(),
            channel_errors: RefCell::new(HashMap::new()),
            values_all_channels: false,
            values_filter: String::new(),
            show_channel_status: false,
            channel_preferences: HashMap::new(),
            channel_preferences_dirty: false,
            sectors: Vec::new(),
            pending_track_definition: None,
            track_definition_error: None,
            reference_lap: None,
            next_panel_id: 1,
            next_math_channel_id: 1,
            distance_axis_cache: None,
            decoded_channel_cache: RefCell::new(HashMap::new()),
            pending_channel_decodes: RefCell::new(HashSet::new()),
            requested_channel_decodes: RefCell::new(Vec::new()),
            pending_math_evaluations: RefCell::new(HashSet::new()),
            requested_math_evaluations: RefCell::new(Vec::new()),
            track_data_cache: RefCell::new(None),
            pending_track_data_build: RefCell::new(false),
            requested_track_data_build: RefCell::new(false),
            resolved_track_data_build: RefCell::new(false),
        }
    }

    pub fn invalidate_derived_caches(&mut self) {
        self.distance_axis_cache = None;
    }

    pub fn invalidate_correction_caches(&mut self) {
        self.correction_revision += 1;
        self.correction_gps_speed = None;
        self.correction_speed = None;
        self.correction_distance = None;
        self.correction_error = None;
        self.invalidate_derived_caches();
        self.track_data_cache.borrow_mut().take();
        *self.pending_track_data_build.borrow_mut() = false;
        *self.requested_track_data_build.borrow_mut() = false;
        *self.resolved_track_data_build.borrow_mut() = false;
        self.request_track_data_build();
    }

    pub fn invalidate_session_caches(&mut self) {
        self.analysis_constant_cache.borrow_mut().clear();
        self.correction_gps_speed = None;
        self.correction_speed = None;
        self.correction_distance = None;
        self.correction_error = None;
        self.correction_revision += 1;
        self.invalidate_derived_caches();
        self.decoded_channel_cache.borrow_mut().clear();
        self.channel_errors.borrow_mut().clear();
        self.pending_channel_decodes.borrow_mut().clear();
        self.requested_channel_decodes.borrow_mut().clear();
        self.pending_math_evaluations.borrow_mut().clear();
        self.requested_math_evaluations.borrow_mut().clear();
        self.track_data_cache.borrow_mut().take();
        *self.pending_track_data_build.borrow_mut() = false;
        *self.requested_track_data_build.borrow_mut() = false;
        *self.resolved_track_data_build.borrow_mut() = false;
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

    pub fn corrections_settled(&self) -> bool {
        *self.resolved_track_data_build.borrow()
    }

    pub fn is_track_data_build_pending(&self) -> bool {
        *self.pending_track_data_build.borrow()
    }

    pub fn store_track_data(&mut self, track_data: Option<TrackData>) {
        *self.pending_track_data_build.borrow_mut() = false;
        *self.resolved_track_data_build.borrow_mut() = true;
        *self.track_data_cache.borrow_mut() = track_data.map(Arc::new);
        self.apply_pending_track_definition();
    }

    pub fn apply_pending_track_definition(&mut self) {
        if let Some(track) = self.track_data_if_ready()
            && let Some(definition) = self.pending_track_definition.take()
        {
            match i3rs_core::import_track_definition(&track, &definition) {
                Ok(sectors) => {
                    self.sectors = sectors;
                    self.track_definition_error = None;
                }
                Err(error) => {
                    self.sectors.clear();
                    self.track_definition_error = Some(error);
                }
            }
        }
    }

    /// The zoomed range, or the whole session when nothing is zoomed.
    pub fn analysis_range(&self) -> (f64, f64) {
        self.zoom_range
            .unwrap_or((0.0, self.data_duration.unwrap_or(0.0)))
    }

    /// Move the cursor to `start` unless it already lies in `[start, end)`.
    pub fn clamp_cursor_to(&mut self, (start, end): (f64, f64)) {
        if self.cursor_time.is_none_or(|t| t < start || t >= end) {
            self.cursor_time = Some(start);
        }
    }

    /// Select a lap (or the whole session) and zoom every panel to it.
    pub fn select_lap(&mut self, lap: Option<usize>) {
        self.selected_lap = lap;
        let key = self.recording_key().to_string();
        if let Some(model) = self.recording_analyses.get_mut(&key) {
            model.selected_range = lap.and_then(|i| model.ranges.get(i)).map(|r| r.id.clone());
        }
        self.zoom_from_timeline = true;
        self.zoom_range = lap
            .and_then(|index| self.laps.get(index))
            .map(|lap| (lap.start_time, lap.end_time));
        if let Some(range) = self.zoom_range {
            self.clamp_cursor_to(range);
        }
    }

    /// An explicit cursor jump (e.g. a click) stops playback.
    pub fn seek(&mut self, time: f64) {
        self.playback_active = false;
        self.cursor_time = Some(time);
    }

    /// Hovering a stationary pointer must not override the playback clock.
    pub fn set_hover_cursor(&mut self, time: f64) {
        if !self.playback_active {
            self.cursor_time = Some(time);
        }
    }

    pub fn advance_playback(&mut self, elapsed: f64) {
        if !self.playback_active {
            return;
        }
        let (start, end) = self.analysis_range();
        if !start.is_finite() || !end.is_finite() || end <= start {
            self.playback_active = false;
            return;
        }
        let time = self
            .cursor_time
            .filter(|t| t.is_finite())
            .unwrap_or(start)
            .clamp(start, end);
        let next =
            (time + elapsed.clamp(0.0, 0.25) * self.playback_speed.clamp(0.1, 16.0)).min(end);
        self.cursor_time = Some(next);
        if next >= end {
            self.playback_active = false;
        }
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
        if self.channel_errors.borrow().contains_key(&channel_idx) {
            return;
        }
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
    ) {
        self.pending_channel_decodes
            .borrow_mut()
            .remove(&channel_idx);
        self.decoded_channel_cache.borrow_mut().insert(
            channel_idx,
            Arc::new(DecodedChannel {
                data: Arc::from(data),
                stats,
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
    fn readout_and_csv_share_display_conversion_and_metadata() {
        let mut shared = SharedState::new();
        let mut definition =
            shared.create_math_channel_def("Temperature".into(), "0".into(), "C".into(), 1);
        definition.freq = 1;
        definition.dec_places = 1;
        definition.data = Some(std::sync::Arc::from([0.0, 100.0, f64::NAN]));
        shared.math_channels.push(definition);
        let mut plotted =
            crate::panels::utils::create_plotted_channel(super::ChannelId::Math(0), &shared, 0)
                .unwrap();
        let preset = crate::panels::utils::display_presets_for_unit("C")[0];
        plotted.display_scale = preset.scale;
        plotted.display_offset = preset.offset;
        plotted.display_unit = Some(preset.unit.into());
        let info = crate::panels::utils::build_plotted_channel_info(&plotted, &shared);
        let path = std::env::temp_dir().join("i3rs_gui_export_display.csv");
        i3rs_core::export_csv(&path, &[info.export_channel()], None).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().next(), Some("Time (s),Temperature (°F)"));
        for (row, raw) in csv.lines().skip(1).zip(info.data.iter()) {
            let cell = row.split(',').nth(1).unwrap();
            if raw.is_finite() {
                assert_eq!(cell, format!("{:.1}", info.transform_value(*raw)));
            } else {
                assert!(cell.is_empty());
            }
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn playback_respects_range_speed_and_hover_and_stops_at_end() {
        let mut shared = SharedState::new();
        shared.zoom_range = Some((10.0, 11.0));
        shared.playback_active = true;
        shared.playback_speed = 2.0;
        shared.advance_playback(0.2);
        assert_eq!(shared.cursor_time, Some(10.4));
        shared.set_hover_cursor(99.0);
        assert_eq!(shared.cursor_time, Some(10.4));
        shared.advance_playback(0.2);
        shared.advance_playback(0.2);
        assert_eq!(shared.cursor_time, Some(11.0));
        assert!(!shared.playback_active);
        shared.set_hover_cursor(10.5);
        assert_eq!(shared.cursor_time, Some(10.5));
    }

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

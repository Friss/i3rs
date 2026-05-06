//! Graph panel: time-series plotting with overlay and tiled modes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, VLine};
use i3rs_core::{
    Lap, LdFile, detect_laps, downsample_minmax, format_state_value, is_state_channel,
};

use crate::state::{
    CHANNEL_COLORS, ChannelId, ChannelPreference, DistanceAxisCache, DownsampleSeriesKey,
    DownsampleSeriesRequest, GraphMode, GraphXAxis, PlottedChannel, SharedState, YAxis,
    channel_preference_key, compute_channel_stats,
};

use super::gauge::{
    GaugeChannel, GaugeDrawContext, GaugeStyle, best_gauge_grid, default_style_for_name, draw_gauge,
};
use super::utils::{
    ChannelDisplayMeta, DisplayTransformFingerprint, build_plotted_channel_info,
    create_plotted_channel, display_transform_fingerprint, interp_at_time, refresh_plotted_channel,
    resolve_channel_display_meta, resolve_channel_meta, resolve_plotted_channel_display_meta,
};

/// Format a value using file-parsed enum labels, falling back to hardcoded labels.
fn format_enum_value(name: &str, enum_labels: &HashMap<i64, String>, value: f64) -> Option<String> {
    if !enum_labels.is_empty() {
        let v = value.round() as i64;
        if let Some(label) = enum_labels.get(&v) {
            return Some(label.clone());
        }
    }
    format_state_value(name, value)
}

fn uses_discrete_values(name: &str, enum_labels: &HashMap<i64, String>) -> bool {
    !enum_labels.is_empty() || is_state_channel(name)
}

/// Build a freq map for a set of plotted channels (used to pass into closures).
fn build_freq_map(channels: &[&PlottedChannel], shared: &SharedState) -> HashMap<ChannelId, u16> {
    channels
        .iter()
        .map(|pc| {
            let freq = match pc.channel_id {
                ChannelId::Physical(idx) => shared
                    .ld_file
                    .as_ref()
                    .and_then(|ld| ld.channels.get(idx))
                    .map_or(0, |ch| ch.freq),
                ChannelId::Math(idx) => shared.math_channels.get(idx).map_or(0, |mc| mc.freq),
            };
            (pc.channel_id, freq)
        })
        .collect()
}

/// Actions from context menus.
enum ContextAction {
    Remove(ChannelId),
    ChangeColor(ChannelId, egui::Color32),
    SetYAxis(ChannelId, YAxis),
    AddGauge(ChannelId),
    SetDisplayTransform(ChannelId, f64, f64, Option<String>),
    SaveGlobalPreference(ChannelId, ChannelPreference),
    ApplyGlobalPreference(ChannelId),
    ClearGlobalPreference(ChannelId),
}

struct DisplayUnitPreset {
    label: &'static str,
    scale: f64,
    offset: f64,
    unit: &'static str,
}

fn normalized_unit(unit: &str) -> String {
    unit.to_ascii_lowercase().replace(' ', "")
}

fn display_presets_for_unit(unit: &str) -> Vec<DisplayUnitPreset> {
    match normalized_unit(unit).as_str() {
        "km/h" | "kph" => vec![DisplayUnitPreset {
            label: "Show as mph",
            scale: 0.621_371_192,
            offset: 0.0,
            unit: "mph",
        }],
        "mph" => vec![DisplayUnitPreset {
            label: "Show as km/h",
            scale: 1.609_344,
            offset: 0.0,
            unit: "km/h",
        }],
        "kpa" => vec![DisplayUnitPreset {
            label: "Show as psi",
            scale: 0.145_037_738,
            offset: 0.0,
            unit: "psi",
        }],
        "psi" => vec![DisplayUnitPreset {
            label: "Show as kPa",
            scale: 6.894_757_293,
            offset: 0.0,
            unit: "kPa",
        }],
        "°c" | "degc" | "c" => vec![DisplayUnitPreset {
            label: "Show as °F",
            scale: 1.8,
            offset: 32.0,
            unit: "°F",
        }],
        "°f" | "degf" | "f" => vec![DisplayUnitPreset {
            label: "Show as °C",
            scale: 5.0 / 9.0,
            offset: -32.0 * 5.0 / 9.0,
            unit: "°C",
        }],
        _ => Vec::new(),
    }
}

fn transformed_value_for_display(
    channel: &PlottedChannel,
    name: &str,
    enum_labels: &HashMap<i64, String>,
    value: f64,
) -> f64 {
    if uses_discrete_values(name, enum_labels) {
        value
    } else {
        value * channel.display_scale + channel.display_offset
    }
}

#[derive(Clone, Copy)]
struct SampleRange {
    start: usize,
    end: usize,
}

impl SampleRange {
    fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

#[derive(Clone, Copy)]
struct PlotXRange {
    min: f64,
    max: f64,
}

#[derive(Clone, Copy)]
struct PlotTransform {
    origin_axis: f64,
    x_scale: f64,
    x_offset: f64,
    y_scale: f64,
    y_offset: f64,
}

struct LapSeriesRenderParams<'a> {
    data: &'a Arc<[f64]>,
    freq: u16,
    lap: &'a Lap,
    axis: &'a ActiveGraphXAxis,
    transform: PlotTransform,
    target_width: usize,
    preserve_native_samples: bool,
}

fn preserve_native_samples_for_channel(channel_id: ChannelId, shared: &SharedState) -> bool {
    let (name, _, _, _, enum_labels) = resolve_channel_display_meta(channel_id, shared);
    uses_discrete_values(&name, &enum_labels)
}

fn native_plot_points_for_range(
    data: &[f64],
    freq: u16,
    samples: SampleRange,
    axis: &ActiveGraphXAxis,
    x_range: PlotXRange,
    y_scale: f64,
    y_offset: f64,
) -> Vec<egui_plot::PlotPoint> {
    if freq == 0 || samples.is_empty() {
        return Vec::new();
    }

    data[samples.start..samples.end]
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let sample_idx = samples.start + idx;
            let time = sample_idx as f64 / freq as f64;
            egui_plot::PlotPoint::new(axis.axis_value_at_time(time), *value * y_scale + y_offset)
        })
        .filter(|point| point.x >= x_range.min && point.x <= x_range.max)
        .collect()
}

fn native_lap_plot_points_for_range(
    data: &[f64],
    freq: u16,
    samples: SampleRange,
    axis: &ActiveGraphXAxis,
    x_range: PlotXRange,
    transform: PlotTransform,
) -> Vec<egui_plot::PlotPoint> {
    if freq == 0 || samples.is_empty() {
        return Vec::new();
    }

    data[samples.start..samples.end]
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let sample_idx = samples.start + idx;
            let time = sample_idx as f64 / freq as f64;
            let transformed_x = (axis.axis_value_at_time(time) - transform.origin_axis)
                * transform.x_scale
                + transform.x_offset;
            egui_plot::PlotPoint::new(
                transformed_x,
                *value * transform.y_scale + transform.y_offset,
            )
        })
        .filter(|point| point.x >= x_range.min && point.x <= x_range.max)
        .collect()
}

#[derive(Clone, Copy)]
pub enum OverlaySource {
    MainSession,
    External(usize),
}

#[derive(Clone, Copy)]
pub struct LapOverlay {
    pub source: OverlaySource,
    pub lap_idx: usize,
    pub manual_offset: f64,
    pub stretch_to_reference: bool,
}

pub struct OverlayChannelCacheEntry {
    pub data: Arc<[f64]>,
    pub freq: u16,
}

pub struct OverlaySession {
    pub path: PathBuf,
    pub file_name: String,
    pub ld_file: Arc<LdFile>,
    pub laps: Vec<Lap>,
    pub distance_axis_cache: Option<DistanceAxisCache>,
    pub channel_cache: HashMap<String, OverlayChannelCacheEntry>,
}

#[derive(Clone, PartialEq, Eq)]
enum GraphAxisFingerprint {
    Time,
    Distance {
        data_ptr: usize,
        data_len: usize,
        freq: u16,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct GraphRenderFingerprint {
    data_ptr: usize,
    data_len: usize,
    x_bounds: (u64, u64),
    axis: GraphAxisFingerprint,
    transform: DisplayTransformFingerprint,
    target_width: usize,
    preserve_native_samples: bool,
}

struct GraphRenderCacheEntry {
    fingerprint: GraphRenderFingerprint,
    plot_points: Vec<egui_plot::PlotPoint>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum LapRenderCacheKey {
    Reference(ChannelId),
    Overlay {
        channel_id: ChannelId,
        overlay_idx: usize,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct LapRenderFingerprint {
    data_ptr: usize,
    data_len: usize,
    lap_bounds: (u64, u64),
    x_bounds: (u64, u64),
    axis: GraphAxisFingerprint,
    origin_axis_bits: u64,
    x_scale_bits: u64,
    x_offset_bits: u64,
    transform: DisplayTransformFingerprint,
    target_width: usize,
    preserve_native_samples: bool,
}

struct LapRenderCacheEntry {
    fingerprint: LapRenderFingerprint,
    plot_points: Vec<egui_plot::PlotPoint>,
}

struct DisplayStats {
    min: f64,
    max: f64,
    avg: f64,
}

#[derive(Clone, Copy)]
struct ManageChannelDragPayload {
    channel_id: ChannelId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManageChannelsSelection {
    Group(usize),
    Channel(ChannelId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddChannelPickerTarget {
    ExistingGroup(usize),
    NewGraph,
}

enum ManageChannelsAction {
    Context(ContextAction),
    MoveBefore {
        dragged: ChannelId,
        target: ChannelId,
    },
    MoveToGroupEnd {
        dragged: ChannelId,
        group: usize,
    },
    MoveToNewGraph(ChannelId),
    AddToGroup {
        channel_id: ChannelId,
        group: usize,
    },
    AddToNewGraph(ChannelId),
    Remove(ChannelId),
}

/// A single graph panel with its own set of plotted channels.
pub struct GraphPanel {
    pub id: u64,
    pub title: String,
    pub plotted_channels: Vec<PlottedChannel>,
    pub embedded_gauges: Vec<GaugeChannel>,
    pub colors: Vec<egui::Color32>,
    pub graph_mode: GraphMode,
    pub x_axis_mode: GraphXAxis,
    pub reference_lap: Option<usize>,
    pub lap_overlays: Vec<LapOverlay>,
    pub overlay_sessions: Vec<OverlaySession>,
    pub embedded_gauge_height: f32,
    pub tile_heights: Vec<f32>,
    show_manage_channels_window: bool,
    manage_channels_selection: Option<ManageChannelsSelection>,
    show_add_channel_picker: bool,
    add_channel_target: Option<AddChannelPickerTarget>,
    add_channel_filter: String,
    add_channel_selection: Option<ChannelId>,
    /// Set when the first channel is added; consumed on next render to reset zoom.
    needs_zoom_reset: bool,
    render_cache: HashMap<ChannelId, GraphRenderCacheEntry>,
    lap_render_cache: HashMap<LapRenderCacheKey, LapRenderCacheEntry>,
}

impl GraphPanel {
    fn preferred_color_for_channel(
        channel_id: ChannelId,
        shared: &SharedState,
    ) -> Option<egui::Color32> {
        let (name, _, _, _) = resolve_channel_meta(channel_id, shared);
        let pref = shared
            .channel_preferences
            .get(&channel_preference_key(&name))?;
        let [r, g, b] = pref.color?;
        Some(egui::Color32::from_rgb(r, g, b))
    }

    fn next_tile_group(&self) -> usize {
        self.plotted_channels
            .iter()
            .map(|pc| pc.tile_group)
            .max()
            .map_or(0, |group| group + 1)
    }

    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            plotted_channels: Vec::new(),
            embedded_gauges: Vec::new(),
            colors: CHANNEL_COLORS.to_vec(),
            graph_mode: GraphMode::Tiled,
            x_axis_mode: GraphXAxis::Time,
            reference_lap: None,
            lap_overlays: Vec::new(),
            overlay_sessions: Vec::new(),
            embedded_gauge_height: 176.0,
            tile_heights: Vec::new(),
            show_manage_channels_window: false,
            manage_channels_selection: None,
            show_add_channel_picker: false,
            add_channel_target: None,
            add_channel_filter: String::new(),
            add_channel_selection: None,
            needs_zoom_reset: false,
            render_cache: HashMap::new(),
            lap_render_cache: HashMap::new(),
        }
    }

    pub fn reset_for_new_main_session(&mut self) {
        self.plotted_channels.clear();
        self.embedded_gauges.clear();
        self.reference_lap = None;
        self.lap_overlays.clear();
        self.overlay_sessions.clear();
        self.tile_heights.clear();
        self.show_manage_channels_window = false;
        self.manage_channels_selection = None;
        self.show_add_channel_picker = false;
        self.add_channel_target = None;
        self.add_channel_filter.clear();
        self.add_channel_selection = None;
        self.needs_zoom_reset = false;
        self.render_cache.clear();
        self.lap_render_cache.clear();
    }

    pub fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        self.add_channel_to_group(channel_id, self.next_tile_group(), shared);
    }

    pub fn add_channel_to_group(
        &mut self,
        channel_id: ChannelId,
        tile_group: usize,
        shared: &SharedState,
    ) {
        if self.is_channel_plotted(channel_id) {
            return;
        }
        let color_idx = self.plotted_channels.len() % self.colors.len();
        if let Some(mut pc) = create_plotted_channel(channel_id, shared, color_idx) {
            if Self::preferred_color_for_channel(channel_id, shared).is_none() {
                pc.color = self.colors[color_idx];
            }
            pc.tile_group = tile_group;
            self.plotted_channels.push(pc);
        }
    }

    pub fn remove_channel(&mut self, channel_id: ChannelId) {
        self.plotted_channels
            .retain(|pc| pc.channel_id != channel_id);
        self.render_cache.remove(&channel_id);
        self.lap_render_cache.retain(|key, _| {
            !matches!(
                key,
                LapRenderCacheKey::Reference(id)
                    | LapRenderCacheKey::Overlay { channel_id: id, .. }
                    if *id == channel_id
            )
        });
    }

    pub fn add_embedded_gauge(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self
            .embedded_gauges
            .iter()
            .any(|g| g.channel.channel_id == channel_id)
        {
            return;
        }
        let color_idx = self.embedded_gauges.len() % self.colors.len();
        if let Some(pc) = create_plotted_channel(channel_id, shared, color_idx) {
            let (name, _, _, _) = resolve_channel_meta(channel_id, shared);
            self.embedded_gauges.push(GaugeChannel {
                channel: pc,
                style: default_style_for_name(&name),
            });
        }
    }

    pub fn add_embedded_gauge_with_style(
        &mut self,
        channel_id: ChannelId,
        shared: &SharedState,
        style: GaugeStyle,
    ) {
        self.add_embedded_gauge(channel_id, shared);
        if let Some(gauge) = self
            .embedded_gauges
            .iter_mut()
            .find(|g| g.channel.channel_id == channel_id)
        {
            gauge.style = style;
        }
    }

    pub fn load_external_overlay_path(&mut self, path: PathBuf) -> Option<usize> {
        if let Some((idx, _)) = self
            .overlay_sessions
            .iter()
            .enumerate()
            .find(|(_, session)| session.path == path)
        {
            return Some(idx);
        }

        let ld_file = Arc::new(LdFile::open(&path).ok()?);
        let laps = detect_laps(&ld_file);
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let session_idx = self.overlay_sessions.len();
        self.overlay_sessions.push(OverlaySession {
            path,
            file_name,
            ld_file: ld_file.clone(),
            laps,
            distance_axis_cache: derive_distance_axis_cache_for_ld(&ld_file).map(|cache| {
                DistanceAxisCache {
                    data: Arc::from(cache.data),
                    freq: cache.freq,
                }
            }),
            channel_cache: HashMap::new(),
        });
        Some(session_idx)
    }

    pub fn toggle_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self.is_channel_plotted(channel_id) {
            self.remove_channel(channel_id);
        } else {
            self.add_channel(channel_id, shared);
        }
    }

    pub fn is_channel_plotted(&self, channel_id: ChannelId) -> bool {
        self.plotted_channels
            .iter()
            .any(|pc| pc.channel_id == channel_id)
    }

    /// Render the graph panel UI.
    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        let _perf = crate::perf_metrics::scope("graph draw");

        // Handle pending channel toggle from browser
        if let Some(ch_id) = shared.pending_toggle_channel.take() {
            let was_empty = self.plotted_channels.is_empty();
            self.toggle_channel(ch_id, shared);
            if was_empty && !self.plotted_channels.is_empty() {
                self.needs_zoom_reset = true;
            }
        }

        for channel in &mut self.plotted_channels {
            refresh_plotted_channel(channel, shared);
        }
        for gauge in &mut self.embedded_gauges {
            refresh_plotted_channel(&mut gauge.channel, shared);
        }

        if self.plotted_channels.is_empty() {
            if shared.dragging_channel.is_some()
                && ui.input(|i| i.pointer.any_released())
                && ui.ui_contains_pointer()
                && let Some(ch_id) = shared.dragging_channel.take()
            {
                self.add_channel(ch_id, shared);
                self.needs_zoom_reset = true;
            }
            ui.centered_and_justified(|ui| {
                ui.label("Click channels in the browser to plot them, or drag and drop");
            });
            self.show_manage_channels_window(ui.ctx(), shared);
            return;
        }

        self.retain_valid_overlay_state(shared);
        self.show_toolbar(ui, shared);
        if !self.embedded_gauges.is_empty() {
            ui.add_space(4.0);
            let available_height = ui.available_height();
            let max_gauge_height = (available_height - 96.0).max(120.0);
            self.embedded_gauge_height = self.embedded_gauge_height.clamp(120.0, max_gauge_height);
            let gauge_height = self.embedded_gauge_height;
            let (gauge_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), gauge_height),
                egui::Sense::hover(),
            );
            let mut gauge_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(gauge_rect)
                    .layout(*ui.layout()),
            );
            gauge_ui.set_clip_rect(gauge_rect);
            self.show_embedded_gauges(&mut gauge_ui, shared);
            let gauge_resize_delta = self.show_resize_handle(ui).y;
            if gauge_resize_delta.abs() > 0.0 {
                self.embedded_gauge_height = (self.embedded_gauge_height + gauge_resize_delta)
                    .clamp(120.0, max_gauge_height);
            }
            ui.separator();
        }

        for pc in &self.plotted_channels {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(pc, shared));
        }
        for gc in &self.embedded_gauges {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(&gc.channel, shared));
        }

        let needs_zoom_reset = self.needs_zoom_reset;
        self.needs_zoom_reset = false;
        let (x_axis, x_axis_warning) = Self::resolve_x_axis(shared, self.x_axis_mode);

        if let Some(warning) = x_axis_warning {
            ui.label(egui::RichText::new(warning).small().weak());
            ui.add_space(4.0);
        }

        if self.reference_lap.is_some() {
            match self.graph_mode {
                GraphMode::Overlay => {
                    self.show_lap_overlay_graph(ui, shared, needs_zoom_reset, &x_axis)
                }
                GraphMode::Tiled => {
                    self.show_lap_overlay_tiled_graphs(ui, shared, needs_zoom_reset, &x_axis)
                }
            }
        } else {
            match self.graph_mode {
                GraphMode::Overlay => {
                    self.show_overlay_graph(ui, shared, needs_zoom_reset, &x_axis)
                }
                GraphMode::Tiled => self.show_tiled_graphs(ui, shared, needs_zoom_reset, &x_axis),
            }
        }

        self.show_manage_channels_window(ui.ctx(), shared);
    }

    fn retain_valid_overlay_state(&mut self, shared: &SharedState) {
        if self
            .reference_lap
            .is_some_and(|idx| idx >= shared.laps.len())
        {
            self.reference_lap = None;
        }

        if self.reference_lap.is_none() {
            self.lap_overlays.clear();
            return;
        }

        self.lap_overlays.retain(|overlay| match overlay.source {
            OverlaySource::MainSession => overlay.lap_idx < shared.laps.len(),
            OverlaySource::External(session_idx) => self
                .overlay_sessions
                .get(session_idx)
                .is_some_and(|session| overlay.lap_idx < session.laps.len()),
        });
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        let mut new_reference = self.reference_lap;
        ui.horizontal_wrapped(|ui| {
            ui.label("Reference lap:");
            egui::ComboBox::from_id_salt(format!("graph_ref_lap_{}", self.id))
                .selected_text(match new_reference {
                    Some(idx) if idx < shared.laps.len() => shared.laps[idx].name.clone(),
                    _ => "Full session".into(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_reference, None, "Full session");
                    for (idx, lap) in shared.laps.iter().enumerate() {
                        let label = format!(
                            "{} ({})",
                            lap.name,
                            i3rs_core::format_duration(lap.duration())
                        );
                        ui.selectable_value(&mut new_reference, Some(idx), label);
                    }
                });

            if ui
                .add_enabled(
                    self.reference_lap.is_some() && shared.selected_lap.is_some(),
                    egui::Button::new("Add Selected Lap"),
                )
                .clicked()
                && let Some(selected_lap) = shared.selected_lap
                && self.reference_lap != Some(selected_lap)
                && !self.lap_overlays.iter().any(|overlay| {
                    matches!(overlay.source, OverlaySource::MainSession)
                        && overlay.lap_idx == selected_lap
                })
            {
                self.lap_overlays.push(LapOverlay {
                    source: OverlaySource::MainSession,
                    lap_idx: selected_lap,
                    manual_offset: 0.0,
                    stretch_to_reference: false,
                });
                self.lap_render_cache.clear();
                self.needs_zoom_reset = true;
            }

            if ui
                .add_enabled(
                    self.reference_lap.is_some(),
                    egui::Button::new("Add Overlay File..."),
                )
                .clicked()
            {
                self.load_external_overlay();
                self.lap_render_cache.clear();
                self.needs_zoom_reset = true;
            }

            if ui
                .add_enabled(
                    !self.lap_overlays.is_empty() || !self.overlay_sessions.is_empty(),
                    egui::Button::new("Clear Overlays"),
                )
                .clicked()
            {
                self.lap_overlays.clear();
                self.overlay_sessions.clear();
                self.lap_render_cache.clear();
                self.needs_zoom_reset = true;
            }

            ui.separator();

            ui.menu_button("Add Gauge", |ui| {
                let mut to_add = None;
                for pc in &self.plotted_channels {
                    if self
                        .embedded_gauges
                        .iter()
                        .any(|g| g.channel.channel_id == pc.channel_id)
                    {
                        continue;
                    }
                    let (name, _, _, _) = resolve_channel_meta(pc.channel_id, shared);
                    if ui.button(name).clicked() {
                        to_add = Some(pc.channel_id);
                        ui.close();
                    }
                }
                if let Some(channel_id) = to_add {
                    self.add_embedded_gauge(channel_id, shared);
                }
            });

            if ui
                .add_enabled(
                    !self.embedded_gauges.is_empty(),
                    egui::Button::new("Clear Gauges"),
                )
                .clicked()
            {
                self.embedded_gauges.clear();
            }
        });

        if new_reference != self.reference_lap {
            self.reference_lap = new_reference;
            if let Some(reference_lap) = self.reference_lap {
                self.lap_overlays.retain(|overlay| {
                    !matches!(overlay.source, OverlaySource::MainSession)
                        || overlay.lap_idx != reference_lap
                });
            } else {
                self.lap_overlays.clear();
            }
            self.lap_render_cache.clear();
            self.needs_zoom_reset = true;
        }

        if !self.lap_overlays.is_empty() {
            ui.add_space(4.0);
            egui::Grid::new(format!("graph_overlay_rows_{}", self.id))
                .num_columns(5)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Overlay");
                    ui.strong("Lap");
                    ui.strong("Offset");
                    ui.strong("Stretch");
                    ui.strong("");
                    ui.end_row();

                    let mut remove_idx = None;
                    for idx in 0..self.lap_overlays.len() {
                        let source_label = {
                            let overlay = &self.lap_overlays[idx];
                            self.overlay_source_label(overlay)
                        };
                        let lap_options: Vec<String> = {
                            let overlay = &self.lap_overlays[idx];
                            self.overlay_laps_for_source(shared, overlay)
                                .iter()
                                .map(|lap| {
                                    format!(
                                        "{} ({})",
                                        lap.name,
                                        i3rs_core::format_duration(lap.duration())
                                    )
                                })
                                .collect()
                        };
                        let overlay = &mut self.lap_overlays[idx];

                        ui.label(source_label);

                        let lap_label = lap_options
                            .get(overlay.lap_idx)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".into());
                        egui::ComboBox::from_id_salt(format!(
                            "graph_overlay_lap_{}_{}",
                            self.id, idx
                        ))
                        .selected_text(lap_label)
                        .show_ui(ui, |ui| {
                            for (lap_idx, label) in lap_options.iter().enumerate() {
                                ui.selectable_value(&mut overlay.lap_idx, lap_idx, label);
                            }
                        });

                        ui.add(
                            egui::DragValue::new(&mut overlay.manual_offset)
                                .speed(0.01)
                                .range(-30.0..=30.0)
                                .suffix(" s"),
                        );
                        ui.checkbox(&mut overlay.stretch_to_reference, "");
                        if ui.small_button("x").clicked() {
                            remove_idx = Some(idx);
                        }
                        ui.end_row();
                    }

                    if let Some(idx) = remove_idx {
                        self.lap_overlays.remove(idx);
                        self.lap_render_cache.clear();
                    }
                });
        }

        if !self.embedded_gauges.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.weak("Panel gauges:");
                let mut remove_idx = None;
                for (idx, gc) in self.embedded_gauges.iter_mut().enumerate() {
                    let (name, _, _, _) = resolve_channel_meta(gc.channel.channel_id, shared);
                    ui.menu_button(name, |ui| {
                        ui.selectable_value(&mut gc.style, GaugeStyle::Analog, "Analog");
                        ui.selectable_value(&mut gc.style, GaugeStyle::Bar, "Bar");
                        ui.selectable_value(&mut gc.style, GaugeStyle::Digital, "Digital");
                        ui.selectable_value(
                            &mut gc.style,
                            GaugeStyle::SteeringWheel,
                            "Steering Wheel",
                        );
                        ui.separator();
                        if ui.button("Remove").clicked() {
                            remove_idx = Some(idx);
                            ui.close();
                        }
                    });
                }
                if let Some(idx) = remove_idx {
                    self.embedded_gauges.remove(idx);
                }
            });
        }

        ui.separator();
    }

    fn show_embedded_gauges(&mut self, ui: &mut egui::Ui, shared: &SharedState) {
        let spacing = 8.0_f32;
        let clip_rect = ui.clip_rect();
        let available_width = ui.available_width().min(clip_rect.width()).max(1.0);
        let available_height = (clip_rect.bottom() - ui.cursor().top()).max(1.0);
        let (cols, gauge_size) = best_gauge_grid(
            self.embedded_gauges.len(),
            available_width,
            available_height,
            spacing,
        );

        egui::Grid::new(format!("graph_embedded_gauges_{}", self.id))
            .num_columns(cols)
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                for (idx, gauge) in self.embedded_gauges.iter_mut().enumerate() {
                    let (name, unit, freq, dec_places, enum_labels) =
                        resolve_plotted_channel_display_meta(&gauge.channel, shared);
                    let raw_value = shared
                        .cursor_time
                        .and_then(|t| interp_at_time(&gauge.channel.data, freq, t));
                    let discrete = !enum_labels.is_empty();
                    let value = raw_value.map(|v| {
                        if discrete {
                            v
                        } else {
                            v * gauge.channel.display_scale + gauge.channel.display_offset
                        }
                    });
                    let mut min = gauge.channel.cached_min * gauge.channel.display_scale
                        + gauge.channel.display_offset;
                    let mut max = gauge.channel.cached_max * gauge.channel.display_scale
                        + gauge.channel.display_offset;
                    if gauge.channel.display_scale < 0.0 {
                        std::mem::swap(&mut min, &mut max);
                    }

                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(gauge_size, gauge_size),
                        egui::Sense::click(),
                    );
                    response.context_menu(|ui| {
                        ui.selectable_value(&mut gauge.style, GaugeStyle::Analog, "Analog");
                        ui.selectable_value(&mut gauge.style, GaugeStyle::Bar, "Bar");
                        ui.selectable_value(&mut gauge.style, GaugeStyle::Digital, "Digital");
                        ui.selectable_value(
                            &mut gauge.style,
                            GaugeStyle::SteeringWheel,
                            "Steering Wheel",
                        );
                    });

                    let painter = ui.painter_at(rect);
                    let ctx = GaugeDrawContext {
                        name: &name,
                        unit: &unit,
                        value,
                        min,
                        max,
                        dec_places,
                        color: gauge.channel.color,
                    };
                    draw_gauge(&painter, rect, gauge.style, &ctx);

                    if (idx + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
    }

    fn load_external_overlay(&mut self) {
        if let Some(path) = crate::platform::pick_overlay_file()
            && let Some(session_idx) = self.load_external_overlay_path(path)
        {
            let default_lap_idx = self
                .overlay_sessions
                .get(session_idx)
                .and_then(|session| default_overlay_lap_index(&session.laps))
                .unwrap_or(0);
            self.lap_overlays.push(LapOverlay {
                source: OverlaySource::External(session_idx),
                lap_idx: default_lap_idx,
                manual_offset: 0.0,
                stretch_to_reference: false,
            });
        }
    }

    fn overlay_laps_for_source<'a>(
        &'a self,
        shared: &'a SharedState,
        overlay: &LapOverlay,
    ) -> &'a [Lap] {
        match overlay.source {
            OverlaySource::MainSession => &shared.laps,
            OverlaySource::External(session_idx) => self
                .overlay_sessions
                .get(session_idx)
                .map(|session| session.laps.as_slice())
                .unwrap_or(&[]),
        }
    }

    fn overlay_source_label(&self, overlay: &LapOverlay) -> String {
        match overlay.source {
            OverlaySource::MainSession => "Current Session".into(),
            OverlaySource::External(session_idx) => self
                .overlay_sessions
                .get(session_idx)
                .map(|session| session.file_name.clone())
                .unwrap_or_else(|| "External".into()),
        }
    }

    fn show_overlay_graph(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
        x_axis: &ActiveGraphXAxis,
    ) {
        let cursor_group = egui::Id::new("global_cursor_link");
        let data_duration = shared.data_duration.unwrap_or(f64::MAX);
        let full_x_range = x_axis.full_range(data_duration);

        let mut plot = Plot::new(format!("overlay_{}", self.id))
            .x_axis_label(x_axis.label())
            .allow_drag(egui::Vec2b::new(true, false))
            .allow_zoom(egui::Vec2b::new(true, false))
            .allow_scroll(false)
            .show_axes(true)
            .show_grid(true)
            .y_axis_min_width(36.0)
            .link_cursor(cursor_group, egui::Vec2b::new(true, false));

        if needs_zoom_reset {
            plot = plot
                .include_x(full_x_range.0)
                .include_x(full_x_range.1)
                .auto_bounds(egui::Vec2b::new(true, true));
        }

        let laps = shared.laps.as_slice();
        let show_markers = shared.show_lap_markers;
        let cursor_time = shared.cursor_time;
        let plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();
        let channel_meta: Vec<ChannelDisplayMeta> = self
            .plotted_channels
            .iter()
            .map(|pc| resolve_plotted_channel_display_meta(pc, shared))
            .collect();

        let mut new_cursor_time = None;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;

        let y_range = Self::compute_y_range(&plotted);
        let freq_map = build_freq_map(&plotted, shared);
        let render_cache = &mut self.render_cache;

        let response = plot.show(ui, |plot_ui| {
            if needs_zoom_reset {
                plot_ui.set_plot_bounds_x(full_x_range.0..=full_x_range.1);
            } else if let Some((x_min, x_max)) = zoom_range {
                let (axis_min, axis_max) = x_axis.axis_range_for_time_range(x_min, x_max);
                plot_ui.set_plot_bounds_x(axis_min..=axis_max);
            }

            if let Some((y_min, y_max)) = y_range {
                let padding = if (y_max - y_min).abs() < 1e-10 {
                    1.0
                } else {
                    (y_max - y_min) * 0.05
                };
                plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
            }

            Self::draw_channels(plot_ui, &plotted, &freq_map, x_axis, shared, render_cache);

            if show_markers {
                Self::draw_lap_markers(plot_ui, laps, x_axis);
            }

            if let Some(t) = cursor_time {
                Self::draw_cursor_line(plot_ui, x_axis.axis_value_at_time(t));
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                new_cursor_time = x_axis.time_from_axis_value(coord.x, data_duration);
            }
        });

        if response.response.hovered()
            && let Some(t) = new_cursor_time
        {
            shared.cursor_time = Some(t);
        }

        let bounds = response.transform.bounds();
        let visible_time_range = Self::time_range_from_plot_bounds(
            x_axis,
            bounds.min()[0],
            bounds.max()[0],
            data_duration,
        );

        Self::draw_legend(
            ui,
            response.response.rect,
            &plotted,
            &channel_meta,
            shared.cursor_time,
            Some(visible_time_range),
        );

        if needs_zoom_reset {
            shared.zoom_range = Some((0.0, data_duration));
        } else if !zoom_from_timeline {
            shared.zoom_range = Some(Self::time_range_from_plot_bounds(
                x_axis,
                bounds.min()[0],
                bounds.max()[0],
                data_duration,
            ));
        }

        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && response.response.contains_pointer()
            && let Some(ch_id) = shared.dragging_channel.take()
        {
            self.add_channel_to_group(ch_id, 0, shared);
            self.needs_zoom_reset = true;
        }

        self.handle_context_menu(&response.response, shared);
    }

    fn show_tiled_graphs(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
        x_axis: &ActiveGraphXAxis,
    ) {
        let cursor_group = egui::Id::new("global_cursor_link");
        let laps = shared.laps.as_slice();
        let show_markers = shared.show_lap_markers;
        let cursor_time = shared.cursor_time;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;
        let data_duration = shared.data_duration.unwrap_or(f64::MAX);
        let full_x_range = x_axis.full_range(data_duration);
        let tile_groups = self.tiled_channel_groups();
        let n = tile_groups.len().max(1);

        // Pre-compute metadata for each channel to avoid borrowing shared inside closures
        let channel_meta: Vec<ChannelDisplayMeta> = self
            .plotted_channels
            .iter()
            .map(|pc| resolve_plotted_channel_display_meta(pc, shared))
            .collect();

        let all_plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();
        let freq_map = build_freq_map(&all_plotted, shared);

        let available_height = ui.available_height();
        self.ensure_tile_heights(n, available_height);

        let mut any_hovered_cursor: Option<f64> = None;
        let mut hovered_x_bounds: Option<(f64, f64)> = None;
        let mut first_x_bounds: Option<(f64, f64)> = None;
        let mut responses = Vec::new();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
            .show(ui, |ui| {
                for (tile_idx, group) in tile_groups.iter().enumerate() {
                    let plot_id = format!("tile_{}_{}", self.id, tile_idx);
                    let tile_height = self.tile_heights[tile_idx].max(80.0);

                    let mut plot = Plot::new(plot_id)
                        .height(tile_height)
                        .allow_drag(egui::Vec2b::new(true, false))
                        .allow_zoom(egui::Vec2b::new(true, false))
                        .allow_scroll(false)
                        .show_axes(true)
                        .show_grid(true)
                        .y_axis_min_width(36.0)
                        .link_cursor(cursor_group, egui::Vec2b::new(true, false));

                    if tile_idx == n - 1 {
                        plot = plot.x_axis_label(x_axis.label());
                    }

                    if needs_zoom_reset {
                        plot = plot
                            .include_x(full_x_range.0)
                            .include_x(full_x_range.1)
                            .auto_bounds(egui::Vec2b::new(true, true));
                    }

                    let grouped: Vec<&PlottedChannel> = group
                        .iter()
                        .map(|&channel_idx| &self.plotted_channels[channel_idx])
                        .collect();
                    let y_range = Self::compute_y_range(&grouped);
                    let mut tile_cursor = None;

                    let resp = plot.show(ui, |plot_ui| {
                        if needs_zoom_reset {
                            plot_ui.set_plot_bounds_x(full_x_range.0..=full_x_range.1);
                        } else if let Some((x_min, x_max)) = zoom_range {
                            let (axis_min, axis_max) =
                                x_axis.axis_range_for_time_range(x_min, x_max);
                            plot_ui.set_plot_bounds_x(axis_min..=axis_max);
                        }

                        if let Some((y_min, y_max)) = y_range {
                            let padding = if (y_max - y_min).abs() < 1e-10 {
                                1.0
                            } else {
                                (y_max - y_min) * 0.05
                            };
                            plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
                        }

                        Self::draw_channels(
                            plot_ui,
                            &grouped,
                            &freq_map,
                            x_axis,
                            shared,
                            &mut self.render_cache,
                        );

                        if show_markers {
                            Self::draw_lap_markers(plot_ui, laps, x_axis);
                        }

                        if let Some(t) = cursor_time {
                            Self::draw_cursor_line(plot_ui, x_axis.axis_value_at_time(t));
                        }

                        if let Some(coord) = plot_ui.pointer_coordinate() {
                            tile_cursor = x_axis.time_from_axis_value(coord.x, data_duration);
                        }
                    });

                    let bounds = resp.transform.bounds();
                    let x_pair = (bounds.min()[0], bounds.max()[0]);
                    if first_x_bounds.is_none() {
                        first_x_bounds = Some(x_pair);
                    }

                    if resp.response.hovered() {
                        hovered_x_bounds = Some(x_pair);
                        if let Some(t) = tile_cursor {
                            any_hovered_cursor = Some(t);
                        }
                    }

                    // Draw legend with pre-computed metadata
                    Self::draw_group_legend(
                        ui,
                        resp.response.rect,
                        &grouped,
                        &channel_meta,
                        group,
                        cursor_time,
                        Some(Self::time_range_from_plot_bounds(
                            x_axis,
                            x_pair.0,
                            x_pair.1,
                            data_duration,
                        )),
                    );

                    responses.push((group.clone(), resp.response));

                    if tile_idx + 1 < n {
                        let delta = self.show_resize_handle(ui).y;
                        if delta.abs() > 0.0 {
                            Self::apply_adjacent_tile_resize(
                                &mut self.tile_heights,
                                tile_idx,
                                delta,
                            );
                        }
                    }
                }
            });

        for (group, resp) in &responses {
            self.handle_tile_group_context_menu(resp, group, &channel_meta, shared);
        }

        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
        {
            let target_group = responses
                .iter()
                .find(|(_, resp)| resp.contains_pointer())
                .and_then(|(group, _)| group.first())
                .and_then(|&idx| self.plotted_channels.get(idx))
                .map(|channel| channel.tile_group);
            if let Some(ch_id) = shared.dragging_channel.take() {
                self.add_channel_to_group(
                    ch_id,
                    target_group.unwrap_or_else(|| self.next_tile_group()),
                    shared,
                );
                if target_group.is_none() {
                    self.tile_heights.clear();
                }
                self.needs_zoom_reset = true;
            }
        }

        if let Some(t) = any_hovered_cursor {
            shared.cursor_time = Some(t);
        }

        if needs_zoom_reset {
            shared.zoom_range = Some((0.0, data_duration));
        } else if !zoom_from_timeline
            && let Some((x_min, x_max)) = hovered_x_bounds.or(first_x_bounds)
        {
            shared.zoom_range = Some(Self::time_range_from_plot_bounds(
                x_axis,
                x_min,
                x_max,
                data_duration,
            ));
        }
    }

    fn show_lap_overlay_graph(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
        x_axis: &ActiveGraphXAxis,
    ) {
        let Some(reference_lap_idx) = self.reference_lap else {
            return;
        };
        let Some(reference_lap) = shared.laps.get(reference_lap_idx).cloned() else {
            return;
        };
        let Some(viewport) = OverlayViewport::new(
            reference_lap,
            x_axis.clone(),
            shared.data_duration.unwrap_or_default(),
        ) else {
            return;
        };

        let cursor_group = egui::Id::new(format!("lap_overlay_cursor_{}", self.id));
        let mut plot = Plot::new(format!("lap_overlay_{}", self.id))
            .x_axis_label(viewport.axis_label())
            .allow_drag(egui::Vec2b::new(true, false))
            .allow_zoom(egui::Vec2b::new(true, false))
            .allow_scroll(false)
            .show_axes(true)
            .show_grid(true)
            .y_axis_min_width(36.0)
            .link_cursor(cursor_group, egui::Vec2b::new(true, false));

        if needs_zoom_reset {
            let (x_min, x_max) = viewport.full_range();
            plot = plot
                .include_x(x_min)
                .include_x(x_max)
                .auto_bounds(egui::Vec2b::new(true, true));
        }

        let prepared_overlays = self.prepare_overlay_series(
            shared,
            x_axis,
            &viewport,
            shared.data_duration.unwrap_or_default(),
        );
        let plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();
        let channel_meta: Vec<ChannelDisplayMeta> = self
            .plotted_channels
            .iter()
            .map(|pc| resolve_plotted_channel_display_meta(pc, shared))
            .collect();
        let freq_map = build_freq_map(&plotted, shared);
        let y_range = Self::compute_y_range(&plotted);
        let cursor_time = shared.cursor_time;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;

        let mut new_cursor_time = None;
        let response = plot.show(ui, |plot_ui| {
            let mut draw_specs = Vec::new();
            let (x_min, x_max) = if needs_zoom_reset {
                viewport.full_range()
            } else if let Some((z_min, z_max)) = zoom_range {
                viewport.axis_range_for_time_range(z_min, z_max)
            } else {
                viewport.full_range()
            };
            plot_ui.set_plot_bounds_x(x_min..=x_max);

            if let Some((y_min, y_max)) = y_range {
                let padding = if (y_max - y_min).abs() < 1e-10 {
                    1.0
                } else {
                    (y_max - y_min) * 0.05
                };
                plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
            }

            let target_width = plot_ui.response().rect.width().max(100.0) as usize;
            let lap_render_cache = &mut self.lap_render_cache;
            for pc in &plotted {
                let freq = freq_map.get(&pc.channel_id).copied().unwrap_or(0);
                let preserve_native_samples =
                    preserve_native_samples_for_channel(pc.channel_id, shared);
                let reference_key = LapRenderCacheKey::Reference(pc.channel_id);
                ensure_lap_series_cache(
                    plot_ui,
                    shared,
                    lap_render_cache,
                    reference_key,
                    LapSeriesRenderParams {
                        data: &pc.data,
                        freq,
                        lap: &viewport.reference_lap,
                        axis: &viewport.reference_axis,
                        transform: PlotTransform {
                            origin_axis: viewport.reference_origin_axis,
                            x_scale: 1.0,
                            x_offset: 0.0,
                            y_scale: pc.display_scale,
                            y_offset: pc.display_offset,
                        },
                        target_width,
                        preserve_native_samples,
                    },
                );
                draw_specs.push((reference_key, pc.color, 2.0));

                for prepared in prepared_overlays.get(&pc.channel_id).into_iter().flatten() {
                    let overlay_key = LapRenderCacheKey::Overlay {
                        channel_id: pc.channel_id,
                        overlay_idx: prepared.overlay_idx,
                    };
                    ensure_lap_series_cache(
                        plot_ui,
                        shared,
                        lap_render_cache,
                        overlay_key,
                        LapSeriesRenderParams {
                            data: &prepared.data,
                            freq: prepared.freq,
                            lap: &prepared.lap,
                            axis: &prepared.axis,
                            transform: PlotTransform {
                                origin_axis: prepared.origin_axis,
                                x_scale: prepared.scale,
                                x_offset: prepared.offset,
                                y_scale: pc.display_scale,
                                y_offset: pc.display_offset,
                            },
                            target_width,
                            preserve_native_samples,
                        },
                    );
                    draw_specs.push((overlay_key, prepared.color, prepared.width));
                }
            }

            let lap_render_cache = &self.lap_render_cache;
            for (cache_key, color, width) in draw_specs {
                draw_cached_lap_series(plot_ui, lap_render_cache, &cache_key, color, width);
            }

            if let Some(cursor_time) = cursor_time {
                Self::draw_cursor_line(plot_ui, viewport.axis_value_for_time(cursor_time));
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                new_cursor_time = viewport.time_from_axis_value(coord.x);
            }
        });

        if response.response.hovered()
            && let Some(cursor_time) = new_cursor_time
        {
            shared.cursor_time = Some(cursor_time);
        }

        let bounds = response.transform.bounds();
        let visible_time_range =
            viewport.time_range_from_plot_bounds(bounds.min()[0], bounds.max()[0]);

        Self::draw_overlay_summary(
            ui,
            response.response.rect,
            &viewport.reference_lap,
            &self.lap_overlays,
            &self.overlay_sessions,
        );
        Self::draw_legend(
            ui,
            response.response.rect,
            &plotted,
            &channel_meta,
            shared.cursor_time,
            Some(visible_time_range),
        );

        if needs_zoom_reset {
            shared.zoom_range = Some((
                viewport.reference_lap.start_time,
                viewport.reference_lap.end_time,
            ));
        } else if !zoom_from_timeline {
            let bounds = response.transform.bounds();
            shared.zoom_range =
                Some(viewport.time_range_from_plot_bounds(bounds.min()[0], bounds.max()[0]));
        }

        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && response.response.contains_pointer()
            && let Some(ch_id) = shared.dragging_channel.take()
        {
            self.add_channel_to_group(ch_id, 0, shared);
            self.needs_zoom_reset = true;
        }

        self.handle_context_menu(&response.response, shared);
    }

    fn show_lap_overlay_tiled_graphs(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
        x_axis: &ActiveGraphXAxis,
    ) {
        let Some(reference_lap_idx) = self.reference_lap else {
            return;
        };
        let Some(reference_lap) = shared.laps.get(reference_lap_idx).cloned() else {
            return;
        };
        let Some(viewport) = OverlayViewport::new(
            reference_lap,
            x_axis.clone(),
            shared.data_duration.unwrap_or_default(),
        ) else {
            return;
        };

        let cursor_group = egui::Id::new(format!("lap_overlay_tiled_cursor_{}", self.id));
        let channel_meta: Vec<ChannelDisplayMeta> = self
            .plotted_channels
            .iter()
            .map(|pc| resolve_plotted_channel_display_meta(pc, shared))
            .collect();
        let tile_groups = self.tiled_channel_groups();
        let all_plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();
        let freq_map = build_freq_map(&all_plotted, shared);
        let available_height = ui.available_height();
        self.ensure_tile_heights(tile_groups.len().max(1), available_height);
        let cursor_time = shared.cursor_time;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;
        let mut hovered_cursor = None;
        let mut hovered_bounds = None;
        let mut first_bounds = None;
        let prepared_overlays = self.prepare_overlay_series(
            shared,
            x_axis,
            &viewport,
            shared.data_duration.unwrap_or_default(),
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut responses = Vec::new();
                for (tile_idx, group) in tile_groups.iter().enumerate() {
                    let tile_height = self.tile_heights[tile_idx].max(80.0);
                    let grouped: Vec<&PlottedChannel> = group
                        .iter()
                        .map(|&channel_idx| &self.plotted_channels[channel_idx])
                        .collect();

                    let mut plot = Plot::new(format!("lap_overlay_tile_{}_{}", self.id, tile_idx))
                        .height(tile_height)
                        .allow_drag(egui::Vec2b::new(true, false))
                        .allow_zoom(egui::Vec2b::new(true, false))
                        .allow_scroll(false)
                        .show_axes(true)
                        .show_grid(true)
                        .y_axis_min_width(36.0)
                        .link_cursor(cursor_group, egui::Vec2b::new(true, false));
                    if tile_idx == tile_groups.len() - 1 {
                        plot = plot.x_axis_label(viewport.axis_label());
                    }

                    let y_range = Self::compute_y_range(&grouped);
                    let mut tile_cursor = None;
                    let resp = plot.show(ui, |plot_ui| {
                        let mut draw_specs = Vec::new();
                        let (x_min, x_max) = if needs_zoom_reset {
                            viewport.full_range()
                        } else if let Some((z_min, z_max)) = zoom_range {
                            viewport.axis_range_for_time_range(z_min, z_max)
                        } else {
                            viewport.full_range()
                        };
                        plot_ui.set_plot_bounds_x(x_min..=x_max);

                        if let Some((y_min, y_max)) = y_range {
                            let padding = if (y_max - y_min).abs() < 1e-10 {
                                1.0
                            } else {
                                (y_max - y_min) * 0.05
                            };
                            plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
                        }

                        let target_width = plot_ui.response().rect.width().max(100.0) as usize;
                        let lap_render_cache = &mut self.lap_render_cache;
                        for pc in &grouped {
                            let freq = freq_map.get(&pc.channel_id).copied().unwrap_or(0);
                            let preserve_native_samples =
                                preserve_native_samples_for_channel(pc.channel_id, shared);
                            let reference_key = LapRenderCacheKey::Reference(pc.channel_id);
                            ensure_lap_series_cache(
                                plot_ui,
                                shared,
                                lap_render_cache,
                                reference_key,
                                LapSeriesRenderParams {
                                    data: &pc.data,
                                    freq,
                                    lap: &viewport.reference_lap,
                                    axis: &viewport.reference_axis,
                                    transform: PlotTransform {
                                        origin_axis: viewport.reference_origin_axis,
                                        x_scale: 1.0,
                                        x_offset: 0.0,
                                        y_scale: pc.display_scale,
                                        y_offset: pc.display_offset,
                                    },
                                    target_width,
                                    preserve_native_samples,
                                },
                            );
                            draw_specs.push((reference_key, pc.color, 2.0));

                            for prepared in
                                prepared_overlays.get(&pc.channel_id).into_iter().flatten()
                            {
                                let overlay_key = LapRenderCacheKey::Overlay {
                                    channel_id: pc.channel_id,
                                    overlay_idx: prepared.overlay_idx,
                                };
                                ensure_lap_series_cache(
                                    plot_ui,
                                    shared,
                                    lap_render_cache,
                                    overlay_key,
                                    LapSeriesRenderParams {
                                        data: &prepared.data,
                                        freq: prepared.freq,
                                        lap: &prepared.lap,
                                        axis: &prepared.axis,
                                        transform: PlotTransform {
                                            origin_axis: prepared.origin_axis,
                                            x_scale: prepared.scale,
                                            x_offset: prepared.offset,
                                            y_scale: pc.display_scale,
                                            y_offset: pc.display_offset,
                                        },
                                        target_width,
                                        preserve_native_samples,
                                    },
                                );
                                draw_specs.push((overlay_key, prepared.color, prepared.width));
                            }
                        }

                        let lap_render_cache = &self.lap_render_cache;
                        for (cache_key, color, width) in draw_specs {
                            draw_cached_lap_series(
                                plot_ui,
                                lap_render_cache,
                                &cache_key,
                                color,
                                width,
                            );
                        }

                        if let Some(cursor_time) = cursor_time {
                            Self::draw_cursor_line(
                                plot_ui,
                                viewport.axis_value_for_time(cursor_time),
                            );
                        }
                        if let Some(coord) = plot_ui.pointer_coordinate() {
                            tile_cursor = viewport.time_from_axis_value(coord.x);
                        }
                    });

                    let bounds = resp.transform.bounds();
                    let pair = (bounds.min()[0], bounds.max()[0]);
                    if first_bounds.is_none() {
                        first_bounds = Some(pair);
                    }
                    if resp.response.hovered() {
                        hovered_bounds = Some(pair);
                        hovered_cursor = tile_cursor;
                    }
                    Self::draw_group_legend(
                        ui,
                        resp.response.rect,
                        &grouped,
                        &channel_meta,
                        group,
                        cursor_time,
                        Some(viewport.time_range_from_plot_bounds(pair.0, pair.1)),
                    );
                    responses.push((group.clone(), resp.response));

                    if tile_idx + 1 < tile_groups.len() {
                        let delta = self.show_resize_handle(ui).y;
                        if delta.abs() > 0.0 {
                            Self::apply_adjacent_tile_resize(
                                &mut self.tile_heights,
                                tile_idx,
                                delta,
                            );
                        }
                    }
                }

                for (group, response) in &responses {
                    self.handle_tile_group_context_menu(response, group, &channel_meta, shared);
                }

                if shared.dragging_channel.is_some()
                    && ui.input(|i| i.pointer.any_released())
                    && ui.ui_contains_pointer()
                {
                    let target_group = responses
                        .iter()
                        .find(|(_, resp)| resp.contains_pointer())
                        .and_then(|(group, _)| group.first())
                        .and_then(|&idx| self.plotted_channels.get(idx))
                        .map(|channel| channel.tile_group);
                    if let Some(ch_id) = shared.dragging_channel.take() {
                        self.add_channel_to_group(
                            ch_id,
                            target_group.unwrap_or_else(|| self.next_tile_group()),
                            shared,
                        );
                        if target_group.is_none() {
                            self.tile_heights.clear();
                        }
                        self.needs_zoom_reset = true;
                    }
                }
            });

        if let Some(cursor_time) = hovered_cursor {
            shared.cursor_time = Some(cursor_time);
        }
        if needs_zoom_reset {
            shared.zoom_range = Some((
                viewport.reference_lap.start_time,
                viewport.reference_lap.end_time,
            ));
        } else if !zoom_from_timeline && let Some((x_min, x_max)) = hovered_bounds.or(first_bounds)
        {
            shared.zoom_range = Some(viewport.time_range_from_plot_bounds(x_min, x_max));
        }
    }

    /// Clamp an X range to [0, duration], preserving width.
    fn clamp_x_range(x_min: f64, x_max: f64, duration: f64) -> (f64, f64) {
        let width = x_max - x_min;
        if width >= duration {
            return (0.0, duration);
        }
        let mut min = x_min;
        let mut max = x_max;
        if min < 0.0 {
            min = 0.0;
            max = width;
        }
        if max > duration {
            max = duration;
            min = duration - width;
        }
        (min, max)
    }

    /// Compute Y range from cached min/max values (O(n_channels), not O(n_samples)).
    fn compute_y_range(channels: &[&PlottedChannel]) -> Option<(f64, f64)> {
        let mut global_min = f64::MAX;
        let mut global_max = f64::MIN;
        let mut has_data = false;

        for pc in channels {
            if !pc.data.is_empty() {
                let mut display_min = pc.cached_min * pc.display_scale + pc.display_offset;
                let mut display_max = pc.cached_max * pc.display_scale + pc.display_offset;
                if pc.display_scale < 0.0 {
                    std::mem::swap(&mut display_min, &mut display_max);
                }
                if display_min < global_min {
                    global_min = display_min;
                }
                if display_max > global_max {
                    global_max = display_max;
                }
                has_data = true;
            }
        }

        if has_data {
            Some((global_min, global_max))
        } else {
            None
        }
    }

    fn draw_channels<'a>(
        plot_ui: &mut egui_plot::PlotUi<'a>,
        channels: &[&PlottedChannel],
        freq_map: &HashMap<ChannelId, u16>,
        x_axis: &ActiveGraphXAxis,
        shared: &SharedState,
        render_cache: &'a mut HashMap<ChannelId, GraphRenderCacheEntry>,
    ) {
        let bounds = plot_ui.plot_bounds();
        let x_min = bounds.min()[0];
        let x_max = bounds.max()[0];
        let pixels_wide = plot_ui.response().rect.width() as usize;
        let target_width = pixels_wide.max(100);
        let axis_fingerprint = match x_axis {
            ActiveGraphXAxis::Time => GraphAxisFingerprint::Time,
            ActiveGraphXAxis::Distance { data, freq } => GraphAxisFingerprint::Distance {
                data_ptr: data.as_ptr() as usize,
                data_len: data.len(),
                freq: *freq,
            },
        };

        for pc in channels {
            let freq = freq_map.get(&pc.channel_id).copied().unwrap_or(0);
            if freq == 0 {
                continue;
            }

            let total_samples = pc.data.len();
            let total_duration = total_samples.saturating_sub(1) as f64 / freq as f64;
            let visible_t_min = x_axis
                .time_from_axis_value(x_min, total_duration)
                .unwrap_or(0.0);
            let visible_t_max = x_axis
                .time_from_axis_value(x_max, total_duration)
                .unwrap_or(total_duration);
            let (visible_t_min, visible_t_max) = if visible_t_min <= visible_t_max {
                (visible_t_min, visible_t_max)
            } else {
                (visible_t_max, visible_t_min)
            };

            let start_sample = if visible_t_min > 0.0 {
                ((visible_t_min * freq as f64) as usize).min(total_samples)
            } else {
                0
            };
            let end_sample = if x_max > 0.0 {
                ((visible_t_max * freq as f64) as usize + 1).min(total_samples)
            } else {
                total_samples
            };

            if start_sample >= end_sample {
                continue;
            }

            let preserve_native_samples =
                preserve_native_samples_for_channel(pc.channel_id, shared);
            let fingerprint = GraphRenderFingerprint {
                data_ptr: pc.data.as_ptr() as usize,
                data_len: pc.data.len(),
                x_bounds: (x_min.to_bits(), x_max.to_bits()),
                axis: axis_fingerprint.clone(),
                transform: display_transform_fingerprint(pc),
                target_width,
                preserve_native_samples,
            };

            let cache_entry =
                render_cache
                    .entry(pc.channel_id)
                    .or_insert_with(|| GraphRenderCacheEntry {
                        fingerprint: fingerprint.clone(),
                        plot_points: Vec::new(),
                    });

            if cache_entry.fingerprint != fingerprint {
                cache_entry.fingerprint = fingerprint.clone();
                cache_entry.plot_points.clear();
            }

            if cache_entry.plot_points.is_empty() {
                let samples = SampleRange {
                    start: start_sample,
                    end: end_sample,
                };
                let visible_sample_count = samples.len();
                let should_background_downsample = !preserve_native_samples
                    && visible_sample_count > target_width.saturating_mul(4);
                let downsampled = if preserve_native_samples {
                    cache_entry.plot_points = native_plot_points_for_range(
                        &pc.data,
                        freq,
                        samples,
                        x_axis,
                        PlotXRange {
                            min: x_min,
                            max: x_max,
                        },
                        1.0,
                        0.0,
                    );
                    None
                } else {
                    match pc.channel_id {
                        ChannelId::Physical(channel_idx) => {
                            if let Some(decoded) =
                                shared.decoded_physical_channel_if_ready(channel_idx)
                            {
                                decoded
                                    .best_lod_level_for_view(visible_sample_count, target_width)
                                    .map(|level| {
                                        level
                                            .points
                                            .iter()
                                            .copied()
                                            .filter(|point| {
                                                let point_time = point.time;
                                                point_time >= visible_t_min
                                                    && point_time <= visible_t_max
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                    .filter(|points| !points.is_empty())
                                    .or_else(|| {
                                        downsample_series_for_range(
                                            shared,
                                            &pc.data,
                                            freq,
                                            samples,
                                            target_width,
                                            should_background_downsample,
                                        )
                                    })
                            } else {
                                shared.request_physical_channel_decode(channel_idx);
                                None
                            }
                        }
                        ChannelId::Math(_) => downsample_series_for_range(
                            shared,
                            &pc.data,
                            freq,
                            samples,
                            target_width,
                            should_background_downsample,
                        ),
                    }
                };

                if let Some(downsampled) = downsampled {
                    cache_entry.plot_points = downsampled
                        .iter()
                        .map(|point| {
                            egui_plot::PlotPoint::new(
                                x_axis.axis_value_at_time(point.time),
                                ((point.min + point.max) / 2.0) * pc.display_scale
                                    + pc.display_offset,
                            )
                        })
                        .filter(|point| point.x >= x_min && point.x <= x_max)
                        .collect();
                }
            }
        }

        let render_cache = &*render_cache;
        for pc in channels {
            let Some(cache_entry) = render_cache.get(&pc.channel_id) else {
                continue;
            };
            if cache_entry.plot_points.is_empty() {
                continue;
            }

            let line = Line::new("", PlotPoints::Borrowed(cache_entry.plot_points.as_slice()))
                .color(pc.color)
                .width(1.5);
            plot_ui.line(line);
        }
    }

    fn draw_legend(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        channels: &[&PlottedChannel],
        channel_meta: &[ChannelDisplayMeta],
        cursor_time: Option<f64>,
        visible_time_range: Option<(f64, f64)>,
    ) {
        let line_height = 15.0;
        let pad = 4.0;
        for (i, pc) in channels.iter().enumerate() {
            if let Some(meta) = channel_meta.get(i) {
                let y = plot_rect.top() + pad + i as f32 * line_height;
                Self::draw_legend_entry(
                    ui,
                    plot_rect,
                    pc,
                    meta,
                    cursor_time,
                    visible_time_range,
                    y,
                );
            }
        }
    }

    fn draw_group_legend(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        channels: &[&PlottedChannel],
        all_meta: &[ChannelDisplayMeta],
        group: &[usize],
        cursor_time: Option<f64>,
        visible_time_range: Option<(f64, f64)>,
    ) {
        let line_height = 15.0;
        let pad = 4.0;
        for (row, &channel_idx) in group.iter().enumerate() {
            if let (Some(pc), Some(meta)) = (channels.get(row), all_meta.get(channel_idx)) {
                let y = plot_rect.top() + pad + row as f32 * line_height;
                Self::draw_legend_entry(
                    ui,
                    plot_rect,
                    pc,
                    meta,
                    cursor_time,
                    visible_time_range,
                    y,
                );
            }
        }
    }

    fn draw_legend_entry(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        pc: &PlottedChannel,
        meta: &ChannelDisplayMeta,
        cursor_time: Option<f64>,
        visible_time_range: Option<(f64, f64)>,
        y: f32,
    ) {
        let (ref name, ref unit, freq, dec_places, ref enum_labels) = *meta;
        let painter = ui.painter();
        let font = egui::FontId::proportional(12.0);
        let pad = 4.0;

        let min_color = egui::Color32::from_rgb(80, 140, 255);
        let max_color = egui::Color32::from_rgb(255, 80, 80);
        let avg_color = egui::Color32::from_rgb(255, 180, 50);

        let swatch = egui::Rect::from_min_size(
            egui::pos2(plot_rect.left() + pad, y + 2.0),
            egui::vec2(8.0, 8.0),
        );
        painter.rect_filled(swatch, 1.0, pc.color);

        let name_x = swatch.right() + 4.0;
        let label = if unit.is_empty() {
            name.clone()
        } else {
            format!("{} [{}]", name, unit)
        };
        let label_width = painter
            .layout_no_wrap(label.clone(), font.clone(), pc.color)
            .size()
            .x;
        let mut left_cluster_right = name_x + label_width;

        let dec = dec_places.max(0) as usize;
        let fmt = |v: f64| -> String {
            format_enum_value(name, enum_labels, v)
                .unwrap_or_else(|| format!("{:.prec$}", v, prec = dec))
        };

        let cursor_text = cursor_time.map(|t| {
            let raw_val = crate::panels::cursor_readout::value_at_time(
                &pc.data,
                freq,
                t,
                uses_discrete_values(name, enum_labels),
            );
            let val = transformed_value_for_display(pc, name, enum_labels, raw_val);
            fmt(val)
        });
        if let Some(cursor_text) = &cursor_text {
            let width = painter
                .layout_no_wrap(cursor_text.clone(), font.clone(), pc.color)
                .size()
                .x;
            left_cluster_right += 16.0 + width;
        }

        let visible_stats =
            Self::display_stats_for_time_range(pc, name, enum_labels, freq, visible_time_range);
        let right_cluster_rect = visible_stats.as_ref().map(|stats| {
            let icon_size = 7.0;
            let spacing = 6.0;
            let avg_width = painter
                .layout_no_wrap(fmt(stats.avg), font.clone(), avg_color)
                .size()
                .x;
            let max_width = painter
                .layout_no_wrap(fmt(stats.max), font.clone(), max_color)
                .size()
                .x;
            let min_width = painter
                .layout_no_wrap(fmt(stats.min), font.clone(), min_color)
                .size()
                .x;
            let total_width = avg_width + max_width + min_width + 3.0 * icon_size + 9.0 * spacing;
            egui::Rect::from_min_max(
                egui::pos2(plot_rect.right() - pad - total_width - pad, y - 2.0),
                egui::pos2(plot_rect.right() - 2.0, y + 15.0),
            )
        });

        let left_cluster_rect = egui::Rect::from_min_max(
            egui::pos2(plot_rect.left() + 2.0, y - 2.0),
            egui::pos2(left_cluster_right + pad, y + 15.0),
        );
        painter.rect_filled(
            left_cluster_rect,
            4.0,
            egui::Color32::from_rgba_premultiplied(10, 10, 14, 150),
        );
        if let Some(rect) = right_cluster_rect {
            painter.rect_filled(
                rect,
                4.0,
                egui::Color32::from_rgba_premultiplied(10, 10, 14, 150),
            );
        }

        let label_rect = painter.text(
            egui::pos2(name_x, y),
            egui::Align2::LEFT_TOP,
            &label,
            font.clone(),
            pc.color,
        );

        if let Some(cursor_text) = cursor_text {
            painter.text(
                egui::pos2(label_rect.right() + 16.0, y),
                egui::Align2::LEFT_TOP,
                cursor_text,
                font.clone(),
                pc.color,
            );
        }

        // Right side: colored min/max/avg stats (i2 style)
        if let Some(stats) = visible_stats {
            let icon_size = 7.0;
            let spacing = 6.0;
            let mut x = plot_rect.right() - pad;
            let icon_y = y + 3.0;

            // Helper: draw a stat value with a colored square icon, positioned right-to-left
            let draw_stat = |x: &mut f32, value: f64, color: egui::Color32| {
                let text = fmt(value);
                let rect = painter.text(
                    egui::pos2(*x, y),
                    egui::Align2::RIGHT_TOP,
                    &text,
                    font.clone(),
                    color,
                );
                *x = rect.left() - spacing;
                let icon = egui::Rect::from_min_size(
                    egui::pos2(*x - icon_size, icon_y),
                    egui::vec2(icon_size, icon_size),
                );
                painter.rect_filled(icon, 1.0, color);
                *x = icon.left() - spacing * 2.0;
            };

            draw_stat(&mut x, stats.avg, avg_color);

            // Max uses a triangle icon instead of a square
            let max_text = fmt(stats.max);
            let max_rect = painter.text(
                egui::pos2(x, y),
                egui::Align2::RIGHT_TOP,
                &max_text,
                font.clone(),
                max_color,
            );
            x = max_rect.left() - spacing;
            let tri_cx = x - icon_size / 2.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(tri_cx, icon_y),
                    egui::pos2(tri_cx - icon_size / 2.0, icon_y + icon_size),
                    egui::pos2(tri_cx + icon_size / 2.0, icon_y + icon_size),
                ],
                max_color,
                egui::Stroke::NONE,
            ));
            x = tri_cx - icon_size / 2.0 - spacing * 2.0;

            draw_stat(&mut x, stats.min, min_color);
        }
    }

    fn display_stats_for_time_range(
        pc: &PlottedChannel,
        name: &str,
        enum_labels: &HashMap<i64, String>,
        freq: u16,
        visible_time_range: Option<(f64, f64)>,
    ) -> Option<DisplayStats> {
        let stats = visible_time_range
            .and_then(|range| Self::channel_stats_in_time_range(&pc.data, freq, range))
            .unwrap_or_else(|| compute_channel_stats(&pc.data));
        if !stats.min.is_finite() && !stats.max.is_finite() && !stats.avg.is_finite() {
            return None;
        }

        let avg = transformed_value_for_display(pc, name, enum_labels, stats.avg);
        let mut min = transformed_value_for_display(pc, name, enum_labels, stats.min);
        let mut max = transformed_value_for_display(pc, name, enum_labels, stats.max);
        if !uses_discrete_values(name, enum_labels) && pc.display_scale < 0.0 {
            std::mem::swap(&mut min, &mut max);
        }
        Some(DisplayStats { min, max, avg })
    }

    fn channel_stats_in_time_range(
        data: &[f64],
        freq: u16,
        visible_time_range: (f64, f64),
    ) -> Option<crate::state::ChannelStats> {
        if data.is_empty() || freq == 0 {
            return None;
        }
        let (time_min, time_max) = if visible_time_range.0 <= visible_time_range.1 {
            visible_time_range
        } else {
            (visible_time_range.1, visible_time_range.0)
        };
        let start = ((time_min.max(0.0) * freq as f64).floor() as usize).min(data.len());
        let end = ((time_max.max(0.0) * freq as f64).ceil() as usize + 1).min(data.len());
        (start < end).then(|| compute_channel_stats(&data[start..end]))
    }

    fn ensure_tile_heights(&mut self, tile_count: usize, available_height: f32) {
        if tile_count == 0 {
            self.tile_heights.clear();
            return;
        }
        let default_height = (available_height / tile_count as f32).max(80.0);
        if self.tile_heights.len() < tile_count {
            self.tile_heights.resize(tile_count, default_height);
        } else if self.tile_heights.len() > tile_count {
            self.tile_heights.truncate(tile_count);
        }
        for height in &mut self.tile_heights {
            *height = height.max(80.0);
        }
    }

    fn apply_adjacent_tile_resize(tile_heights: &mut [f32], upper_idx: usize, delta: f32) {
        if upper_idx + 1 >= tile_heights.len() {
            return;
        }
        let min_height = 80.0;
        let max_down = tile_heights[upper_idx + 1] - min_height;
        let max_up = tile_heights[upper_idx] - min_height;
        let applied = delta.clamp(-max_up, max_down);
        tile_heights[upper_idx] += applied;
        tile_heights[upper_idx + 1] -= applied;
    }

    fn show_resize_handle(&self, ui: &mut egui::Ui) -> egui::Vec2 {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 8.0),
            egui::Sense::click_and_drag(),
        );
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        response.drag_motion()
    }

    fn draw_cursor_line(plot_ui: &mut egui_plot::PlotUi, axis_value: f64) {
        let cursor_line = VLine::new("cursor", axis_value)
            .color(egui::Color32::from_rgb(255, 255, 0))
            .width(1.0);
        plot_ui.vline(cursor_line);
    }

    fn draw_lap_markers(plot_ui: &mut egui_plot::PlotUi, laps: &[Lap], x_axis: &ActiveGraphXAxis) {
        let marker_color = egui::Color32::from_rgba_premultiplied(200, 200, 200, 80);
        for lap in laps {
            let vline = VLine::new(&lap.name, x_axis.axis_value_at_time(lap.start_time))
                .color(marker_color)
                .width(1.0)
                .style(egui_plot::LineStyle::dashed_dense());
            plot_ui.vline(vline);
        }
    }

    fn handle_context_menu(&mut self, response: &egui::Response, shared: &mut SharedState) {
        response.context_menu(|ui| {
            if ui.button("Manage Channels...").clicked() {
                self.show_manage_channels_window = true;
                ui.close();
            }
            ui.separator();
            ui.label("Channels:");
            ui.separator();

            let mut action: Option<ContextAction> = None;

            for pc in &self.plotted_channels {
                let (name, raw_unit, _, _) = resolve_channel_meta(pc.channel_id, shared);
                ui.menu_button(&name, |ui| {
                    Self::show_channel_menu(ui, pc, &raw_unit, shared, &mut action);
                });
            }

            if let Some(act) = action {
                self.apply_context_action(act, shared);
            }
        });
    }

    fn handle_tile_group_context_menu(
        &mut self,
        response: &egui::Response,
        group: &[usize],
        all_meta: &[ChannelDisplayMeta],
        shared: &mut SharedState,
    ) {
        let names: Vec<String> = group
            .iter()
            .filter_map(|&idx| all_meta.get(idx).map(|(name, _, _, _, _)| name.clone()))
            .collect();

        response.context_menu(|ui| {
            if ui.button("Manage Channels...").clicked() {
                self.show_manage_channels_window = true;
                ui.close();
            }
            ui.separator();
            ui.label("Channels");
            ui.separator();

            let mut action: Option<ContextAction> = None;
            for &idx in group {
                let Some(pc) = self.plotted_channels.get(idx) else {
                    continue;
                };
                let name = names
                    .get(group.iter().position(|&g| g == idx).unwrap_or(0))
                    .cloned()
                    .unwrap_or_default();
                let raw_unit = resolve_channel_meta(pc.channel_id, shared).1;
                ui.menu_button(name, |ui| {
                    Self::show_channel_menu(ui, pc, &raw_unit, shared, &mut action);
                });
            }

            if let Some(action) = action {
                self.apply_context_action(action, shared);
            }
        });
    }

    fn show_channel_menu(
        ui: &mut egui::Ui,
        pc: &PlottedChannel,
        raw_unit: &str,
        shared: &SharedState,
        action: &mut Option<ContextAction>,
    ) {
        if ui.button("Remove").clicked() {
            *action = Some(ContextAction::Remove(pc.channel_id));
            ui.close();
        }
        if ui.button("Show as Gauge").clicked() {
            *action = Some(ContextAction::AddGauge(pc.channel_id));
            ui.close();
        }
        ui.separator();
        ui.label("Color:");
        for (i, &c) in CHANNEL_COLORS.iter().enumerate() {
            let label = format!("Color {}", i + 1);
            let resp = ui.selectable_label(pc.color == c, &label);
            let rect = resp.rect;
            let swatch = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 14.0, rect.center().y - 5.0),
                egui::vec2(10.0, 10.0),
            );
            ui.painter().rect_filled(swatch, 2.0, c);
            if resp.clicked() {
                *action = Some(ContextAction::ChangeColor(pc.channel_id, c));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Move to Left Y-axis").clicked() {
            *action = Some(ContextAction::SetYAxis(pc.channel_id, YAxis::Left));
            ui.close();
        }
        if ui.button("Move to Right Y-axis").clicked() {
            *action = Some(ContextAction::SetYAxis(pc.channel_id, YAxis::Right));
            ui.close();
        }

        let presets = display_presets_for_unit(raw_unit);
        if !presets.is_empty() || pc.display_unit.is_some() {
            ui.separator();
            ui.label("Display units:");
            let raw_selected =
                pc.display_scale == 1.0 && pc.display_offset == 0.0 && pc.display_unit.is_none();
            if ui
                .selectable_label(raw_selected, format!("Raw ({})", raw_unit))
                .clicked()
            {
                *action = Some(ContextAction::SetDisplayTransform(
                    pc.channel_id,
                    1.0,
                    0.0,
                    None,
                ));
                ui.close();
            }
            for preset in presets {
                let is_selected = (pc.display_scale - preset.scale).abs() < 1e-9
                    && (pc.display_offset - preset.offset).abs() < 1e-9
                    && pc.display_unit.as_deref() == Some(preset.unit);
                if ui.selectable_label(is_selected, preset.label).clicked() {
                    *action = Some(ContextAction::SetDisplayTransform(
                        pc.channel_id,
                        preset.scale,
                        preset.offset,
                        Some(preset.unit.to_string()),
                    ));
                    ui.close();
                }
            }
        }

        let pref_key = channel_preference_key(&resolve_channel_meta(pc.channel_id, shared).0);
        let has_global_pref = shared.channel_preferences.contains_key(&pref_key);
        ui.separator();
        if ui.button("Save current style as global default").clicked() {
            *action = Some(ContextAction::SaveGlobalPreference(
                pc.channel_id,
                ChannelPreference {
                    color: Some([pc.color.r(), pc.color.g(), pc.color.b()]),
                    display_scale: pc.display_scale,
                    display_offset: pc.display_offset,
                    display_unit: pc.display_unit.clone(),
                },
            ));
            ui.close();
        }
        if ui
            .add_enabled(has_global_pref, egui::Button::new("Apply global default"))
            .clicked()
        {
            *action = Some(ContextAction::ApplyGlobalPreference(pc.channel_id));
            ui.close();
        }
        if ui
            .add_enabled(has_global_pref, egui::Button::new("Clear global default"))
            .clicked()
        {
            *action = Some(ContextAction::ClearGlobalPreference(pc.channel_id));
            ui.close();
        }
    }

    fn tiled_channel_groups(&self) -> Vec<Vec<usize>> {
        let mut ordered_groups = Vec::new();
        let mut group_lookup: HashMap<usize, usize> = HashMap::new();

        for (channel_idx, plotted) in self.plotted_channels.iter().enumerate() {
            let bucket = if let Some(existing) = group_lookup.get(&plotted.tile_group) {
                *existing
            } else {
                let next = ordered_groups.len();
                ordered_groups.push(Vec::new());
                group_lookup.insert(plotted.tile_group, next);
                next
            };
            ordered_groups[bucket].push(channel_idx);
        }

        ordered_groups
    }

    fn tiled_channel_groups_with_keys(&self) -> Vec<(usize, Vec<usize>)> {
        let mut ordered_groups = Vec::new();
        let mut group_lookup: HashMap<usize, usize> = HashMap::new();

        for (channel_idx, plotted) in self.plotted_channels.iter().enumerate() {
            let bucket = if let Some(existing) = group_lookup.get(&plotted.tile_group) {
                *existing
            } else {
                let next = ordered_groups.len();
                ordered_groups.push((plotted.tile_group, Vec::new()));
                group_lookup.insert(plotted.tile_group, next);
                next
            };
            ordered_groups[bucket].1.push(channel_idx);
        }

        ordered_groups
    }

    fn normalize_tile_groups(&mut self) {
        let mut next_group = 0usize;
        let mut remap = HashMap::new();
        for channel in &mut self.plotted_channels {
            let mapped = *remap.entry(channel.tile_group).or_insert_with(|| {
                let current = next_group;
                next_group += 1;
                current
            });
            channel.tile_group = mapped;
        }
    }

    fn move_channel_before(&mut self, dragged: ChannelId, target: ChannelId) {
        let Some(from_idx) = self
            .plotted_channels
            .iter()
            .position(|pc| pc.channel_id == dragged)
        else {
            return;
        };
        let target_group = self
            .plotted_channels
            .iter()
            .find(|pc| pc.channel_id == target)
            .map(|pc| pc.tile_group);
        let Some(target_group) = target_group else {
            return;
        };

        let mut channel = self.plotted_channels.remove(from_idx);
        channel.tile_group = target_group;

        let Some(target_idx) = self
            .plotted_channels
            .iter()
            .position(|pc| pc.channel_id == target)
        else {
            self.plotted_channels.push(channel);
            self.normalize_tile_groups();
            return;
        };
        self.plotted_channels.insert(target_idx, channel);
        self.normalize_tile_groups();
    }

    fn move_channel_to_group_end(&mut self, dragged: ChannelId, group: usize) {
        let Some(from_idx) = self
            .plotted_channels
            .iter()
            .position(|pc| pc.channel_id == dragged)
        else {
            return;
        };
        let mut channel = self.plotted_channels.remove(from_idx);
        channel.tile_group = group;

        let insert_idx = self
            .plotted_channels
            .iter()
            .rposition(|pc| pc.tile_group == group)
            .map(|idx| idx + 1)
            .unwrap_or(self.plotted_channels.len());
        self.plotted_channels.insert(insert_idx, channel);
        self.normalize_tile_groups();
    }

    fn move_channel_to_new_graph(&mut self, channel_id: ChannelId) {
        let Some(from_idx) = self
            .plotted_channels
            .iter()
            .position(|pc| pc.channel_id == channel_id)
        else {
            return;
        };
        let mut channel = self.plotted_channels.remove(from_idx);
        channel.tile_group = self.next_tile_group();
        self.plotted_channels.push(channel);
        self.normalize_tile_groups();
        self.tile_heights.clear();
    }

    fn sync_manage_channels_state(&mut self) {
        match self.manage_channels_selection {
            Some(ManageChannelsSelection::Channel(channel_id))
                if !self.is_channel_plotted(channel_id) =>
            {
                self.manage_channels_selection = None;
            }
            Some(ManageChannelsSelection::Group(group))
                if !self
                    .plotted_channels
                    .iter()
                    .any(|pc| pc.tile_group == group) =>
            {
                self.manage_channels_selection = None;
            }
            _ => {}
        }
    }

    fn apply_context_action(&mut self, action: ContextAction, shared: &mut SharedState) {
        match action {
            ContextAction::Remove(id) => self.remove_channel(id),
            ContextAction::ChangeColor(id, color) => {
                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    pc.color = color;
                }
            }
            ContextAction::SetYAxis(id, axis) => {
                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    pc.y_axis = axis;
                }
            }
            ContextAction::SetDisplayTransform(id, scale, offset, unit) => {
                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    pc.display_scale = scale;
                    pc.display_offset = offset;
                    pc.display_unit = unit;
                }
            }
            ContextAction::SaveGlobalPreference(id, preference) => {
                let channel_name = resolve_channel_meta(id, shared).0;
                shared
                    .channel_preferences
                    .insert(channel_preference_key(&channel_name), preference.clone());
                shared.channel_preferences_dirty = true;

                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    if let Some(color) = preference.color {
                        pc.color = egui::Color32::from_rgb(color[0], color[1], color[2]);
                    }
                    pc.display_scale = preference.display_scale;
                    pc.display_offset = preference.display_offset;
                    pc.display_unit = preference.display_unit;
                }
            }
            ContextAction::ApplyGlobalPreference(id) => {
                let channel_name = resolve_channel_meta(id, shared).0;
                if let Some(preference) = shared
                    .channel_preferences
                    .get(&channel_preference_key(&channel_name))
                    .cloned()
                    && let Some(pc) = self
                        .plotted_channels
                        .iter_mut()
                        .find(|pc| pc.channel_id == id)
                {
                    if let Some(color) = preference.color {
                        pc.color = egui::Color32::from_rgb(color[0], color[1], color[2]);
                    }
                    pc.display_scale = preference.display_scale;
                    pc.display_offset = preference.display_offset;
                    pc.display_unit = preference.display_unit;
                }
            }
            ContextAction::ClearGlobalPreference(id) => {
                let channel_name = resolve_channel_meta(id, shared).0;
                shared
                    .channel_preferences
                    .remove(&channel_preference_key(&channel_name));
                shared.channel_preferences_dirty = true;
            }
            ContextAction::AddGauge(id) => {
                if self
                    .embedded_gauges
                    .iter()
                    .all(|g| g.channel.channel_id != id)
                    && let Some(pc) = self.plotted_channels.iter().find(|pc| pc.channel_id == id)
                {
                    let style = default_style_for_name("");
                    self.embedded_gauges.push(GaugeChannel {
                        channel: PlottedChannel {
                            channel_id: pc.channel_id,
                            color: pc.color,
                            data: pc.data.clone(),
                            tile_group: pc.tile_group,
                            y_axis: pc.y_axis,
                            display_scale: pc.display_scale,
                            display_offset: pc.display_offset,
                            display_unit: pc.display_unit.clone(),
                            cached_min: pc.cached_min,
                            cached_max: pc.cached_max,
                            cached_avg: pc.cached_avg,
                        },
                        style,
                    });
                }
            }
        }
    }

    fn show_manage_channels_window(&mut self, ctx: &egui::Context, shared: &mut SharedState) {
        if !self.show_manage_channels_window {
            self.show_add_channel_picker = false;
            self.add_channel_target = None;
            return;
        }

        self.sync_manage_channels_state();

        let mut open = self.show_manage_channels_window;
        let mut pending_actions = Vec::new();
        let mut pending_selection = self.manage_channels_selection;
        let mut open_picker = None;
        let viewport_id = egui::ViewportId::from_hash_of(format!("manage_channels_{}", self.id));
        ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title(format!("Manage Channels - {}", self.title))
                .with_inner_size([720.0, 560.0]),
            |viewport_ui, _class| {
                if viewport_ui.input(|i| i.viewport().close_requested()) {
                    open = false;
                    self.show_add_channel_picker = false;
                    self.add_channel_target = None;
                    self.add_channel_selection = None;
                    self.add_channel_filter.clear();
                    return;
                }

                let picker_ctx = viewport_ui.ctx().clone();
                let ui = &mut *viewport_ui;
                ui.horizontal(|ui| {
                    ui.label("Channels");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            open = false;
                        }
                    });
                });
                ui.small(
                    "Drag rows onto the drop lines to reorder them or move them into another graph.",
                );
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        let grouped = self.tiled_channel_groups_with_keys();
                        for (visual_group_idx, (tile_group, indices)) in grouped.iter().enumerate()
                        {
                            let group_selected =
                                pending_selection == Some(ManageChannelsSelection::Group(*tile_group));
                            ui.group(|ui| {
                                let header = ui.selectable_label(
                                    group_selected,
                                    format!("Graph {}", visual_group_idx + 1),
                                );
                                if header.clicked() {
                                    pending_selection =
                                        Some(ManageChannelsSelection::Group(*tile_group));
                                }
                                header.on_hover_text(format!("{} channel(s)", indices.len()));
                                ui.add_space(2.0);

                                for (row_pos, &channel_idx) in indices.iter().enumerate() {
                                    let pc = &self.plotted_channels[channel_idx];
                                    if row_pos == 0 {
                                        let start_drop = Self::manage_channel_drop_zone(
                                            ui,
                                            &format!(
                                                "Drop at start of Graph {}",
                                                visual_group_idx + 1
                                            ),
                                        );
                                        if let Some(payload) = start_drop
                                            .dnd_release_payload::<ManageChannelDragPayload>()
                                        {
                                            pending_actions.push(
                                                ManageChannelsAction::MoveBefore {
                                                    dragged: payload.channel_id,
                                                    target: pc.channel_id,
                                                },
                                            );
                                        }
                                    }

                                    let (name, unit, _, _, _) =
                                        resolve_plotted_channel_display_meta(pc, shared);
                                    let row_label = if unit.is_empty() {
                                        name.clone()
                                    } else {
                                        format!("{name} [{unit}]")
                                    };
                                    let row_selected = pending_selection
                                        == Some(ManageChannelsSelection::Channel(pc.channel_id));
                                    let drag_id = egui::Id::new((
                                        "manage_channel_row",
                                        self.id,
                                        Self::channel_id_key(pc.channel_id),
                                    ));
                                    let drag_response = ui.dnd_drag_source(
                                        drag_id,
                                        ManageChannelDragPayload {
                                            channel_id: pc.channel_id,
                                        },
                                        |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("::").weak());
                                                ui.colored_label(pc.color, "■");
                                                ui.add(
                                                    egui::Button::new(row_label.clone())
                                                        .selected(row_selected),
                                                )
                                            })
                                            .inner
                                        },
                                    );

                                    if drag_response.inner.clicked() {
                                        pending_selection =
                                            Some(ManageChannelsSelection::Channel(pc.channel_id));
                                    }

                                    let raw_unit = resolve_channel_meta(pc.channel_id, shared).1;
                                    drag_response.inner.context_menu(|ui| {
                                        let mut action = None;
                                        Self::show_channel_menu(
                                            ui,
                                            pc,
                                            &raw_unit,
                                            shared,
                                            &mut action,
                                        );
                                        if let Some(action) = action {
                                            pending_actions
                                                .push(ManageChannelsAction::Context(action));
                                        }
                                    });

                                    let drop_label = if row_pos + 1 == indices.len() {
                                        format!("Drop at end of Graph {}", visual_group_idx + 1)
                                    } else {
                                        "Drop here".to_string()
                                    };
                                    let row_drop =
                                        Self::manage_channel_drop_zone(ui, &drop_label);
                                    if let Some(payload) =
                                        row_drop.dnd_release_payload::<ManageChannelDragPayload>()
                                    {
                                        if row_pos + 1 < indices.len() {
                                            let next_idx = indices[row_pos + 1];
                                            let next_id =
                                                self.plotted_channels[next_idx].channel_id;
                                            pending_actions.push(
                                                ManageChannelsAction::MoveBefore {
                                                    dragged: payload.channel_id,
                                                    target: next_id,
                                                },
                                            );
                                        } else {
                                            pending_actions.push(
                                                ManageChannelsAction::MoveToGroupEnd {
                                                    dragged: payload.channel_id,
                                                    group: *tile_group,
                                                },
                                            );
                                        }
                                    }
                                }
                            });
                            ui.add_space(6.0);
                        }
                    });

                let selected_channel = match pending_selection {
                    Some(ManageChannelsSelection::Channel(channel_id))
                        if self.is_channel_plotted(channel_id) =>
                    {
                        Some(channel_id)
                    }
                    _ => None,
                };
                let selected_group = match pending_selection {
                    Some(ManageChannelsSelection::Group(group))
                        if self.plotted_channels.iter().any(|pc| pc.tile_group == group) =>
                    {
                        Some(group)
                    }
                    Some(ManageChannelsSelection::Channel(channel_id)) => self
                        .plotted_channels
                        .iter()
                        .find(|pc| pc.channel_id == channel_id)
                        .map(|pc| pc.tile_group),
                    _ => None,
                };

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Add Graph").clicked() {
                        if let Some(channel_id) = selected_channel {
                            pending_actions.push(ManageChannelsAction::MoveToNewGraph(channel_id));
                        } else {
                            open_picker = Some(AddChannelPickerTarget::NewGraph);
                        }
                    }

                    if ui.button("Add Channel...").clicked() {
                        open_picker = Some(match selected_group {
                            Some(group) => AddChannelPickerTarget::ExistingGroup(group),
                            None => AddChannelPickerTarget::NewGraph,
                        });
                    }

                    if ui
                        .add_enabled(selected_channel.is_some(), egui::Button::new("Remove"))
                        .clicked()
                        && let Some(channel_id) = selected_channel
                    {
                        pending_actions.push(ManageChannelsAction::Remove(channel_id));
                    }
                });

                ui.small("Add Channel... goes to the selected graph, or creates a new graph at the bottom if nothing is selected.");

                self.show_add_channel_picker_window(&picker_ctx, shared, &mut pending_actions);
            },
        );

        self.manage_channels_selection = pending_selection;
        if let Some(target) = open_picker {
            self.show_add_channel_picker = true;
            self.add_channel_target = Some(target);
            self.add_channel_filter.clear();
            self.add_channel_selection = None;
        }

        if !open {
            self.show_add_channel_picker = false;
            self.add_channel_target = None;
            self.add_channel_selection = None;
            self.add_channel_filter.clear();
        }

        self.show_manage_channels_window = open;

        for action in pending_actions {
            match action {
                ManageChannelsAction::Context(action) => self.apply_context_action(action, shared),
                ManageChannelsAction::MoveBefore { dragged, target } => {
                    if dragged != target {
                        self.move_channel_before(dragged, target);
                    }
                }
                ManageChannelsAction::MoveToGroupEnd { dragged, group } => {
                    self.move_channel_to_group_end(dragged, group);
                }
                ManageChannelsAction::MoveToNewGraph(channel_id) => {
                    self.move_channel_to_new_graph(channel_id);
                }
                ManageChannelsAction::AddToGroup { channel_id, group } => {
                    self.add_channel_to_group(channel_id, group, shared);
                }
                ManageChannelsAction::AddToNewGraph(channel_id) => {
                    self.add_channel(channel_id, shared);
                }
                ManageChannelsAction::Remove(channel_id) => self.remove_channel(channel_id),
            }
        }

        self.sync_manage_channels_state();
    }

    fn show_add_channel_picker_window(
        &mut self,
        ctx: &egui::Context,
        shared: &SharedState,
        pending_actions: &mut Vec<ManageChannelsAction>,
    ) {
        if !self.show_add_channel_picker {
            return;
        }

        let Some(target) = self.add_channel_target else {
            self.show_add_channel_picker = false;
            return;
        };

        let mut open = self.show_add_channel_picker;
        let mut close_picker = false;
        let title = match target {
            AddChannelPickerTarget::ExistingGroup(group) => {
                let visual_idx = self
                    .tiled_channel_groups_with_keys()
                    .iter()
                    .position(|(tile_group, _)| *tile_group == group)
                    .map(|idx| idx + 1)
                    .unwrap_or(group + 1);
                format!("Add Channel to Graph {visual_idx}")
            }
            AddChannelPickerTarget::NewGraph => "Add Channel to New Graph".to_string(),
        };

        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_size([420.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut self.add_channel_filter);
                });
                ui.separator();

                let filter = self.add_channel_filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for (channel_id, label) in self.available_channel_entries(shared) {
                            if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                                continue;
                            }

                            let already_plotted = self.is_channel_plotted(channel_id);
                            let selected = self.add_channel_selection == Some(channel_id);
                            ui.horizontal(|ui| {
                                let response = ui.add_enabled(
                                    !already_plotted,
                                    egui::Button::new(label.clone()).selected(selected),
                                );
                                if response.clicked() {
                                    self.add_channel_selection = Some(channel_id);
                                }
                                if response.double_clicked() {
                                    match target {
                                        AddChannelPickerTarget::ExistingGroup(group) => {
                                            pending_actions.push(
                                                ManageChannelsAction::AddToGroup {
                                                    channel_id,
                                                    group,
                                                },
                                            );
                                        }
                                        AddChannelPickerTarget::NewGraph => {
                                            pending_actions.push(
                                                ManageChannelsAction::AddToNewGraph(channel_id),
                                            );
                                        }
                                    }
                                    close_picker = true;
                                }
                                if already_plotted {
                                    ui.weak("Already plotted");
                                }
                            });
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            matches!(
                                self.add_channel_selection,
                                Some(channel_id) if !self.is_channel_plotted(channel_id)
                            ),
                            egui::Button::new("Add"),
                        )
                        .clicked()
                        && let Some(channel_id) = self.add_channel_selection
                    {
                        match target {
                            AddChannelPickerTarget::ExistingGroup(group) => {
                                pending_actions
                                    .push(ManageChannelsAction::AddToGroup { channel_id, group });
                            }
                            AddChannelPickerTarget::NewGraph => {
                                pending_actions
                                    .push(ManageChannelsAction::AddToNewGraph(channel_id));
                            }
                        }
                        close_picker = true;
                    }

                    if ui.button("Cancel").clicked() {
                        close_picker = true;
                    }
                });
            });

        if close_picker {
            open = false;
        }
        self.show_add_channel_picker = open;
        if !self.show_add_channel_picker {
            self.add_channel_target = None;
            self.add_channel_selection = None;
            self.add_channel_filter.clear();
        }
    }

    fn manage_channel_drop_zone(ui: &mut egui::Ui, label: &str) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
        if response
            .dnd_hover_payload::<ManageChannelDragPayload>()
            .is_some()
        {
            let fill = egui::Color32::from_rgba_premultiplied(70, 110, 170, 80);
            let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 170, 240));
            ui.painter().rect_filled(rect, 4.0, fill);
            ui.painter()
                .rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Outside);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
        }
        response
    }

    fn available_channel_entries(&self, shared: &SharedState) -> Vec<(ChannelId, String)> {
        let mut entries = Vec::new();

        if let Some(ld) = &shared.ld_file {
            let mut physical: Vec<_> = ld.channels.iter().collect();
            physical.sort_by_key(|channel| channel.name.to_lowercase());
            for channel in physical {
                let label = format!(
                    "{} [{}] {}Hz",
                    channel.name,
                    if channel.unit.is_empty() {
                        "-"
                    } else {
                        &channel.unit
                    },
                    channel.freq
                );
                entries.push((ChannelId::Physical(channel.index), label));
            }
        }

        let mut math_entries: Vec<_> = shared
            .math_channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.data.is_some())
            .map(|(idx, channel)| {
                let label = format!(
                    "f {} [{}] {}Hz",
                    channel.name,
                    if channel.unit.is_empty() {
                        "-"
                    } else {
                        &channel.unit
                    },
                    channel.freq
                );
                (ChannelId::Math(idx), label)
            })
            .collect();
        math_entries.sort_by_key(|(_, label)| label.to_lowercase());
        entries.extend(math_entries);

        entries
    }

    fn channel_id_key(channel_id: ChannelId) -> (u8, usize) {
        match channel_id {
            ChannelId::Physical(idx) => (0, idx),
            ChannelId::Math(idx) => (1, idx),
        }
    }

    fn resolve_x_axis(
        shared: &mut SharedState,
        requested_mode: GraphXAxis,
    ) -> (ActiveGraphXAxis, Option<String>) {
        if requested_mode == GraphXAxis::Time {
            return (ActiveGraphXAxis::Time, None);
        }

        if let Some(cache) = shared.distance_axis_cache.clone() {
            return (
                ActiveGraphXAxis::Distance {
                    data: cache.data,
                    freq: cache.freq,
                },
                None,
            );
        }

        let (derived_cache, waiting_on_sources) = derive_distance_axis_cache(shared);
        if let Some(cache) = derived_cache {
            shared.distance_axis_cache = Some(cache.clone());
            return (
                ActiveGraphXAxis::Distance {
                    data: cache.data,
                    freq: cache.freq,
                },
                None,
            );
        }

        (
            ActiveGraphXAxis::Time,
            Some(
                if waiting_on_sources {
                    "Distance X-axis is still loading source channels; falling back to time."
                } else {
                    "Distance X-axis unavailable for this session; falling back to time."
                }
                .into(),
            ),
        )
    }

    fn time_range_from_plot_bounds(
        x_axis: &ActiveGraphXAxis,
        axis_min: f64,
        axis_max: f64,
        duration: f64,
    ) -> (f64, f64) {
        let t0 = x_axis
            .time_from_axis_value(axis_min, duration)
            .unwrap_or(0.0);
        let t1 = x_axis
            .time_from_axis_value(axis_max, duration)
            .unwrap_or(duration);
        let (min_t, max_t) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        Self::clamp_x_range(min_t, max_t, duration)
    }

    fn resolve_overlay_render_data(
        &mut self,
        shared: &SharedState,
        overlay: &LapOverlay,
        channel_id: ChannelId,
    ) -> Option<ResolvedOverlayRenderData> {
        let (name, _, freq, _) = resolve_channel_meta(channel_id, shared);
        match overlay.source {
            OverlaySource::MainSession => {
                let lap = shared.laps.get(overlay.lap_idx)?.clone();
                let data = self
                    .plotted_channels
                    .iter()
                    .find(|pc| pc.channel_id == channel_id)?
                    .data
                    .clone();
                Some(ResolvedOverlayRenderData { data, freq, lap })
            }
            OverlaySource::External(session_idx) => {
                let session = self.overlay_sessions.get_mut(session_idx)?;
                let lap = session.laps.get(overlay.lap_idx)?.clone();
                let (data, freq) = resolve_overlay_channel_data(session, &name)?;
                Some(ResolvedOverlayRenderData { data, freq, lap })
            }
        }
    }

    fn prepare_overlay_series(
        &mut self,
        shared: &SharedState,
        x_axis: &ActiveGraphXAxis,
        viewport: &OverlayViewport,
        main_session_duration: f64,
    ) -> HashMap<ChannelId, Vec<PreparedLapOverlay>> {
        let mut prepared = HashMap::new();
        let channel_ids: Vec<ChannelId> = self
            .plotted_channels
            .iter()
            .map(|pc| pc.channel_id)
            .collect();

        for channel_id in channel_ids {
            let mut series = Vec::new();
            for overlay_idx in 0..self.lap_overlays.len() {
                let overlay = self.lap_overlays[overlay_idx];
                if let Some(rendered) =
                    self.resolve_overlay_render_data(shared, &overlay, channel_id)
                    && let Some(source_axis) =
                        self.overlay_axis_for_source(&overlay, x_axis, main_session_duration)
                    && let Some(raw_len) = lap_axis_length(
                        &source_axis.axis,
                        &rendered.lap,
                        source_axis.session_duration,
                    )
                {
                    let scale = if overlay.stretch_to_reference && raw_len > 0.0 {
                        viewport.reference_axis_length / raw_len
                    } else {
                        1.0
                    };
                    let lap = rendered.lap;
                    let origin_axis = lap_axis_origin(&source_axis.axis, &lap);
                    let axis_offset = overlay_axis_offset(
                        &source_axis.axis,
                        &lap,
                        source_axis.session_duration,
                        overlay.manual_offset,
                    );
                    let base_color = self
                        .plotted_channels
                        .iter()
                        .find(|pc| pc.channel_id == channel_id)
                        .map(|pc| pc.color)
                        .unwrap_or(CHANNEL_COLORS[overlay_idx % CHANNEL_COLORS.len()]);
                    series.push(PreparedLapOverlay {
                        data: rendered.data,
                        freq: rendered.freq,
                        lap,
                        origin_axis,
                        axis: source_axis.axis,
                        scale,
                        offset: axis_offset,
                        color: tint_color(base_color, overlay_idx + 1),
                        width: 1.35,
                        overlay_idx,
                    });
                }
            }
            prepared.insert(channel_id, series);
        }

        prepared
    }

    fn overlay_axis_for_source(
        &self,
        overlay: &LapOverlay,
        x_axis: &ActiveGraphXAxis,
        session_duration: f64,
    ) -> Option<OverlayAxisHandle> {
        match overlay.source {
            OverlaySource::MainSession => Some(OverlayAxisHandle {
                axis: x_axis.clone(),
                session_duration,
            }),
            OverlaySource::External(session_idx) => {
                let session = self.overlay_sessions.get(session_idx)?;
                match self.x_axis_mode {
                    GraphXAxis::Time => Some(OverlayAxisHandle {
                        axis: ActiveGraphXAxis::Time,
                        session_duration: session.ld_file.duration_secs(),
                    }),
                    GraphXAxis::Distance => {
                        session
                            .distance_axis_cache
                            .as_ref()
                            .map(|cache| OverlayAxisHandle {
                                axis: ActiveGraphXAxis::Distance {
                                    data: Arc::clone(&cache.data),
                                    freq: cache.freq,
                                },
                                session_duration: session.ld_file.duration_secs(),
                            })
                    }
                }
            }
        }
    }

    fn draw_overlay_summary(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        reference_lap: &Lap,
        overlays: &[LapOverlay],
        overlay_sessions: &[OverlaySession],
    ) {
        let painter = ui.painter();
        let mut lines = Vec::with_capacity(overlays.len() + 1);
        lines.push(format!(
            "Ref: {} ({})",
            reference_lap.name,
            i3rs_core::format_duration(reference_lap.duration())
        ));
        for overlay in overlays {
            let source = match overlay.source {
                OverlaySource::MainSession => "Current Session".to_string(),
                OverlaySource::External(session_idx) => overlay_sessions
                    .get(session_idx)
                    .map(|session| session.file_name.clone())
                    .unwrap_or_else(|| "External".into()),
            };
            lines.push(format!(
                "{}  offset {:+.2}s{}",
                source,
                overlay.manual_offset,
                if overlay.stretch_to_reference {
                    "  stretch"
                } else {
                    ""
                }
            ));
        }

        let font = egui::FontId::proportional(11.0);
        let line_height = 14.0;
        let width = 220.0;
        let height = line_height * lines.len() as f32 + 8.0;
        let rect = egui::Rect::from_min_size(
            egui::pos2(plot_rect.right() - width - 6.0, plot_rect.top() + 6.0),
            egui::vec2(width, height),
        );
        painter.rect_filled(
            rect,
            6.0,
            egui::Color32::from_rgba_premultiplied(8, 8, 12, 170),
        );
        for (idx, line) in lines.iter().enumerate() {
            painter.text(
                egui::pos2(
                    rect.left() + 8.0,
                    rect.top() + 4.0 + idx as f32 * line_height,
                ),
                egui::Align2::LEFT_TOP,
                line,
                font.clone(),
                egui::Color32::LIGHT_GRAY,
            );
        }
    }
}

struct OverlayAxisHandle {
    axis: ActiveGraphXAxis,
    session_duration: f64,
}

struct ResolvedOverlayRenderData {
    data: Arc<[f64]>,
    freq: u16,
    lap: Lap,
}

struct PreparedLapOverlay {
    data: Arc<[f64]>,
    freq: u16,
    lap: Lap,
    axis: ActiveGraphXAxis,
    origin_axis: f64,
    scale: f64,
    offset: f64,
    color: egui::Color32,
    width: f32,
    overlay_idx: usize,
}

#[derive(Clone)]
enum ActiveGraphXAxis {
    Time,
    Distance { data: Arc<[f64]>, freq: u16 },
}

impl ActiveGraphXAxis {
    fn label(&self) -> &'static str {
        match self {
            Self::Time => "Time (s)",
            Self::Distance { .. } => "Distance (m)",
        }
    }

    fn axis_value_at_time(&self, time: f64) -> f64 {
        match self {
            Self::Time => time,
            Self::Distance { data, freq } => super::utils::interp_at_time(data, *freq, time)
                .unwrap_or_else(|| data.last().copied().unwrap_or(0.0)),
        }
    }

    fn axis_range_for_time_range(&self, time_min: f64, time_max: f64) -> (f64, f64) {
        let a = self.axis_value_at_time(time_min);
        let b = self.axis_value_at_time(time_max);
        if a <= b { (a, b) } else { (b, a) }
    }

    fn full_range(&self, duration: f64) -> (f64, f64) {
        match self {
            Self::Time => (0.0, duration),
            Self::Distance { data, .. } => (0.0, data.last().copied().unwrap_or(0.0)),
        }
    }

    fn time_from_axis_value(&self, axis_value: f64, duration: f64) -> Option<f64> {
        match self {
            Self::Time => Some(axis_value.clamp(0.0, duration)),
            Self::Distance { data, freq } => time_from_monotonic_axis(data, *freq, axis_value),
        }
    }
}

struct OverlayViewport {
    reference_lap: Lap,
    reference_axis: ActiveGraphXAxis,
    reference_session_duration: f64,
    reference_origin_axis: f64,
    reference_axis_length: f64,
}

impl OverlayViewport {
    fn new(
        reference_lap: Lap,
        reference_axis: ActiveGraphXAxis,
        reference_session_duration: f64,
    ) -> Option<Self> {
        let reference_origin_axis = lap_axis_origin(&reference_axis, &reference_lap);
        let reference_axis_length =
            lap_axis_length(&reference_axis, &reference_lap, reference_session_duration)?;
        Some(Self {
            reference_lap,
            reference_axis,
            reference_session_duration,
            reference_origin_axis,
            reference_axis_length,
        })
    }

    fn axis_label(&self) -> &'static str {
        match self.reference_axis {
            ActiveGraphXAxis::Time => "Lap Time (s)",
            ActiveGraphXAxis::Distance { .. } => "Lap Distance (m)",
        }
    }

    fn full_range(&self) -> (f64, f64) {
        (0.0, self.reference_axis_length.max(0.001))
    }

    fn axis_value_for_time(&self, absolute_time: f64) -> f64 {
        self.reference_axis.axis_value_at_time(absolute_time) - self.reference_origin_axis
    }

    fn axis_range_for_time_range(&self, time_min: f64, time_max: f64) -> (f64, f64) {
        let a = self.axis_value_for_time(time_min);
        let b = self.axis_value_for_time(time_max);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (full_min, full_max) = self.full_range();
        (a.clamp(full_min, full_max), b.clamp(full_min, full_max))
    }

    fn time_from_axis_value(&self, relative_axis_value: f64) -> Option<f64> {
        self.reference_axis
            .time_from_axis_value(
                relative_axis_value + self.reference_origin_axis,
                self.reference_session_duration,
            )
            .map(|time| time.clamp(self.reference_lap.start_time, self.reference_lap.end_time))
    }

    fn time_range_from_plot_bounds(&self, axis_min: f64, axis_max: f64) -> (f64, f64) {
        let t0 = self
            .time_from_axis_value(axis_min)
            .unwrap_or(self.reference_lap.start_time);
        let t1 = self
            .time_from_axis_value(axis_max)
            .unwrap_or(self.reference_lap.end_time);
        let (min_t, max_t) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        (min_t, max_t)
    }
}

fn graph_axis_fingerprint(x_axis: &ActiveGraphXAxis) -> GraphAxisFingerprint {
    match x_axis {
        ActiveGraphXAxis::Time => GraphAxisFingerprint::Time,
        ActiveGraphXAxis::Distance { data, freq } => GraphAxisFingerprint::Distance {
            data_ptr: data.as_ptr() as usize,
            data_len: data.len(),
            freq: *freq,
        },
    }
}

fn downsample_series_for_range(
    shared: &SharedState,
    data: &Arc<[f64]>,
    freq: u16,
    samples: SampleRange,
    target_width: usize,
    use_background_cache: bool,
) -> Option<Vec<i3rs_core::DownsampledPoint>> {
    if use_background_cache {
        let key = DownsampleSeriesKey {
            data_ptr: data.as_ptr() as usize,
            data_len: data.len(),
            freq,
            start_sample: samples.start,
            end_sample: samples.end,
            target_width,
        };
        if let Some(points) = shared.downsampled_series_if_ready(&key) {
            Some(points.iter().copied().collect())
        } else {
            shared.request_downsampled_series(DownsampleSeriesRequest {
                key,
                data: Arc::clone(data),
                freq,
                start_sample: samples.start,
                end_sample: samples.end,
                target_width,
            });
            None
        }
    } else {
        Some(downsample_minmax(
            &data[samples.start..samples.end],
            freq,
            samples.start,
            target_width,
        ))
    }
}

fn ensure_lap_series_cache(
    plot_ui: &egui_plot::PlotUi,
    shared: &SharedState,
    render_cache: &mut HashMap<LapRenderCacheKey, LapRenderCacheEntry>,
    cache_key: LapRenderCacheKey,
    params: LapSeriesRenderParams<'_>,
) {
    let LapSeriesRenderParams {
        data,
        freq,
        lap,
        axis,
        transform,
        target_width,
        preserve_native_samples,
    } = params;

    if freq == 0 || data.is_empty() {
        return;
    }

    let start_sample = ((lap.start_time * freq as f64).floor() as usize).min(data.len());
    let end_sample = ((lap.end_time * freq as f64).ceil() as usize + 1).min(data.len());
    if start_sample >= end_sample {
        return;
    }

    let bounds = plot_ui.plot_bounds();
    let x_min = bounds.min()[0];
    let x_max = bounds.max()[0];
    let fingerprint = LapRenderFingerprint {
        data_ptr: data.as_ptr() as usize,
        data_len: data.len(),
        lap_bounds: (lap.start_time.to_bits(), lap.end_time.to_bits()),
        x_bounds: (x_min.to_bits(), x_max.to_bits()),
        axis: graph_axis_fingerprint(axis),
        origin_axis_bits: transform.origin_axis.to_bits(),
        x_scale_bits: transform.x_scale.to_bits(),
        x_offset_bits: transform.x_offset.to_bits(),
        transform: DisplayTransformFingerprint {
            scale_bits: transform.y_scale.to_bits(),
            offset_bits: transform.y_offset.to_bits(),
            unit: None,
        },
        target_width,
        preserve_native_samples,
    };

    let cache_entry = render_cache
        .entry(cache_key)
        .or_insert_with(|| LapRenderCacheEntry {
            fingerprint: fingerprint.clone(),
            plot_points: Vec::new(),
        });

    if cache_entry.fingerprint != fingerprint || cache_entry.plot_points.is_empty() {
        let samples = SampleRange {
            start: start_sample,
            end: end_sample,
        };
        let visible_sample_count = samples.len();
        if preserve_native_samples {
            cache_entry.fingerprint = fingerprint;
            cache_entry.plot_points = native_lap_plot_points_for_range(
                data,
                freq,
                samples,
                axis,
                PlotXRange {
                    min: x_min,
                    max: x_max,
                },
                PlotTransform {
                    origin_axis: transform.origin_axis,
                    x_scale: transform.x_scale,
                    x_offset: transform.x_offset,
                    y_scale: 1.0,
                    y_offset: 0.0,
                },
            );
            return;
        }

        let should_background_downsample = visible_sample_count > target_width.saturating_mul(4);
        let downsampled = downsample_series_for_range(
            shared,
            data,
            freq,
            samples,
            target_width,
            should_background_downsample,
        );

        if let Some(downsampled) = downsampled {
            cache_entry.fingerprint = fingerprint;
            cache_entry.plot_points = downsampled
                .iter()
                .map(|point| {
                    let transformed_x = (axis.axis_value_at_time(point.time)
                        - transform.origin_axis)
                        * transform.x_scale
                        + transform.x_offset;
                    egui_plot::PlotPoint::new(
                        transformed_x,
                        ((point.min + point.max) / 2.0) * transform.y_scale + transform.y_offset,
                    )
                })
                .filter(|point| point.x >= x_min && point.x <= x_max)
                .collect();
        }
    }
}

fn draw_cached_lap_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    render_cache: &'a HashMap<LapRenderCacheKey, LapRenderCacheEntry>,
    cache_key: &LapRenderCacheKey,
    color: egui::Color32,
    width: f32,
) {
    let Some(cache_entry) = render_cache.get(cache_key) else {
        return;
    };
    if cache_entry.plot_points.is_empty() {
        return;
    }

    plot_ui.line(
        Line::new("", PlotPoints::Borrowed(cache_entry.plot_points.as_slice()))
            .color(color)
            .width(width),
    );
}

fn lap_axis_origin(axis: &ActiveGraphXAxis, lap: &Lap) -> f64 {
    axis.axis_value_at_time(lap.start_time)
}

fn lap_axis_length(axis: &ActiveGraphXAxis, lap: &Lap, session_duration: f64) -> Option<f64> {
    let start = axis.axis_value_at_time(lap.start_time);
    let end = axis.axis_value_at_time(lap.end_time.min(session_duration));
    Some((end - start).abs())
}

fn overlay_axis_offset(
    axis: &ActiveGraphXAxis,
    lap: &Lap,
    session_duration: f64,
    manual_offset: f64,
) -> f64 {
    match axis {
        ActiveGraphXAxis::Time => manual_offset,
        ActiveGraphXAxis::Distance { .. } => {
            let shifted_time = (lap.start_time + manual_offset).clamp(0.0, session_duration);
            axis.axis_value_at_time(shifted_time) - axis.axis_value_at_time(lap.start_time)
        }
    }
}

fn tint_color(color: egui::Color32, offset: usize) -> egui::Color32 {
    let lighten = (offset as f32 * 0.12).min(0.45);
    let lerp =
        |component: u8| -> u8 { (component as f32 + (255.0 - component as f32) * lighten) as u8 };
    egui::Color32::from_rgba_premultiplied(lerp(color.r()), lerp(color.g()), lerp(color.b()), 220)
}

fn resolve_overlay_channel_data(
    session: &mut OverlaySession,
    requested_name: &str,
) -> Option<(Arc<[f64]>, u16)> {
    let normalized_requested = normalized_name(requested_name);
    if let Some(cached) = session.channel_cache.get(&normalized_requested) {
        return Some((Arc::clone(&cached.data), cached.freq));
    }

    let channel = session.ld_file.find_channel_by_name(requested_name)?;
    let data = Arc::from(session.ld_file.read_channel_data(channel)?);
    session.channel_cache.insert(
        normalized_requested,
        OverlayChannelCacheEntry {
            data: Arc::clone(&data),
            freq: channel.freq,
        },
    );
    Some((data, channel.freq))
}

fn default_overlay_lap_index(laps: &[Lap]) -> Option<usize> {
    laps.iter()
        .enumerate()
        .filter(|(_, lap)| lap.name.starts_with("Lap "))
        .min_by(|(_, a), (_, b)| a.duration().total_cmp(&b.duration()))
        .map(|(idx, _)| idx)
        .or_else(|| (!laps.is_empty()).then_some(0))
}

fn derive_distance_axis_cache(shared: &SharedState) -> (Option<DistanceAxisCache>, bool) {
    let (distance_cache, waiting_on_distance) = find_distance_channel(shared);
    if let Some(cache) = distance_cache {
        return (Some(cache), false);
    }

    let (speed_series, waiting_on_speed) = find_speed_channel(shared);
    let Some(speed_series) = speed_series else {
        return (None, waiting_on_distance || waiting_on_speed);
    };

    (
        integrate_speed_series(&speed_series.0, speed_series.1, &speed_series.2).map(|data| {
            DistanceAxisCache {
                data: Arc::from(data),
                freq: speed_series.1,
            }
        }),
        waiting_on_distance || waiting_on_speed,
    )
}

struct OwnedDistanceAxisCache {
    data: Vec<f64>,
    freq: u16,
}

fn derive_distance_axis_cache_for_ld(ld: &LdFile) -> Option<OwnedDistanceAxisCache> {
    if let Some(cache) = find_distance_channel_in_ld(ld) {
        return Some(cache);
    }

    let (data, freq, unit) = find_speed_channel_in_ld(ld)?;
    integrate_speed_series(&data, freq, &unit).map(|integrated| OwnedDistanceAxisCache {
        data: integrated,
        freq,
    })
}

fn find_distance_channel(shared: &SharedState) -> (Option<DistanceAxisCache>, bool) {
    let distance_names = ["distance", "lap distance", "distance driven"];
    let mut waiting_on_sources = false;

    for mc in &shared.math_channels {
        if normalized_name(&mc.name).as_str().eq_any(&distance_names)
            && let Some(data) = &mc.data
            && mc.freq > 0
            && is_monotonic_non_decreasing(data)
        {
            return (
                Some(DistanceAxisCache {
                    data: Arc::clone(data),
                    freq: mc.freq,
                }),
                false,
            );
        } else if normalized_name(&mc.name).as_str().eq_any(&distance_names) && mc.error.is_none() {
            waiting_on_sources = true;
        }
    }

    let Some(ld) = shared.ld_file.as_ref() else {
        return (None, waiting_on_sources);
    };
    for ch in &ld.channels {
        if normalized_name(&ch.name).as_str().eq_any(&distance_names) {
            if let Some(decoded) = shared.decoded_physical_channel_if_ready(ch.index) {
                if is_monotonic_non_decreasing(&decoded.data) {
                    return (
                        Some(DistanceAxisCache {
                            data: Arc::clone(&decoded.data),
                            freq: ch.freq,
                        }),
                        false,
                    );
                }
            } else {
                shared.request_physical_channel_decode(ch.index);
                waiting_on_sources = true;
            }
        }
    }

    (None, waiting_on_sources)
}

fn find_distance_channel_in_ld(ld: &LdFile) -> Option<OwnedDistanceAxisCache> {
    let distance_names = ["distance", "lap distance", "distance driven"];
    for channel in &ld.channels {
        if normalized_name(&channel.name)
            .as_str()
            .eq_any(&distance_names)
            && let Some(data) = ld.read_channel_data(channel)
            && is_monotonic_non_decreasing(&data)
        {
            return Some(OwnedDistanceAxisCache {
                data,
                freq: channel.freq,
            });
        }
    }
    None
}

type SpeedChannelLookup = Option<(Arc<[f64]>, u16, String)>;

fn find_speed_channel(shared: &SharedState) -> (SpeedChannelLookup, bool) {
    let speed_names = [
        "gps speed",
        "vehicle speed",
        "corr speed",
        "ground speed",
        "speed",
    ];
    let mut waiting_on_sources = false;

    for mc in &shared.math_channels {
        if normalized_name(&mc.name).as_str().eq_any(&speed_names)
            && let Some(data) = &mc.data
            && mc.freq > 0
        {
            return (Some((Arc::clone(data), mc.freq, mc.unit.clone())), false);
        } else if normalized_name(&mc.name).as_str().eq_any(&speed_names) && mc.error.is_none() {
            waiting_on_sources = true;
        }
    }

    let Some(ld) = shared.ld_file.as_ref() else {
        return (None, waiting_on_sources);
    };
    for ch in &ld.channels {
        if normalized_name(&ch.name).as_str().eq_any(&speed_names) {
            if let Some(decoded) = shared.decoded_physical_channel_if_ready(ch.index) {
                return (
                    Some((Arc::clone(&decoded.data), ch.freq, ch.unit.clone())),
                    false,
                );
            }
            shared.request_physical_channel_decode(ch.index);
            waiting_on_sources = true;
        }
    }

    (None, waiting_on_sources)
}

fn find_speed_channel_in_ld(ld: &LdFile) -> Option<(Vec<f64>, u16, String)> {
    let speed_names = [
        "gps speed",
        "vehicle speed",
        "corr speed",
        "ground speed",
        "speed",
    ];
    for channel in &ld.channels {
        if normalized_name(&channel.name).as_str().eq_any(&speed_names)
            && let Some(data) = ld.read_channel_data(channel)
        {
            return Some((data, channel.freq, channel.unit.clone()));
        }
    }
    None
}

fn integrate_speed_series(data: &[f64], freq: u16, unit: &str) -> Option<Vec<f64>> {
    if freq == 0 {
        return None;
    }

    let to_mps = speed_unit_to_mps(unit)?;
    if data.is_empty() {
        return Some(Vec::new());
    }

    let dt = 1.0 / freq as f64;
    let mut result = Vec::with_capacity(data.len());
    let mut sum = 0.0;
    result.push(0.0);

    for i in 0..data.len() - 1 {
        let v0 = data[i] * to_mps;
        let v1 = data[i + 1] * to_mps;
        sum += (v0 + v1) * 0.5 * dt;
        result.push(sum.max(*result.last().unwrap_or(&0.0)));
    }

    Some(result)
}

fn speed_unit_to_mps(unit: &str) -> Option<f64> {
    let normalized = unit.to_ascii_lowercase().replace(' ', "");
    if normalized.contains("km/h") || normalized.contains("kmh") || normalized.contains("kph") {
        Some(1.0 / 3.6)
    } else if normalized.contains("mph") {
        Some(0.44704)
    } else if normalized.contains("m/s") || normalized.contains("mps") {
        Some(1.0)
    } else {
        None
    }
}

fn normalized_name(name: &str) -> String {
    i3rs_core::normalize_channel_name(name)
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn time_from_monotonic_axis(data: &[f64], freq: u16, axis_value: f64) -> Option<f64> {
    if freq == 0 || data.is_empty() || !is_monotonic_non_decreasing(data) {
        return None;
    }

    let first = data.first().copied()?;
    let last = data.last().copied()?;
    let clamped = axis_value.clamp(first.min(last), first.max(last));
    let idx = data.partition_point(|v| *v < clamped);

    if idx == 0 {
        return Some(0.0);
    }
    if idx >= data.len() {
        return Some((data.len().saturating_sub(1)) as f64 / freq as f64);
    }

    let x0 = data[idx - 1];
    let x1 = data[idx];
    let t0 = (idx - 1) as f64 / freq as f64;
    let t1 = idx as f64 / freq as f64;

    if (x1 - x0).abs() < f64::EPSILON {
        Some(t1)
    } else {
        Some(t0 + (clamped - x0) / (x1 - x0) * (t1 - t0))
    }
}

fn is_monotonic_non_decreasing(data: &[f64]) -> bool {
    const EPSILON: f64 = 1e-6;

    data.windows(2).all(|pair| {
        let prev = pair[0];
        let next = pair[1];
        prev.is_finite() && next.is_finite() && next + EPSILON >= prev
    })
}

trait NormalizedNameExt {
    fn eq_any(&self, values: &[&str]) -> bool;
}

impl NormalizedNameExt for str {
    fn eq_any(&self, values: &[&str]) -> bool {
        values.contains(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveGraphXAxis, GraphAxisFingerprint, GraphPanel, GraphRenderFingerprint,
        LapRenderFingerprint, OverlaySource, PlotXRange, SampleRange,
        display_transform_fingerprint, integrate_speed_series, is_monotonic_non_decreasing,
        native_plot_points_for_range, overlay_axis_offset, time_from_monotonic_axis,
    };
    use crate::state::{ChannelId, PlottedChannel, YAxis};
    use i3rs_core::Lap;
    use std::sync::Arc;

    #[test]
    fn integrates_kmh_speed_into_distance() {
        let speed = vec![36.0, 36.0, 36.0];
        let distance = integrate_speed_series(&speed, 1, "km/h").unwrap();
        assert_eq!(distance, vec![0.0, 10.0, 20.0]);
    }

    #[test]
    fn maps_distance_back_to_time() {
        let distance = vec![0.0, 10.0, 20.0, 30.0];
        let time = time_from_monotonic_axis(&distance, 2, 15.0).unwrap();
        assert!((time - 0.75).abs() < 1e-6);
    }

    #[test]
    fn native_plot_points_preserve_short_enum_events() {
        let points = native_plot_points_for_range(
            &[0.0, 1.0, 1.0, 0.0],
            10,
            SampleRange { start: 0, end: 4 },
            &ActiveGraphXAxis::Time,
            PlotXRange { min: 0.0, max: 1.0 },
            1.0,
            0.0,
        );

        assert_eq!(points.len(), 4);
        assert!(points.iter().any(|point| point.y == 1.0));
    }

    #[test]
    fn rejects_non_monotonic_distance_axis() {
        let distance = vec![0.0, 10.0, 5.0, 15.0];
        assert!(!is_monotonic_non_decreasing(&distance));
        assert!(time_from_monotonic_axis(&distance, 1, 7.0).is_none());
    }

    #[test]
    fn converts_overlay_offset_into_distance_units() {
        let lap = Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 5.0,
            end_time: 8.0,
        };
        let axis = ActiveGraphXAxis::Distance {
            data: Arc::from(vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0]),
            freq: 1,
        };

        assert!((overlay_axis_offset(&axis, &lap, 8.0, 1.0) - 10.0).abs() < 1e-6);
        assert!((overlay_axis_offset(&axis, &lap, 8.0, -1.0) + 10.0).abs() < 1e-6);
    }

    fn sample_channel(data: Arc<[f64]>) -> PlottedChannel {
        PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data,
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            cached_min: 0.0,
            cached_max: 1.0,
            cached_avg: 0.5,
        }
    }

    #[test]
    fn graph_render_fingerprint_invalidates_for_zoom_transform_and_data_replacement() {
        let channel = sample_channel(Arc::from(vec![0.0, 1.0, 2.0]));
        let base = GraphRenderFingerprint {
            data_ptr: channel.data.as_ptr() as usize,
            data_len: channel.data.len(),
            x_bounds: (0.0f64.to_bits(), 5.0f64.to_bits()),
            axis: GraphAxisFingerprint::Time,
            transform: display_transform_fingerprint(&channel),
            target_width: 320,
            preserve_native_samples: false,
        };

        let zoom_changed = GraphRenderFingerprint {
            x_bounds: (1.0f64.to_bits(), 6.0f64.to_bits()),
            ..base.clone()
        };
        assert!(base != zoom_changed);

        let mut transformed_channel = sample_channel(Arc::from(vec![0.0, 1.0, 2.0]));
        transformed_channel.display_scale = 2.0;
        let transform_changed = GraphRenderFingerprint {
            transform: display_transform_fingerprint(&transformed_channel),
            ..base.clone()
        };
        assert!(base != transform_changed);

        let replaced_channel = sample_channel(Arc::from(vec![0.0, 1.0, 2.0]));
        let replacement_changed = GraphRenderFingerprint {
            data_ptr: replaced_channel.data.as_ptr() as usize,
            ..base
        };
        assert!(transform_changed != replacement_changed);
    }

    #[test]
    fn lap_render_fingerprint_invalidates_for_zoom_and_overlay_offset() {
        let base = LapRenderFingerprint {
            data_ptr: 11,
            data_len: 64,
            lap_bounds: (1.0f64.to_bits(), 2.0f64.to_bits()),
            x_bounds: (0.0f64.to_bits(), 10.0f64.to_bits()),
            axis: GraphAxisFingerprint::Time,
            origin_axis_bits: 0.0f64.to_bits(),
            x_scale_bits: 1.0f64.to_bits(),
            x_offset_bits: 0.0f64.to_bits(),
            transform: super::DisplayTransformFingerprint {
                scale_bits: 1.0f64.to_bits(),
                offset_bits: 0.0f64.to_bits(),
                unit: None,
            },
            target_width: 400,
            preserve_native_samples: false,
        };

        let zoom_changed = LapRenderFingerprint {
            x_bounds: (2.0f64.to_bits(), 12.0f64.to_bits()),
            ..base.clone()
        };
        let offset_changed = LapRenderFingerprint {
            x_offset_bits: 5.0f64.to_bits(),
            ..base.clone()
        };

        assert!(base != zoom_changed);
        assert!(base != offset_changed);
    }

    #[test]
    fn reset_for_new_session_clears_all_render_caches() {
        let mut panel = GraphPanel::new(7, "Graph");
        panel.render_cache.insert(
            ChannelId::Physical(0),
            super::GraphRenderCacheEntry {
                fingerprint: GraphRenderFingerprint {
                    data_ptr: 1,
                    data_len: 2,
                    x_bounds: (0, 1),
                    axis: GraphAxisFingerprint::Time,
                    transform: super::DisplayTransformFingerprint {
                        scale_bits: 1.0f64.to_bits(),
                        offset_bits: 0.0f64.to_bits(),
                        unit: None,
                    },
                    target_width: 100,
                    preserve_native_samples: false,
                },
                plot_points: Vec::new(),
            },
        );
        panel.lap_render_cache.insert(
            super::LapRenderCacheKey::Overlay {
                channel_id: ChannelId::Physical(0),
                overlay_idx: 0,
            },
            super::LapRenderCacheEntry {
                fingerprint: LapRenderFingerprint {
                    data_ptr: 1,
                    data_len: 2,
                    lap_bounds: (0, 1),
                    x_bounds: (0, 1),
                    axis: GraphAxisFingerprint::Time,
                    origin_axis_bits: 0,
                    x_scale_bits: 0,
                    x_offset_bits: 0,
                    transform: super::DisplayTransformFingerprint {
                        scale_bits: 1,
                        offset_bits: 0,
                        unit: None,
                    },
                    target_width: 10,
                    preserve_native_samples: false,
                },
                plot_points: Vec::new(),
            },
        );
        panel.lap_overlays.push(super::LapOverlay {
            source: OverlaySource::MainSession,
            lap_idx: 0,
            manual_offset: 0.0,
            stretch_to_reference: false,
        });

        panel.reset_for_new_main_session();

        assert!(panel.render_cache.is_empty());
        assert!(panel.lap_render_cache.is_empty());
        assert!(panel.lap_overlays.is_empty());
    }
}

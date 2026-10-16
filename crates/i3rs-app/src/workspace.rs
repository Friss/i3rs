//! Workspace save/load: serializes panel layout and channel configuration.

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use i3rs_core::Sector;

use crate::panels::PanelTab;
use crate::panels::fft::FftPanel;
use crate::panels::gauge::{GaugePanel, GaugeStyle};
use crate::panels::graph::{GraphPanel, OverlaySource};
use crate::panels::histogram::HistogramPanel;
use crate::panels::mixture_map::MixtureMapPanel;
use crate::panels::scatter::ScatterPanel;
use crate::panels::track_map::TrackMapPanel;
use crate::panels::utils::create_plotted_channel;
use crate::state::{CHANNEL_COLORS, ChannelId, GraphMode, GraphXAxis, SharedState};

// ---------------------------------------------------------------------------
// Serializable workspace types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub comparisons: Option<ComparisonConfig>,
    pub worksheets: Vec<WorksheetConfig>,
    pub active_worksheet: usize,
    pub last_file_path: Option<String>,
    #[serde(default)]
    pub math_channels: Vec<MathChannelConfig>,
    #[serde(default)]
    pub channel_aliases: Vec<ChannelAliasConfig>,
    #[serde(default)]
    pub channel_settings: i3rs_core::channels::ChannelSettings,
    #[serde(default)]
    pub recording_analyses:
        std::collections::BTreeMap<String, i3rs_core::ranges::RecordingAnalysis>,
    #[serde(default)]
    pub sectors: Vec<Sector>,
    #[serde(default)]
    pub track_definition: Option<i3rs_core::TrackDefinition>,
    #[serde(default)]
    pub reference_lap: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChannelAliasConfig {
    pub alias: String,
    pub target: String,
}

#[derive(Serialize, Deserialize)]
pub struct WorksheetConfig {
    #[serde(default)]
    pub dock: Option<DockState<usize>>,
    #[serde(default)]
    pub window: WorksheetWindow,
    pub name: String,
    pub panels: Vec<PanelConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WorksheetWindow {
    pub detached: bool,
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
}
impl Default for WorksheetWindow {
    fn default() -> Self {
        Self {
            detached: false,
            size: [1000., 700.],
            position: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum PanelConfig {
    Calculations(CalculationsConfig),
    Graph(GraphPanelConfig),
    TrackMap(TrackMapPanelConfig),
    ChannelBrowser,
    CursorReadout,
    Report(ReportPanelConfig),
    Histogram(HistogramPanelConfig),
    Scatter(ScatterPanelConfig),
    Fft(FftPanelConfig),
    Gauge(GaugePanelConfig),
    MixtureMap(MixtureMapPanelConfig),
}

#[derive(Serialize, Deserialize)]
pub struct CalculationsConfig {
    #[serde(default = "default_velocity_bins")]
    pub suspension_bins: usize,
    #[serde(default)]
    pub suspension_output: crate::panels::calculations::DistributionOutput,
    #[serde(default)]
    pub suspension_line: bool,
    pub id: u64,
    pub title: String,
    pub channels: Vec<AnalysisChannelConfig>,
    pub filter: crate::panels::distribution::ComponentFilter,
    pub mode: crate::panels::calculations::CalculationMode,
    pub interval_seconds: f64,
    pub suspension: i3rs_core::distribution::SuspensionSettings,
}

#[derive(Serialize, Deserialize)]
pub struct GraphPanelConfig {
    #[serde(default)]
    pub channels: Vec<GraphChannelSlot>,
    #[serde(default)]
    pub view: crate::comparison::GraphView,
    #[serde(default)]
    pub unresolved_channels: Vec<GraphChannelSlot>,
    #[serde(default)]
    pub full_resolution: bool,
    pub id: u64,
    pub title: String,
    pub channel_names: Vec<String>,
    pub colors: Vec<[u8; 3]>,
    #[serde(default)]
    pub tile_groups: Vec<usize>,
    pub graph_mode: String, // "Tiled" or "Overlay"
    #[serde(default = "default_graph_x_axis_mode")]
    pub x_axis_mode: String, // "Time" or "Distance"
    #[serde(default)]
    pub reference_lap: Option<usize>,
    #[serde(default)]
    pub overlays: Vec<GraphOverlayConfig>,
    #[serde(default)]
    pub embedded_gauges: Vec<GraphGaugeConfig>,
    #[serde(default = "default_graph_embedded_gauge_height")]
    pub embedded_gauge_height: f32,
    /// Whether each channel is a math channel (true) or physical (false).
    #[serde(default)]
    pub is_math: Vec<bool>,
    #[serde(default)]
    pub display_transforms: Vec<GraphDisplayTransformConfig>,
    #[serde(default)]
    pub tile_heights: Vec<f32>,
}

fn default_graph_x_axis_mode() -> String {
    "Time".into()
}

fn default_graph_embedded_gauge_height() -> f32 {
    176.0
}

pub(crate) fn gauge_style_to_string(style: GaugeStyle) -> &'static str {
    match style {
        GaugeStyle::Analog => "Analog",
        GaugeStyle::Bar => "Bar",
        GaugeStyle::Digital => "Digital",
        GaugeStyle::SteeringWheel => "SteeringWheel",
    }
}

pub(crate) fn gauge_style_from_string(style: &str) -> GaugeStyle {
    match style {
        "Bar" => GaugeStyle::Bar,
        "Digital" => GaugeStyle::Digital,
        "SteeringWheel" => GaugeStyle::SteeringWheel,
        _ => GaugeStyle::Analog,
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct ComparisonConfig {
    pub main_lap: Option<usize>,
    pub overlays: Vec<GraphOverlayConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GraphChannelSlot {
    pub channel: AnalysisChannelConfig,
    pub tile_group: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GraphOverlayConfig {
    #[serde(default)]
    pub gps_align: bool,
    #[serde(default = "default_track_annotations")]
    pub visible: bool,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    pub file_path: Option<String>,
    pub lap_index: usize,
    pub manual_offset: f64,
    pub stretch_to_reference: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GraphGaugeConfig {
    #[serde(default)]
    pub channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub settings: crate::panels::gauge::GaugeSettings,
    #[serde(default)]
    pub transform: Option<GraphDisplayTransformConfig>,
    pub channel_name: String,
    pub is_math: bool,
    pub style: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GraphDisplayTransformConfig {
    pub scale: f64,
    pub offset: f64,
    pub unit: Option<String>,
    /// Fixed display range `(min, max)` in display units. `None` = auto-scale.
    #[serde(default)]
    pub scale_manual: Option<(f64, f64)>,
}

impl Default for GraphDisplayTransformConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
            unit: None,
            scale_manual: None,
        }
    }
}

impl GraphDisplayTransformConfig {
    fn of(pc: &crate::state::PlottedChannel) -> Self {
        let (scale, offset, unit) = pc.recording_display_base.clone().unwrap_or((
            pc.display_scale,
            pc.display_offset,
            pc.display_unit.clone(),
        ));
        let scale_manual = (pc.scale_mode == crate::state::ScaleMode::Manual).then(|| {
            let convert = |value| {
                if pc.recording_display_base.is_none() {
                    value
                } else if pc.display_scale.abs() > f64::EPSILON {
                    (value - pc.display_offset) / pc.display_scale * scale + offset
                } else {
                    value
                }
            };
            let a = convert(pc.manual_min);
            let b = convert(pc.manual_max);
            (a.min(b), a.max(b))
        });
        Self {
            scale,
            offset,
            unit,
            scale_manual,
        }
    }

    fn apply_to(&self, pc: &mut crate::state::PlottedChannel) {
        pc.recording_display_base = None;
        pc.display_scale = self.scale;
        pc.display_offset = self.offset;
        pc.display_unit = self.unit.clone();
        apply_saved_scale_limits(pc, self.scale_manual);
    }
}

fn apply_saved_scale_limits(
    channel: &mut crate::state::PlottedChannel,
    scale_manual: Option<(f64, f64)>,
) {
    match scale_manual {
        Some((min, max)) => {
            channel.scale_mode = crate::state::ScaleMode::Manual;
            channel.manual_min = min;
            channel.manual_max = max;
        }
        None => {
            channel.scale_mode = crate::state::ScaleMode::Auto;
            channel.manual_min = channel.cached_min;
            channel.manual_max = channel.cached_max;
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ReportPanelConfig {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub settings: crate::panels::report::ReportSettings,
    #[serde(default)]
    pub channels: Vec<AnalysisChannelConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AnalysisChannelConfig {
    #[serde(default)]
    pub source_unit: Option<String>,
    /// Legacy slots without explicit transforms inherit current channel defaults.
    #[serde(default)]
    pub inherit_display: bool,
    pub name: String,
    pub is_math: bool,
    pub color: [u8; 3],
    pub transform: GraphDisplayTransformConfig,
}

pub(crate) fn save_analysis_channel(
    pc: &crate::state::PlottedChannel,
    shared: &SharedState,
) -> Option<AnalysisChannelConfig> {
    let (name, is_math) = saved_channel_identity(pc, shared)?;
    Some(AnalysisChannelConfig {
        source_unit: if matches!(pc.channel_id, ChannelId::Unresolved(_)) {
            pc.portable_source_unit.clone()
        } else {
            Some(crate::panels::utils::resolve_channel_meta(pc.channel_id, shared).1)
        },
        inherit_display: false,
        name,
        is_math,
        color: [pc.color.r(), pc.color.g(), pc.color.b()],
        transform: GraphDisplayTransformConfig::of(pc),
    })
}

pub(crate) fn load_analysis_channel(
    c: &AnalysisChannelConfig,
    shared: &SharedState,
) -> Option<crate::state::PlottedChannel> {
    let mut pc = resolve_saved_plotted_channel(
        shared,
        &c.name,
        c.is_math,
        eframe::egui::Color32::from_rgb(c.color[0], c.color[1], c.color[2]),
        0,
    )?;
    if !c.inherit_display {
        c.transform.apply_to(&mut pc);
    }
    pc.portable_source_unit = c.source_unit.clone();
    crate::panels::utils::reconcile_source_unit(&mut pc, shared);
    Some(pc)
}

#[derive(Serialize, Deserialize)]
pub struct TrackMapPanelConfig {
    #[serde(default)]
    pub window: WorksheetWindow,
    pub id: u64,
    pub title: String,
    pub color_channel_name: Option<String>,
    #[serde(default)]
    pub section_times_only: bool,
    #[serde(default = "default_track_annotations")]
    pub show_annotations: bool,
    #[serde(default)]
    pub annotation_settings: i3rs_core::annotations::AnnotationSettings,
}

fn default_track_annotations() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub struct HistogramPanelConfig {
    #[serde(default)]
    pub bin_settings: crate::panels::histogram::HistogramBinSettings,
    #[serde(default)]
    pub band_presentation: crate::panels::distribution::BandPresentation,
    #[serde(default)]
    pub channels: Vec<AnalysisChannelConfig>,
    #[serde(default)]
    pub seconds: bool,
    #[serde(default)]
    pub filter: crate::panels::distribution::ComponentFilter,
    pub id: u64,
    pub title: String,
    pub bin_count: usize,
    pub per_lap: bool,
    #[serde(default)]
    pub percentage: bool,
    #[serde(default)]
    pub color_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub color_bands: crate::panels::distribution::ColorBands,
    #[serde(default)]
    pub lock_x_range: bool,
    #[serde(default)]
    pub x_min: f64,
    #[serde(default)]
    pub x_max: f64,
    #[serde(default)]
    pub lock_y_range: bool,
    #[serde(default)]
    pub y_min: f64,
    #[serde(default = "default_histogram_y_headroom_pct")]
    pub y_headroom_pct: f64,
    #[serde(default)]
    pub channel_names: Vec<String>,
    #[serde(default)]
    pub is_math: Vec<bool>,
    #[serde(default)]
    pub colors: Vec<[u8; 3]>,
}

fn default_histogram_y_headroom_pct() -> f64 {
    10.0
}

#[derive(Serialize, Deserialize)]
pub struct ScatterPanelConfig {
    #[serde(default)]
    pub show_interval_averages: bool,
    #[serde(default = "default_interval_width")]
    pub interval_width: f64,
    #[serde(default)]
    pub interval_origin: f64,
    #[serde(default)]
    pub density: bool,
    #[serde(default)]
    pub band_presentation: crate::panels::distribution::BandPresentation,
    #[serde(default)]
    pub x_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub y_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub color_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub color_bands: crate::panels::distribution::ColorBands,
    #[serde(default)]
    pub show_regression: bool,
    #[serde(default)]
    pub filter: crate::panels::distribution::ComponentFilter,
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub x_channel_name: Option<String>,
    #[serde(default)]
    pub y_channel_name: Option<String>,
    #[serde(default)]
    pub x_is_math: bool,
    #[serde(default)]
    pub y_is_math: bool,
    #[serde(default)]
    pub x_color: Option<[u8; 3]>,
    #[serde(default)]
    pub y_color: Option<[u8; 3]>,
    #[serde(default = "default_scatter_point_size")]
    pub point_size: f32,
    #[serde(default)]
    pub bounds_padding_frac: f64,
    #[serde(default)]
    pub lock_bounds: bool,
}

fn default_scatter_point_size() -> f32 {
    1.5
}

#[derive(Serialize, Deserialize)]
pub struct FftPanelConfig {
    #[serde(default)]
    pub channels: Vec<AnalysisChannelConfig>,
    pub id: u64,
    pub title: String,
    pub log_scale: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedGauge {
    pub channel: AnalysisChannelConfig,
    pub style: String,
    #[serde(default)]
    pub settings: crate::panels::gauge::GaugeSettings,
}

#[derive(Serialize, Deserialize)]
pub struct GaugePanelConfig {
    #[serde(default)]
    pub gauges: Vec<SavedGauge>,
    pub id: u64,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
pub struct MixtureMapPanelConfig {
    #[serde(default)]
    pub show_trends: bool,
    #[serde(default = "default_interval_width")]
    pub trend_width: f64,
    #[serde(default)]
    pub band_presentation: crate::panels::distribution::BandPresentation,
    #[serde(default)]
    pub x_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub y_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub value_channel: Option<AnalysisChannelConfig>,
    #[serde(default)]
    pub gear_filter: i3rs_core::distribution::GearFilter,
    #[serde(default)]
    pub filter: crate::panels::distribution::ComponentFilter,
    pub id: u64,
    pub title: String,
    pub bins: usize,
    #[serde(default)]
    pub scatter_mode: bool,
    #[serde(default)]
    pub sensor_delay: f64,
    #[serde(default)]
    pub minimum_y: Option<f64>,
    #[serde(default)]
    pub color_bands: crate::panels::distribution::ColorBands,
    #[serde(default)]
    pub x_channel_name: Option<String>,
    #[serde(default)]
    pub y_channel_name: Option<String>,
    #[serde(default)]
    pub value_channel_name: Option<String>,
    #[serde(default)]
    pub x_is_math: bool,
    #[serde(default)]
    pub y_is_math: bool,
    #[serde(default)]
    pub value_is_math: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MathChannelConfig {
    #[serde(default)]
    pub options: i3rs_core::channels::MathOptions,
    pub name: String,
    pub expression: String,
    pub unit: String,
    pub dec_places: i16,
}

impl MathChannelConfig {
    pub fn create_definition(&self, shared: &mut SharedState) -> crate::state::MathChannelDef {
        let mut definition = shared.create_math_channel_def(
            self.name.clone(),
            self.expression.clone(),
            self.unit.clone(),
            self.dec_places,
        );
        definition.options = self.options.clone();
        definition
    }
}

fn channel_name_and_kind(channel_id: ChannelId, shared: &SharedState) -> Option<(String, bool)> {
    match channel_id {
        ChannelId::Analysis(i) | ChannelId::Unresolved(i) => {
            shared.unresolved_references.borrow().get(i).cloned()
        }
        ChannelId::Physical(idx) => {
            Some((shared.ld_file.as_ref()?.channels[idx].name.clone(), false))
        }
        ChannelId::Math(idx) => {
            let mc = shared.math_channels.get(idx)?;
            Some((format!("{}::{}", mc.options.source, mc.name), true))
        }
    }
}

fn saved_channel_identity(
    pc: &crate::state::PlottedChannel,
    shared: &SharedState,
) -> Option<(String, bool)> {
    pc.requested_reference
        .clone()
        .or_else(|| channel_name_and_kind(pc.channel_id, shared))
}

fn resolve_saved_plotted_channel(
    shared: &SharedState,
    channel_name: &str,
    is_math: bool,
    color: eframe::egui::Color32,
    tile_group: usize,
) -> Option<crate::state::PlottedChannel> {
    let id = shared
        .resolve_slot(channel_name, is_math)
        .unwrap_or_else(|| shared.unresolved_channel(channel_name, is_math));
    let mut plotted = create_plotted_channel(id, shared, tile_group).or_else(|| {
        create_plotted_channel(
            shared.unresolved_channel(channel_name, is_math),
            shared,
            tile_group,
        )
    })?;
    plotted.requested_reference = Some((channel_name.into(), is_math));
    plotted.color = color;
    plotted.tile_group = tile_group;
    Some(plotted)
}

// ---------------------------------------------------------------------------
// Conversion: App state → WorkspaceFile
// ---------------------------------------------------------------------------

fn finite_dock_layout(
    dock: &DockState<PanelTab>,
    locations: &std::collections::HashMap<usize, usize>,
) -> DockState<usize> {
    let mut topology = dock.map_tabs(|tab| locations[&(tab as *const PanelTab as usize)]);
    for (_, node) in topology.iter_all_nodes_mut() {
        node.set_rect(egui::Rect::ZERO);
        if let Some(leaf) = node.get_leaf_mut() {
            leaf.viewport = egui::Rect::ZERO;
            if !leaf.scroll.is_finite() {
                leaf.scroll = 0.;
            }
        }
    }
    topology
}

pub fn save_workspace(
    worksheets: &[(String, &DockState<PanelTab>)],
    active_worksheet: usize,
    shared: &SharedState,
) -> WorkspaceFile {
    let ws_configs: Vec<WorksheetConfig> = worksheets
        .iter()
        .map(|(name, dock)| {
            let mut panels = Vec::new();
            let mut locations = std::collections::HashMap::new();
            for (_path, tab) in dock.iter_all_tabs() {
                locations.insert(tab as *const PanelTab as usize, panels.len());
                let config = match tab {
                    PanelTab::Calculations(p) => PanelConfig::Calculations(CalculationsConfig {
                        suspension_bins: p.suspension_bins,
                        suspension_output: p.suspension_output,
                        suspension_line: p.suspension_line,
                        id: p.id,
                        title: p.title.clone(),
                        channels: p
                            .channels
                            .iter()
                            .filter_map(|c| save_analysis_channel(c, shared))
                            .collect(),
                        filter: p.filter.clone(),
                        mode: p.mode,
                        interval_seconds: p.interval_seconds,
                        suspension: p.suspension,
                    }),
                    PanelTab::Graph(g) => {
                        let mut channel_names = Vec::new();
                        let mut is_math = Vec::new();
                        for pc in &g.plotted_channels {
                            if let Some((name, math)) = saved_channel_identity(pc, shared) {
                                channel_names.push(name);
                                is_math.push(math);
                            }
                        }
                        let colors: Vec<[u8; 3]> = g
                            .plotted_channels
                            .iter()
                            .map(|pc| {
                                let c = pc.color;
                                [c.r(), c.g(), c.b()]
                            })
                            .collect();
                        let tile_groups: Vec<usize> =
                            g.plotted_channels.iter().map(|pc| pc.tile_group).collect();
                        let display_transforms: Vec<GraphDisplayTransformConfig> = g
                            .plotted_channels
                            .iter()
                            .map(GraphDisplayTransformConfig::of)
                            .collect();
                        let overlays: Vec<GraphOverlayConfig> = g
                            .lap_overlays
                            .iter()
                            .map(|overlay| {
                                let file_path = match overlay.source {
                                    OverlaySource::MainSession => None,
                                    OverlaySource::External(session_idx) => g
                                        .overlay_sessions
                                        .get(session_idx)
                                        .map(|session| session.path.to_string_lossy().to_string()),
                                };
                                GraphOverlayConfig {
                                    gps_align: overlay.gps_align,
                                    visible: overlay.visible,
                                    color: overlay.color.map(|c| [c.r(), c.g(), c.b()]),
                                    file_path,
                                    lap_index: overlay.lap_idx,
                                    manual_offset: overlay.manual_offset,
                                    stretch_to_reference: overlay.stretch_to_reference,
                                }
                            })
                            .collect();
                        let embedded_gauges: Vec<GraphGaugeConfig> = g
                            .embedded_gauges
                            .iter()
                            .filter_map(|gauge| {
                                let (channel_name, is_math) =
                                    saved_channel_identity(&gauge.channel, shared)?;
                                Some(GraphGaugeConfig {
                                    channel: save_analysis_channel(&gauge.channel, shared),
                                    settings: gauge.settings,
                                    transform: Some(GraphDisplayTransformConfig::of(
                                        &gauge.channel,
                                    )),
                                    channel_name,
                                    is_math,
                                    style: gauge_style_to_string(gauge.style).into(),
                                })
                            })
                            .chain(g.unresolved_gauges.iter().map(|slot| {
                                GraphGaugeConfig {
                                    channel: Some(slot.channel.clone()),
                                    channel_name: slot.channel.name.clone(),
                                    is_math: slot.channel.is_math,
                                    style: slot.style.clone(),
                                    settings: slot.settings,
                                    transform: (!slot.channel.inherit_display)
                                        .then(|| slot.channel.transform.clone()),
                                }
                            }))
                            .collect();
                        PanelConfig::Graph(GraphPanelConfig {
                            channels: g
                                .plotted_channels
                                .iter()
                                .filter_map(|pc| {
                                    save_analysis_channel(pc, shared).map(|channel| {
                                        GraphChannelSlot {
                                            channel,
                                            tile_group: pc.tile_group,
                                        }
                                    })
                                })
                                .chain(g.unresolved_channels.iter().cloned())
                                .collect(),
                            view: g.view.clone(),
                            unresolved_channels: g.unresolved_channels.clone(),
                            full_resolution: g.full_resolution,
                            id: g.id,
                            title: g.title.clone(),
                            channel_names,
                            colors,
                            tile_groups,
                            graph_mode: match g.graph_mode {
                                GraphMode::Tiled => "Tiled".into(),
                                GraphMode::Overlay => "Overlay".into(),
                            },
                            x_axis_mode: match g.x_axis_mode {
                                GraphXAxis::Time => "Time".into(),
                                GraphXAxis::Distance => "Distance".into(),
                            },
                            reference_lap: g.reference_lap,
                            overlays,
                            embedded_gauges,
                            embedded_gauge_height: g.embedded_gauge_height,
                            is_math,
                            display_transforms,
                            tile_heights: g.tile_heights.clone(),
                        })
                    }
                    PanelTab::TrackMap(t) => {
                        let color_channel_name = t.requested_color.clone().or_else(|| {
                            t.color_channel_idx.and_then(|idx| {
                                shared
                                    .ld_file
                                    .as_ref()
                                    .and_then(|ld| ld.channels.get(idx).map(|ch| ch.name.clone()))
                            })
                        });
                        PanelConfig::TrackMap(TrackMapPanelConfig {
                            window: WorksheetWindow {
                                detached: t.is_popped_out,
                                ..t.window.clone()
                            },
                            id: t.id,
                            title: t.title.clone(),
                            color_channel_name,
                            section_times_only: t.section_times_only,
                            show_annotations: t.show_annotations,
                            annotation_settings: t.annotation_settings.clone(),
                        })
                    }
                    PanelTab::ChannelBrowser => PanelConfig::ChannelBrowser,
                    PanelTab::CursorReadout => PanelConfig::CursorReadout,
                    PanelTab::Report(r) => PanelConfig::Report(ReportPanelConfig {
                        id: r.id,
                        title: r.title.clone(),
                        settings: r.settings.clone(),
                        channels: r
                            .channels
                            .iter()
                            .filter_map(|pc| save_analysis_channel(pc, shared))
                            .collect(),
                    }),
                    PanelTab::Histogram(h) => PanelConfig::Histogram(HistogramPanelConfig {
                        bin_settings: h.bin_settings,
                        band_presentation: h.band_presentation.clone(),
                        channels: h
                            .channels
                            .iter()
                            .filter_map(|p| save_analysis_channel(p, shared))
                            .collect(),
                        filter: h.filter.clone(),
                        seconds: h.seconds,
                        id: h.id,
                        title: h.title.clone(),
                        bin_count: h.bin_count,
                        per_lap: h.per_lap,
                        percentage: h.percentage,
                        color_channel: h
                            .color_channel
                            .as_ref()
                            .and_then(|pc| save_analysis_channel(pc, shared)),
                        color_bands: h.color_bands,
                        lock_x_range: h.lock_x_range,
                        x_min: h.x_min,
                        x_max: h.x_max,
                        lock_y_range: h.lock_y_range,
                        y_min: h.y_min,
                        y_headroom_pct: h.y_headroom_pct,
                        channel_names: h
                            .channels
                            .iter()
                            .filter_map(|pc| {
                                saved_channel_identity(pc, shared).map(|(name, _)| name)
                            })
                            .collect(),
                        is_math: h
                            .channels
                            .iter()
                            .filter_map(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .collect(),
                        colors: h
                            .channels
                            .iter()
                            .map(|pc| {
                                let c = pc.color;
                                [c.r(), c.g(), c.b()]
                            })
                            .collect(),
                    }),
                    PanelTab::Scatter(s) => PanelConfig::Scatter(ScatterPanelConfig {
                        show_interval_averages: s.show_interval_averages,
                        interval_width: s.interval_width,
                        interval_origin: s.interval_origin,
                        density: s.density,
                        band_presentation: s.band_presentation.clone(),
                        x_channel: s
                            .x_channel
                            .as_ref()
                            .and_then(|p| save_analysis_channel(p, shared)),
                        y_channel: s
                            .y_channel
                            .as_ref()
                            .and_then(|p| save_analysis_channel(p, shared)),
                        filter: s.filter.clone(),
                        color_channel: s
                            .color_channel
                            .as_ref()
                            .and_then(|c| save_analysis_channel(c, shared)),
                        color_bands: s.color_bands,
                        show_regression: s.show_regression,
                        id: s.id,
                        title: s.title.clone(),
                        x_channel_name: s.x_channel.as_ref().and_then(|pc| {
                            saved_channel_identity(pc, shared).map(|(name, _)| name)
                        }),
                        y_channel_name: s.y_channel.as_ref().and_then(|pc| {
                            saved_channel_identity(pc, shared).map(|(name, _)| name)
                        }),
                        x_is_math: s
                            .x_channel
                            .as_ref()
                            .and_then(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        y_is_math: s
                            .y_channel
                            .as_ref()
                            .and_then(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        x_color: s.x_channel.as_ref().map(|pc| {
                            let c = pc.color;
                            [c.r(), c.g(), c.b()]
                        }),
                        y_color: s.y_channel.as_ref().map(|pc| {
                            let c = pc.color;
                            [c.r(), c.g(), c.b()]
                        }),
                        point_size: s.point_size,
                        bounds_padding_frac: s.bounds_padding_frac,
                        lock_bounds: s.lock_bounds,
                    }),
                    PanelTab::Fft(f) => PanelConfig::Fft(FftPanelConfig {
                        channels: f
                            .channels
                            .iter()
                            .filter_map(|p| save_analysis_channel(p, shared))
                            .collect(),
                        id: f.id,
                        title: f.title.clone(),
                        log_scale: f.log_scale,
                    }),
                    PanelTab::Gauge(g) => PanelConfig::Gauge(GaugePanelConfig {
                        gauges: g
                            .gauges
                            .iter()
                            .filter_map(|gc| {
                                save_analysis_channel(&gc.channel, shared).map(|channel| {
                                    SavedGauge {
                                        channel,
                                        style: gauge_style_to_string(gc.style).into(),
                                        settings: gc.settings,
                                    }
                                })
                            })
                            .chain(g.unresolved_gauges.iter().cloned())
                            .collect(),
                        id: g.id,
                        title: g.title.clone(),
                    }),
                    PanelTab::MixtureMap(m) => PanelConfig::MixtureMap(MixtureMapPanelConfig {
                        show_trends: m.show_trends,
                        trend_width: m.trend_width,
                        band_presentation: m.band_presentation.clone(),
                        x_channel: m
                            .x_channel
                            .as_ref()
                            .and_then(|p| save_analysis_channel(p, shared)),
                        y_channel: m
                            .y_channel
                            .as_ref()
                            .and_then(|p| save_analysis_channel(p, shared)),
                        value_channel: m
                            .value_channel
                            .as_ref()
                            .and_then(|p| save_analysis_channel(p, shared)),
                        filter: m.filter.clone(),
                        gear_filter: m.gear_filter,
                        id: m.id,
                        title: m.title.clone(),
                        bins: m.bins,
                        scatter_mode: m.scatter_mode,
                        sensor_delay: m.sensor_delay,
                        minimum_y: m.minimum_y,
                        color_bands: m.color_bands,
                        x_channel_name: m.x_channel.as_ref().and_then(|pc| {
                            saved_channel_identity(pc, shared).map(|(name, _)| name)
                        }),
                        y_channel_name: m.y_channel.as_ref().and_then(|pc| {
                            saved_channel_identity(pc, shared).map(|(name, _)| name)
                        }),
                        value_channel_name: m.value_channel.as_ref().and_then(|pc| {
                            saved_channel_identity(pc, shared).map(|(name, _)| name)
                        }),
                        x_is_math: m
                            .x_channel
                            .as_ref()
                            .and_then(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        y_is_math: m
                            .y_channel
                            .as_ref()
                            .and_then(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        value_is_math: m
                            .value_channel
                            .as_ref()
                            .and_then(|pc| {
                                saved_channel_identity(pc, shared).map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                    }),
                };
                panels.push(config);
            }
            WorksheetConfig {
                dock: Some(finite_dock_layout(dock, &locations)),
                window: Default::default(),
                name: name.clone(),
                panels,
            }
        })
        .collect();

    let math_channels: Vec<MathChannelConfig> = shared
        .math_channels
        .iter()
        .map(|mc| MathChannelConfig {
            options: mc.options.clone(),
            name: mc.name.clone(),
            expression: mc.expression.clone(),
            unit: mc.unit.clone(),
            dec_places: mc.dec_places,
        })
        .collect();

    let mut channel_aliases: Vec<ChannelAliasConfig> = shared
        .channel_aliases
        .iter()
        .map(|(alias, target)| ChannelAliasConfig {
            alias: alias.clone(),
            target: target.clone(),
        })
        .collect();

    channel_aliases.sort_by(|a, b| a.alias.cmp(&b.alias));

    let sectors: Vec<Sector> = shared.sectors.clone();

    WorkspaceFile {
        version: 2,
        comparisons: Some(ComparisonConfig {
            main_lap: shared.comparisons.main_lap,
            overlays: shared
                .comparisons
                .overlays
                .iter()
                .map(|o| GraphOverlayConfig {
                    gps_align: o.gps_align,
                    visible: o.visible,
                    color: o.color.map(|c| [c.r(), c.g(), c.b()]),
                    file_path: match o.source {
                        OverlaySource::MainSession => None,
                        OverlaySource::External(i) => shared
                            .comparisons
                            .sessions
                            .get(i)
                            .map(|s| s.path.to_string_lossy().into_owned()),
                    },
                    lap_index: o.lap_idx,
                    manual_offset: o.manual_offset,
                    stretch_to_reference: o.stretch_to_reference,
                })
                .chain(shared.comparisons.unresolved_overlays.iter().cloned())
                .collect(),
        }),
        worksheets: ws_configs,
        active_worksheet,
        last_file_path: shared
            .ld_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        math_channels,
        channel_aliases,
        channel_settings: shared.channel_settings.clone(),
        recording_analyses: shared.snapshot_recording_analysis(),
        sectors,
        track_definition: shared
            .track_data_if_ready()
            .and_then(|track| {
                i3rs_core::export_track_definition(&track, &shared.sectors, "Track sections".into())
            })
            .or_else(|| shared.pending_track_definition.clone()),
        reference_lap: shared.reference_lap,
    }
}

// ---------------------------------------------------------------------------
// Conversion: WorkspaceFile → App state
// ---------------------------------------------------------------------------

/// Restore section identity before asynchronous math/layout restoration can wait.
pub(crate) fn restore_track_sections(workspace: &WorkspaceFile, shared: &mut SharedState) {
    shared.track_definition_error = None;
    if !workspace.recording_analyses.is_empty() {
        for (key, analysis) in &workspace.recording_analyses {
            if key != &analysis.recording {
                shared.message("Ignored mismatched recording analysis identity");
                continue;
            }
            match analysis.validate() {
                Ok(()) => {
                    if key != shared.recording_key() {
                        shared
                            .recording_analyses
                            .insert(key.clone(), analysis.clone());
                    }
                }
                Err(error) => {
                    shared.message(format!("Recording analysis: {error}"));
                }
            }
        }
        if let Some(analysis) = workspace
            .recording_analyses
            .get(shared.recording_key())
            .cloned()
        {
            let applied = shared.apply_recording_analysis(analysis);
            if let Err(error) = applied {
                shared.message(error);
            }
            shared.reference_lap = workspace.reference_lap.filter(|i| *i < shared.laps.len());
            return;
        }
    }
    shared.pending_track_definition = workspace.track_definition.clone();
    // Missing paths cannot establish recording identity (e.g. browser uploads).
    let same_recording = workspace
        .last_file_path
        .as_deref()
        .is_some_and(|saved| Some(saved) == shared.ld_path.as_ref().and_then(|p| p.to_str()));
    // Legacy index-only sections are valid only for the original recording.
    shared.sectors = if workspace.track_definition.is_none() && same_recording {
        workspace.sectors.clone()
    } else {
        Vec::new()
    };
    shared.reference_lap = workspace.reference_lap.filter(|i| *i < shared.laps.len());
    shared.apply_pending_track_definition();
}

pub fn load_workspace(
    workspace: &WorkspaceFile,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    shared.comparisons = Default::default();
    let legacy = workspace
        .worksheets
        .iter()
        .flat_map(|w| &w.panels)
        .find_map(|p| {
            if let PanelConfig::Graph(g) = p {
                g.reference_lap.map(|_| g)
            } else {
                None
            }
        });
    let (main, overlays) = workspace
        .comparisons
        .as_ref()
        .map(|c| (c.main_lap, c.overlays.as_slice()))
        .or_else(|| legacy.map(|g| (g.reference_lap, g.overlays.as_slice())))
        .unwrap_or((None, &[]));
    let mut loader = GraphPanel::new(0, "comparison loader");
    for overlay in overlays {
        let source = if let Some(path) = &overlay.file_path {
            loader
                .load_external_overlay_path(path.into())
                .map(OverlaySource::External)
        } else {
            Some(OverlaySource::MainSession)
        };
        if let Some(source) = source {
            loader.lap_overlays.push(crate::panels::graph::LapOverlay {
                alignment_valid: false,
                gps_align: overlay.gps_align,
                source,
                lap_idx: overlay.lap_index,
                visible: overlay.visible,
                color: overlay
                    .color
                    .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2])),
                manual_offset: overlay.manual_offset,
                stretch_to_reference: overlay.stretch_to_reference,
            });
        } else {
            shared.message(format!("Overlay file unavailable: {:?}", overlay.file_path));
            shared.comparisons.unresolved_overlays.push(overlay.clone());
        }
    }
    shared.comparisons.main_lap = main;
    shared.comparisons.overlays = loader.lap_overlays;
    shared.comparisons.sessions = loader.overlay_sessions;
    shared.channel_settings = workspace.channel_settings.clone();
    restore_track_sections(workspace, shared);
    if let Some(main) = main.filter(|i| *i < shared.laps.len()) {
        shared.select_lap(Some(main));
        shared.reference_lap = Some(main);
    }
    workspace
        .worksheets
        .iter()
        .enumerate()
        .map(|(worksheet_idx, ws_config)| {
            let tabs: Vec<PanelTab> = ws_config
                .panels
                .iter()
                .map(|panel| match panel {
                    PanelConfig::Calculations(c) => {
                        let mut p =
                            crate::panels::calculations::CalculationsPanel::new(c.id, &c.title);
                        p.channels = c
                            .channels
                            .iter()
                            .filter_map(|s| load_analysis_channel(s, shared))
                            .collect();
                        p.filter = c.filter.clone();
                        p.mode = c.mode;
                        p.interval_seconds = c.interval_seconds;
                        p.suspension = c.suspension;
                        p.suspension_bins=c.suspension_bins;p.suspension_output=c.suspension_output;p.suspension_line=c.suspension_line;
                        shared.next_panel_id = shared.next_panel_id.max(c.id + 1);
                        PanelTab::Calculations(p)
                    }
                    PanelConfig::Graph(gc) => {
                        let mut graph = GraphPanel::new(gc.id, &gc.title);
                        graph.graph_mode = match gc.graph_mode.as_str() {
                            "Overlay" => GraphMode::Overlay,
                            _ => GraphMode::Tiled,
                        };
                        graph.x_axis_mode = match gc.x_axis_mode.as_str() {
                            "Distance" => GraphXAxis::Distance,
                            _ => GraphXAxis::Time,
                        };
                        graph.full_resolution = gc.full_resolution;
                        graph.view = gc.view.clone();
                        graph.unresolved_channels = gc.unresolved_channels.clone();
                        graph.embedded_gauge_height = gc.embedded_gauge_height;
                        graph.tile_heights = gc.tile_heights.clone();

                        if !gc.channels.is_empty() {
                            graph.unresolved_channels.clear();
                            for slot in &gc.channels {
                                if let Some(mut pc) = load_analysis_channel(&slot.channel, shared) {
                                    pc.tile_group = slot.tile_group;
                                    graph.plotted_channels.push(pc);
                                }
                            }
                        } else {
                            for (i, name) in gc.channel_names.iter().enumerate() {
                                let color = gc.colors.get(i).copied().unwrap_or([255, 100, 100]);
                                let slot = GraphChannelSlot {
                                    channel: AnalysisChannelConfig {
                                        source_unit: None,
                                        inherit_display: gc
                                            .is_math
                                            .get(i)
                                            .copied()
                                            .unwrap_or(false)
                                            && gc.display_transforms.get(i).is_none(),
                                        name: name.clone(),
                                        is_math: gc.is_math.get(i).copied().unwrap_or(false),
                                        color,
                                        transform: gc
                                            .display_transforms
                                            .get(i)
                                            .cloned()
                                            .unwrap_or_default(),
                                    },
                                    tile_group: gc.tile_groups.get(i).copied().unwrap_or(i),
                                };
                                if let Some(mut pc) = load_analysis_channel(&slot.channel, shared) {
                                    pc.tile_group = slot.tile_group;
                                    graph.plotted_channels.push(pc);
                                } else {
                                    graph.unresolved_channels.push(slot);
                                }
                            }
                        }

                        for saved in &gc.embedded_gauges {
                            let slot = SavedGauge {
                                channel: AnalysisChannelConfig {
                                    source_unit: None,
                                    inherit_display: saved.transform.is_none(),
                                    name: saved.channel_name.clone(),
                                    is_math: saved.is_math,
                                    color: [255, 100, 100],
                                    transform: saved.transform.clone().unwrap_or_default(),
                                },
                                style: saved.style.clone(),
                                settings: saved.settings,
                            };
                            let slot = SavedGauge {
                                channel: saved.channel.clone().unwrap_or(slot.channel),
                                ..slot
                            };
                            if let Some(channel) = load_analysis_channel(&slot.channel, shared) {
                                graph
                                    .embedded_gauges
                                    .push(crate::panels::gauge::GaugeChannel {
                                        last_unit: None,
                                        channel,
                                        style: gauge_style_from_string(&slot.style),
                                        settings: slot.settings,
                                    });
                            } else {
                                graph.unresolved_gauges.push(slot);
                            }
                        }

                        // Track max panel id
                        if gc.id >= shared.next_panel_id {
                            shared.next_panel_id = gc.id + 1;
                        }

                        PanelTab::Graph(graph)
                    }
                    PanelConfig::TrackMap(tc) => {
                        let mut track_map = TrackMapPanel::new(tc.id, &tc.title);
                        track_map.window=tc.window.clone();
                        #[cfg(not(target_arch="wasm32"))] {track_map.is_popped_out=tc.window.detached;}
                        track_map.home_worksheet = worksheet_idx;
                        track_map.section_times_only = tc.section_times_only;
                        track_map.show_annotations = tc.show_annotations;
                        track_map.annotation_settings = tc.annotation_settings.clone();
                        track_map.requested_color = tc.color_channel_name.clone();
                        // Resolve color channel by name
                        if let Some(ref color_name) = tc.color_channel_name
                            && let Some(ld) = &shared.ld_file
                            && let Some(idx) =
                                ld.find_channel_by_name(color_name).map(|ch| ch.index)
                        {
                            track_map.color_channel_idx = Some(idx);
                        }
                        if tc.id >= shared.next_panel_id {
                            shared.next_panel_id = tc.id + 1;
                        }
                        PanelTab::TrackMap(track_map)
                    }
                    PanelConfig::ChannelBrowser => PanelTab::ChannelBrowser,
                    PanelConfig::CursorReadout => PanelTab::CursorReadout,
                    PanelConfig::Report(rc) => {
                        let mut report = crate::panels::report::ReportPanel::new(rc.id, &rc.title);
                        report.settings = rc.settings.clone();
                        report.channels = rc
                            .channels
                            .iter()
                            .filter_map(|c| load_analysis_channel(c, shared))
                            .collect();
                        if rc.id >= shared.next_panel_id {
                            shared.next_panel_id = rc.id + 1;
                        }
                        PanelTab::Report(report)
                    }
                    PanelConfig::Histogram(hc) => {
                        let mut histogram = HistogramPanel::new(hc.id, &hc.title);
                        histogram.band_presentation = hc.band_presentation.clone();
                        histogram.bin_settings = hc.bin_settings;
                        histogram.filter = hc.filter.clone();
                        histogram.seconds = hc.seconds && !hc.percentage;
                        histogram.bin_count = hc.bin_count.clamp(5, 500);
                        histogram.percentage = hc.percentage;
                        histogram.color_bands = hc.color_bands.valid();
                        histogram.color_channel = hc
                            .color_channel
                            .as_ref()
                            .and_then(|c| load_analysis_channel(c, shared));
                        histogram.per_lap = hc.per_lap;
                        histogram.lock_x_range = hc.lock_x_range;
                        histogram.x_min = hc.x_min;
                        histogram.x_max = hc.x_max;
                        histogram.lock_y_range = hc.lock_y_range;
                        histogram.y_min = hc.y_min;
                        histogram.y_headroom_pct = hc.y_headroom_pct;
                        for (i, channel_name) in hc.channel_names.iter().enumerate() {
                            let is_math = hc.is_math.get(i).copied().unwrap_or(false);
                            if let Some(pc) = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                is_math,
                                hc.colors
                                    .get(i)
                                    .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
                                    .unwrap_or(CHANNEL_COLORS[i % CHANNEL_COLORS.len()]),
                                i,
                            ) {
                                histogram.channels.push(pc);
                            }
                        }
                        if !hc.channels.is_empty() {
                            histogram.channels = hc
                                .channels
                                .iter()
                                .filter_map(|p| load_analysis_channel(p, shared))
                                .collect();
                        }
                        if hc.id >= shared.next_panel_id {
                            shared.next_panel_id = hc.id + 1;
                        }
                        PanelTab::Histogram(histogram)
                    }
                    PanelConfig::Scatter(sc) => {
                        let mut scatter = ScatterPanel::new(sc.id, &sc.title);
                        scatter.show_interval_averages=sc.show_interval_averages;scatter.interval_width=sc.interval_width;scatter.interval_origin=sc.interval_origin;
                        scatter.band_presentation = sc.band_presentation.clone();
                        scatter.density = sc.density;
                        scatter.filter = sc.filter.clone();
                        scatter.color_channel = sc
                            .color_channel
                            .as_ref()
                            .and_then(|c| load_analysis_channel(c, shared));
                        scatter.color_bands = sc.color_bands;
                        scatter.show_regression = sc.show_regression;
                        scatter.point_size = sc.point_size;
                        scatter.bounds_padding_frac = sc.bounds_padding_frac;
                        scatter.lock_bounds = sc.lock_bounds;
                        if let Some(channel_name) = &sc.x_channel_name {
                            scatter.x_channel = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                sc.x_is_math,
                                sc.x_color
                                    .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
                                    .unwrap_or(CHANNEL_COLORS[0]),
                                0,
                            );
                        }
                        if let Some(channel_name) = &sc.y_channel_name {
                            scatter.y_channel = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                sc.y_is_math,
                                sc.y_color
                                    .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
                                    .unwrap_or(CHANNEL_COLORS[4]),
                                1,
                            );
                        }
                        if let Some(c) = &sc.x_channel {
                            scatter.x_channel = load_analysis_channel(c, shared);
                        }
                        if let Some(c) = &sc.y_channel {
                            scatter.y_channel = load_analysis_channel(c, shared);
                        }
                        if sc.id >= shared.next_panel_id {
                            shared.next_panel_id = sc.id + 1;
                        }
                        PanelTab::Scatter(Box::new(scatter))
                    }
                    PanelConfig::Fft(fc) => {
                        let mut fft = FftPanel::new(fc.id, &fc.title);
                        fft.log_scale = fc.log_scale;
                        fft.channels = fc
                            .channels
                            .iter()
                            .filter_map(|p| load_analysis_channel(p, shared))
                            .collect();
                        if fc.id >= shared.next_panel_id {
                            shared.next_panel_id = fc.id + 1;
                        }
                        PanelTab::Fft(fft)
                    }
                    PanelConfig::Gauge(gc) => {
                        let mut gauge = GaugePanel::new(gc.id, &gc.title);
                        for saved in &gc.gauges {
                            if let Some(channel) = load_analysis_channel(&saved.channel, shared) {
                                gauge.gauges.push(crate::panels::gauge::GaugeChannel {
                                    last_unit: None,
                                    channel,
                                    style: gauge_style_from_string(&saved.style),
                                    settings: saved.settings,
                                });
                            } else {
                                gauge.unresolved_gauges.push(saved.clone());
                            }
                        }
                        if gc.id >= shared.next_panel_id {
                            shared.next_panel_id = gc.id + 1;
                        }
                        PanelTab::Gauge(gauge)
                    }
                    PanelConfig::MixtureMap(mc) => {
                        let mut mixture_map = MixtureMapPanel::new(mc.id, &mc.title);
                        mixture_map.show_trends=mc.show_trends;mixture_map.trend_width=mc.trend_width;
                        mixture_map.band_presentation = mc.band_presentation.clone();
                        mixture_map.filter = mc.filter.clone();
                        mixture_map.gear_filter = mc.gear_filter;
                        mixture_map.bins = mc.bins.clamp(5, 100);
                        mixture_map.scatter_mode = mc.scatter_mode;
                        mixture_map.sensor_delay =
                            crate::panels::mixture_map::normalized_sensor_delay(mc.sensor_delay);
                        mixture_map.minimum_y = mc.minimum_y.filter(|v| v.is_finite());
                        mixture_map.color_bands = mc.color_bands.valid();
                        if let Some(channel_name) = &mc.x_channel_name {
                            mixture_map.x_channel = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                mc.x_is_math,
                                CHANNEL_COLORS[0],
                                0,
                            );
                        }
                        if let Some(channel_name) = &mc.y_channel_name {
                            mixture_map.y_channel = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                mc.y_is_math,
                                CHANNEL_COLORS[1],
                                1,
                            );
                        }
                        if let Some(channel_name) = &mc.value_channel_name {
                            mixture_map.value_channel = resolve_saved_plotted_channel(
                                shared,
                                channel_name,
                                mc.value_is_math,
                                CHANNEL_COLORS[2],
                                2,
                            );
                        }
                        if let Some(c) = &mc.x_channel {
                            mixture_map.x_channel = load_analysis_channel(c, shared);
                        }
                        if let Some(c) = &mc.y_channel {
                            mixture_map.y_channel = load_analysis_channel(c, shared);
                        }
                        if let Some(c) = &mc.value_channel {
                            mixture_map.value_channel = load_analysis_channel(c, shared);
                        }
                        if mc.id >= shared.next_panel_id {
                            shared.next_panel_id = mc.id + 1;
                        }
                        PanelTab::MixtureMap(Box::new(mixture_map))
                    }
                })
                .collect();

            if let Some(topology) = &ws_config.dock {
                let indices: Vec<_> = topology.iter_all_tabs().map(|(_, i)| *i).collect();
                let unique: std::collections::HashSet<_> = indices.iter().copied().collect();
                if unique.len() == indices.len()
                    && tabs.iter().enumerate().all(|(i,_)|unique.contains(&i)||matches!(&ws_config.panels[i],PanelConfig::TrackMap(c) if c.window.detached))
                    && indices.iter().all(|i| *i < tabs.len())
                    && !tabs.is_empty()
                {
                    let mut slots: Vec<_> = tabs.into_iter().map(Some).collect();
                    let mut dock = topology
                        .map_tabs(|i| slots[*i].take().expect("validated unique layout slot"));
                    for tab in slots.into_iter().flatten(){
                        #[cfg(not(target_arch="wasm32"))] dock.add_window(vec![tab]);
                        #[cfg(target_arch="wasm32")] dock.push_to_focused_leaf(tab);
                    }
                    return (ws_config.name.clone(), dock);
                }
                shared.message(format!(
                    "{}: invalid saved docking layout; restored tabs",
                    ws_config.name
                ));
            }

            // Build dock state — put first tab as root, rest as tabbed
            if tabs.is_empty() {
                return (
                    ws_config.name.clone(),
                    DockState::new(vec![PanelTab::ChannelBrowser]),
                );
            }

            let mut tabs_iter = tabs.into_iter();
            let first = tabs_iter.next().unwrap();
            let mut dock = DockState::new(vec![first]);
            for tab in tabs_iter {
                dock.push_to_focused_leaf(tab);
            }

            let has_browser = dock
                .iter_all_tabs()
                .any(|(_, t)| matches!(t, PanelTab::ChannelBrowser));
            if has_browser
                && let Some(path) = dock.find_tab_from(|t| matches!(t, PanelTab::ChannelBrowser))
            {
                dock.remove_tab(path);
                dock.main_surface_mut().split_left(
                    NodeIndex::root(),
                    0.2,
                    vec![PanelTab::ChannelBrowser],
                );
            }

            let has_readout = dock
                .iter_all_tabs()
                .any(|(_, t)| matches!(t, PanelTab::CursorReadout));
            if has_readout
                && let Some(path) = dock.find_tab_from(|t| matches!(t, PanelTab::CursorReadout))
            {
                dock.remove_tab(path);
                dock.main_surface_mut().split_right(
                    NodeIndex::root(),
                    0.8,
                    vec![PanelTab::CursorReadout],
                );
            }

            (ws_config.name.clone(), dock)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::apply_saved_scale_limits;
    use crate::state::{ChannelId, PlottedChannel, ScaleMode, YAxis};
    use std::sync::Arc;

    fn sample_channel() -> PlottedChannel {
        PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![0.0, 100.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: ScaleMode::Manual,
            manual_min: 10.0,
            manual_max: 20.0,
            cached_min: 0.0,
            cached_max: 100.0,
            cached_avg: 50.0,
        }
    }

    #[test]
    fn scoped_display_transforms_save_base_units_and_manual_limits() {
        use super::*;
        let mut channel = sample_channel();
        channel.recording_display_base = Some((1.0, 0.0, None));
        channel.display_scale = 2.0;
        channel.display_offset = 10.0;
        channel.display_unit = Some("converted".into());
        channel.manual_min = 30.0;
        channel.manual_max = 50.0;
        let config = GraphDisplayTransformConfig::of(&channel);
        assert_eq!((config.scale, config.offset), (1.0, 0.0));
        assert_eq!(config.scale_manual, Some((10.0, 20.0)));
        assert!(config.unit.is_none());
        config.apply_to(&mut channel);
        assert!(channel.recording_display_base.is_none());
        assert_eq!((channel.manual_min, channel.manual_max), (10.0, 20.0));
    }

    #[test]
    fn math_default_units_and_explicit_raw_panel_override_survive_reload() {
        use super::*;
        let mut shared = SharedState::new();
        let mut mc = shared.create_math_channel_def("Angle".into(), "0.1".into(), "rad".into(), 2);
        mc.options.display_unit = Some("deg".into());
        mc.freq = 1;
        mc.data = Some(Arc::from(vec![0.1]));
        shared.math_channels.push(mc);
        let mut graph = GraphPanel::new(1, "Angle");
        graph
            .plotted_channels
            .push(create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap());
        graph
            .plotted_channels
            .push(create_plotted_channel(ChannelId::Math(0), &shared, 1).unwrap());
        crate::panels::utils::set_plotted_channel_display_transform(
            &mut graph.plotted_channels[1],
            1.0,
            0.0,
            None,
        );
        let dock = DockState::new(vec![PanelTab::Graph(graph)]);
        let workspace = save_workspace(&[("Angle".into(), &dock)], 0, &shared);
        let mut restored = load_workspace(&workspace, &mut shared);
        let (_, PanelTab::Graph(graph)) = restored[0].1.iter_all_tabs_mut().next().unwrap() else {
            panic!()
        };
        let channels = &mut graph.plotted_channels;
        for channel in channels.iter_mut() {
            crate::panels::utils::refresh_plotted_channel(channel, &shared);
        }
        assert_eq!(channels[0].display_unit.as_deref(), Some("deg"));
        assert_eq!(channels[1].display_scale, 1.0);
        assert!(channels[1].display_unit.is_none());
        assert_eq!(
            resolve_saved_plotted_channel(&shared, "User::Angle", true, egui::Color32::WHITE, 0)
                .unwrap()
                .display_unit
                .as_deref(),
            Some("deg")
        );
    }

    #[test]
    fn workspace_preserves_source_qualified_math_selections() {
        use super::*;
        let mut shared = SharedState::new();
        for (source, value) in [("User", 1.0), ("Temporary", 2.0)] {
            let mut mc =
                shared.create_math_channel_def("Same".into(), value.to_string(), "".into(), 0);
            mc.options.source = source.into();
            mc.freq = 1;
            mc.data = Some(Arc::from(vec![value]));
            shared.math_channels.push(mc);
        }
        let mut graph = crate::panels::graph::GraphPanel::new(1, "Sources");
        graph
            .plotted_channels
            .push(create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap());
        graph.add_embedded_gauge_with_style(
            ChannelId::Math(0),
            &shared,
            crate::panels::gauge::GaugeStyle::Digital,
        );
        let dock = DockState::new(vec![PanelTab::Graph(graph)]);
        let workspace = save_workspace(&[("Source choice".into(), &dock)], 0, &shared);
        let PanelConfig::Graph(config) = &workspace.worksheets[0].panels[0] else {
            panic!()
        };
        assert_eq!(config.channel_names, ["User::Same"]);
        assert_eq!(config.embedded_gauges[0].channel_name, "User::Same");
        let restored = load_workspace(&workspace, &mut shared);
        let (_, PanelTab::Graph(graph)) = restored[0].1.iter_all_tabs().next().unwrap() else {
            panic!()
        };
        assert_eq!(graph.plotted_channels[0].channel_id, ChannelId::Math(0));
        assert_eq!(
            graph.embedded_gauges[0].channel.channel_id,
            ChannelId::Math(0)
        );
    }

    #[test]
    fn circuit_panels_roundtrip_with_independent_report_channels() {
        use super::*;
        let mut shared = SharedState::new();
        let mut channel =
            shared.create_math_channel_def("Engine test".into(), "1".into(), "rpm".into(), 1);
        channel.freq = 2;
        channel.data = Some(Arc::from(vec![1000.0, 2000.0, 3000.0]));
        shared.math_channels.push(channel);
        let mut report = crate::panels::report::ReportPanel::new(1, "Independent report");
        report.add_channel(ChannelId::Math(0), &shared);
        let mut histogram = HistogramPanel::new(2, "RPM");
        histogram.percentage = true;
        histogram.bin_count = 90;
        histogram.lock_x_range = true;
        histogram.x_min = 0.0;
        histogram.x_max = 9000.0;
        histogram.color_bands.count = 10;
        histogram.color_bands.centered = false;
        histogram.channels.push(sample_channel());
        histogram.channels[0].channel_id = ChannelId::Math(0);
        histogram.color_channel = create_plotted_channel(ChannelId::Math(0), &shared, 0);
        let mut mixture = MixtureMapPanel::new(3, "Mixture");
        mixture.scatter_mode = true;
        mixture.sensor_delay = 0.15;
        mixture.minimum_y = Some(0.5);
        let mut track = TrackMapPanel::new(4, "Section Times");
        track.section_times_only = true;
        let mut graph = GraphPanel::new(5, "Full samples");
        graph.full_resolution = true;
        let dock = DockState::new(vec![
            PanelTab::Graph(graph),
            PanelTab::Report(report),
            PanelTab::Histogram(histogram),
            PanelTab::MixtureMap(Box::new(mixture)),
            PanelTab::TrackMap(track),
        ]);
        let saved = save_workspace(&[("Circuit".into(), &dock)], 0, &shared);
        let json = serde_json::to_string(&saved).unwrap();
        let decoded: WorkspaceFile = serde_json::from_str(&json).unwrap();
        let loaded = load_workspace(&decoded, &mut shared);
        assert!(shared.display_channel_registry.is_empty());
        let tabs: Vec<_> = loaded[0].1.iter_all_tabs().map(|(_, tab)| tab).collect();
        let report = tabs
            .iter()
            .find_map(|tab| {
                if let PanelTab::Report(r) = tab {
                    Some(r)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(report.channels.len(), 1);
        assert_eq!(report.channels[0].channel_id, ChannelId::Math(0));
        assert!(tabs.iter().any(
            |t| matches!(t, PanelTab::Histogram(h) if h.percentage && h.color_channel.is_some()
                && h.bin_count == 90 && h.lock_x_range && h.x_min == 0.0 && h.x_max == 9000.0
                && h.color_bands.count == 10 && !h.color_bands.centered)
        ));
        assert!(
            tabs.iter()
                .any(|t| matches!(t, PanelTab::MixtureMap(m) if m.scatter_mode && m.sensor_delay == 0.15 && m.minimum_y == Some(0.5) && m.color_bands.centered))
        );
        assert!(
            tabs.iter()
                .any(|t| matches!(t, PanelTab::TrackMap(t) if t.section_times_only))
        );
        assert!(
            tabs.iter()
                .any(|t| matches!(t, PanelTab::Graph(g) if g.full_resolution))
        );
        let old_graph: GraphPanelConfig = serde_json::from_str(
            r#"{"id":1,"title":"Graph","channel_names":[],"colors":[],"graph_mode":"Tiled"}"#,
        )
        .unwrap();
        assert!(!old_graph.full_resolution);
        // Old workspace files remain readable without any new fields.
        let legacy: WorkspaceFile =
            serde_json::from_str(r#"{"worksheets":[],"active_worksheet":0,"last_file_path":null}"#)
                .unwrap();
        assert!(legacy.track_definition.is_none());
    }

    #[test]
    fn legacy_section_indices_require_a_known_matching_recording() {
        let mut shared = crate::state::SharedState::new();
        let mut workspace: super::WorkspaceFile = serde_json::from_str(
            r#"{"worksheets":[],"active_worksheet":0,"last_file_path":null,"sectors":[{"name":"Old","start_index":1,"end_index":2}]}"#,
        ).unwrap();
        super::restore_track_sections(&workspace, &mut shared);
        assert!(shared.sectors.is_empty());
        workspace.last_file_path = Some("original.ld".into());
        shared.ld_path = Some("original.ld".into());
        super::restore_track_sections(&workspace, &mut shared);
        assert_eq!(shared.sectors.len(), 1);
        shared.ld_path = Some("different.ld".into());
        super::restore_track_sections(&workspace, &mut shared);
        assert!(shared.sectors.is_empty());
    }

    #[test]
    fn saved_auto_scale_overrides_inherited_manual_preference() {
        let mut channel = sample_channel();

        apply_saved_scale_limits(&mut channel, None);

        assert_eq!(channel.scale_mode, ScaleMode::Auto);
        assert_eq!((channel.manual_min, channel.manual_max), (0.0, 100.0));
    }

    #[test]
    fn saved_manual_scale_restores_limits() {
        let mut channel = sample_channel();

        apply_saved_scale_limits(&mut channel, Some((-5.0, 25.0)));

        assert_eq!(channel.scale_mode, ScaleMode::Manual);
        assert_eq!((channel.manual_min, channel.manual_max), (-5.0, 25.0));
    }
}

#[cfg(test)]
mod configuration_tests {
    use super::*;
    use crate::panels::gauge::{GaugeChannel, GaugeSettings};
    use std::sync::Arc;
    fn slot(name: &str) -> AnalysisChannelConfig {
        AnalysisChannelConfig {
            source_unit: None,
            inherit_display: false,
            name: name.into(),
            is_math: false,
            color: [10, 20, 30],
            transform: Default::default(),
        }
    }
    #[test]
    fn unresolved_graph_and_gauge_slots_survive_repeated_round_trips_and_rebind() {
        let mut shared = SharedState::new();
        let mut graph = GraphPanel::new(1, "Graph");
        graph.view.mode = crate::comparison::ViewMode::Independent;
        graph.view.local_range = Some((2., 4.));
        graph.unresolved_channels.push(GraphChannelSlot {
            channel: slot("Engine Speed"),
            tile_group: 3,
        });
        let mut gauges = GaugePanel::new(2, "Gauges");
        gauges.unresolved_gauges.push(SavedGauge {
            channel: slot("Engine Speed"),
            style: "Analog".into(),
            settings: GaugeSettings {
                range: Some((0., 9000.)),
                warning_above: Some(7000.),
                fault_above: Some(8500.),
                ..Default::default()
            },
        });
        let dock = DockState::new(vec![PanelTab::Graph(graph), PanelTab::Gauge(gauges)]);
        let workspace = save_workspace(&[("Driver".into(), &dock)], 0, &shared);
        let loaded = load_workspace(&workspace, &mut shared);
        let roundtrip = save_workspace(&[(loaded[0].0.clone(), &loaded[0].1)], 0, &shared);
        let roundtrip: WorkspaceFile =
            serde_json::from_str(&serde_json::to_string(&roundtrip).unwrap()).unwrap();
        assert_eq!(roundtrip.version, 2);
        let PanelConfig::Graph(g) = &roundtrip.worksheets[0].panels[0] else {
            panic!()
        };
        assert_eq!(g.channels.len(), 1);
        assert_eq!(g.channels[0].tile_group, 3);
        assert_eq!(g.view.local_range, Some((2., 4.)));
        shared.ld_file = Some(Arc::new(
            i3rs_core::LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        ));
        assert!(load_analysis_channel(&g.channels[0].channel, &shared).is_some());
        let PanelConfig::Gauge(g) = &roundtrip.worksheets[0].panels[1] else {
            panic!()
        };
        assert_eq!(g.gauges[0].settings.fault_above, Some(8500.));
        assert!(load_analysis_channel(&g.gauges[0].channel, &shared).is_some());
    }
    #[test]
    fn gauge_channels_transforms_and_properties_are_saved() {
        let mut shared = SharedState::new();
        let mut def = shared.create_math_channel_def("Test".into(), "1".into(), "rpm".into(), 2);
        def.data = Some(Arc::from(vec![1., 2.]));
        def.freq = 1;
        shared.math_channels.push(def);
        let mut gauge = GaugePanel::new(1, "Gauge");
        let mut channel = create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap();
        channel.display_scale = 2.;
        channel.display_offset = 3.;
        gauge.gauges.push(GaugeChannel {
            last_unit: None,
            channel,
            style: GaugeStyle::Bar,
            settings: GaugeSettings {
                precision: Some(3),
                ticks: 12,
                sweep_degrees: 180.,
                font_size: 22.,
                ..Default::default()
            },
        });
        let dock = DockState::new(vec![PanelTab::Gauge(gauge)]);
        let workspace = save_workspace(&[("Driver".into(), &dock)], 0, &shared);
        let restored = load_workspace(&workspace, &mut shared);
        let (_, PanelTab::Gauge(g)) = restored[0].1.iter_all_tabs().next().unwrap() else {
            panic!()
        };
        assert_eq!(g.gauges[0].channel.display_scale, 2.);
        assert_eq!(g.gauges[0].channel.display_offset, 3.);
        assert_eq!(g.gauges[0].settings.ticks, 12);
        assert_eq!(g.gauges[0].style, GaugeStyle::Bar);
    }
    #[test]
    fn old_workspace_without_version_or_new_settings_still_loads() {
        let json = r#"{"worksheets":[{"name":"Driver","panels":[{"Gauge":{"id":1,"title":"Gauge"}}]}],"active_worksheet":0,"last_file_path":null}"#;
        let workspace: WorkspaceFile = serde_json::from_str(json).unwrap();
        assert_eq!(workspace.version, 0);
        assert_eq!(load_workspace(&workspace, &mut SharedState::new()).len(), 1);
    }
}

#[cfg(test)]
mod legacy_math_display_tests {
    use super::*;
    #[test]
    fn old_graph_without_saved_transforms_retains_math_display_units() {
        let mut shared = SharedState::new();
        let mut def = shared.create_math_channel_def("Angle".into(), "1".into(), "rad".into(), 2);
        def.options.display_unit = Some("deg".into());
        def.data = Some(std::sync::Arc::from(vec![1., 2.]));
        def.freq = 1;
        shared.math_channels.push(def);
        let json = r#"{"worksheets":[{"name":"Angles","panels":[{"Graph":{"id":1,"title":"Angle","channel_names":["Angle"],"colors":[[255,0,0]],"graph_mode":"Tiled","is_math":[true]}}]}],"active_worksheet":0,"last_file_path":null}"#;
        let workspace: WorkspaceFile = serde_json::from_str(json).unwrap();
        let restored = load_workspace(&workspace, &mut shared);
        let (_, PanelTab::Graph(graph)) = restored[0].1.iter_all_tabs().next().unwrap() else {
            panic!()
        };
        assert_eq!(
            graph.plotted_channels[0].display_unit.as_deref(),
            Some("deg")
        );
        assert!(
            (graph.plotted_channels[0].display_scale - 180. / std::f64::consts::PI).abs() < 1e-10
        );
    }
}

#[cfg(test)]
mod portable_component_tests {
    use super::*;
    use crate::panels::{calculations::CalculationsPanel, report::ReportPanel};
    fn slot(shared: &SharedState) -> crate::state::PlottedChannel {
        let id = shared.unresolved_channel("Missing::Requested", false);
        let mut pc = create_plotted_channel(id, shared, 0).unwrap();
        pc.requested_reference = Some(("Missing::Requested".into(), false));
        pc.display_scale = 2.;
        pc.display_offset = 3.;
        pc.display_unit = Some("display unit".into());
        pc
    }
    #[test]
    fn every_component_retains_missing_slots_and_split_floating_window_topology() {
        let mut shared = SharedState::new();
        let mut graph = GraphPanel::new(1, "Graph");
        graph.plotted_channels.push(slot(&shared));
        let mut histogram = HistogramPanel::new(2, "Histo");
        histogram.channels.push(slot(&shared));
        histogram.color_channel = Some(slot(&shared));
        histogram.bin_settings.width = Some(100.);
        histogram.bin_settings.centered = true;
        histogram.band_presentation.hidden.push(3);
        let mut scatter = ScatterPanel::new(3, "Scatter");
        scatter.x_channel = Some(slot(&shared));
        scatter.y_channel = Some(slot(&shared));
        scatter.color_channel = Some(slot(&shared));
        scatter.density = true;
        scatter.show_interval_averages = true;
        scatter.interval_width = 250.;
        scatter.interval_origin = 50.;
        let mut mixture = MixtureMapPanel::new(4, "Mixture");
        mixture.x_channel = Some(slot(&shared));
        mixture.y_channel = Some(slot(&shared));
        mixture.value_channel = Some(slot(&shared));
        mixture.show_trends = true;
        mixture.trend_width = 200.;
        let mut fft = FftPanel::new(5, "FFT");
        fft.channels.push(slot(&shared));
        let mut calc = CalculationsPanel::new(6, "Calculations");
        calc.channels.push(slot(&shared));
        calc.interval_seconds = 0.25;
        calc.suspension_bins = 51;
        calc.suspension_line = true;
        calc.suspension_output = crate::panels::calculations::DistributionOutput::Percent;
        let mut report = ReportPanel::new(7, "Report");
        report.channels.push(slot(&shared));
        let mut track = TrackMapPanel::new(8, "Track");
        track.requested_color = Some("Missing GPS colour".into());
        let mut dock = DockState::new(vec![PanelTab::Graph(graph), PanelTab::Histogram(histogram)]);
        dock.main_surface_mut().split_right(
            NodeIndex::root(),
            0.4,
            vec![
                PanelTab::Scatter(Box::new(scatter)),
                PanelTab::MixtureMap(Box::new(mixture)),
            ],
        );
        let window = dock.add_window(vec![
            PanelTab::Fft(fft),
            PanelTab::Calculations(calc),
            PanelTab::Report(report),
            PanelTab::TrackMap(track),
        ]);
        dock.get_window_state_mut(window)
            .unwrap()
            .set_position(egui::pos2(20., 30.))
            .set_size(egui::vec2(600., 500.));
        let mut saved = save_workspace(&[("Sheet".into(), &dock)], 0, &shared);
        saved.worksheets[0].window = WorksheetWindow {
            detached: true,
            size: [900., 600.],
            position: Some([100., 200.]),
        };
        let json = serde_json::to_string(&saved).unwrap();
        let saved: WorkspaceFile = serde_json::from_str(&json).unwrap();
        assert!(saved.worksheets[0].window.detached);
        let loaded = load_workspace(&saved, &mut shared);
        let saved = save_workspace(&[(loaded[0].0.clone(), &loaded[0].1)], 0, &shared);
        assert_eq!(saved.worksheets[0].panels.len(), 8);
        let topology: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(saved.worksheets[0].dock.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        let original: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(topology, original["worksheets"][0]["dock"]);
        let mut slots = 0;
        for config in &saved.worksheets[0].panels {
            match config {
                PanelConfig::Graph(g) => slots += g.channels.len(),
                PanelConfig::Histogram(h) => {
                    slots += h.channels.len() + usize::from(h.color_channel.is_some());
                    assert_eq!(h.bin_settings.width, Some(100.));
                    assert_eq!(h.band_presentation.hidden, [3]);
                }
                PanelConfig::Scatter(s) => {
                    slots += usize::from(s.x_channel.is_some())
                        + usize::from(s.y_channel.is_some())
                        + usize::from(s.color_channel.is_some());
                    assert!(s.density);
                    assert!(s.show_interval_averages);
                    assert_eq!((s.interval_width, s.interval_origin), (250., 50.));
                }
                PanelConfig::MixtureMap(m) => {
                    slots += usize::from(m.x_channel.is_some())
                        + usize::from(m.y_channel.is_some())
                        + usize::from(m.value_channel.is_some())
                }
                PanelConfig::Fft(f) => slots += f.channels.len(),
                PanelConfig::Calculations(c) => {
                    slots += c.channels.len();
                    assert_eq!(c.interval_seconds, 0.25);
                    assert_eq!(c.suspension_bins, 51);
                    assert!(c.suspension_line);
                    assert_eq!(
                        c.suspension_output,
                        crate::panels::calculations::DistributionOutput::Percent
                    );
                }
                PanelConfig::Report(r) => slots += r.channels.len(),
                PanelConfig::TrackMap(t) => {
                    assert_eq!(t.color_channel_name.as_deref(), Some("Missing GPS colour"))
                }
                _ => {}
            }
        }
        assert_eq!(slots, 12);
    }
}

fn default_interval_width() -> f64 {
    100.
}

fn default_velocity_bins() -> usize {
    41
}

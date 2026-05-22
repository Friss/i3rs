//! Workspace save/load: serializes panel layout and channel configuration.

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use i3rs_core::Sector;

use crate::panels::PanelTab;
use crate::panels::fft::FftPanel;
use crate::panels::gauge::{GaugePanel, GaugeStyle};
use crate::panels::graph::{EmbeddedTrack, GraphPanel, OverlaySource};
use crate::panels::histogram::HistogramPanel;
use crate::panels::mixture_map::MixtureMapPanel;
use crate::panels::motorcycle_chassis::MotorcycleChassisPanel;
use crate::panels::scatter::ScatterPanel;
use crate::panels::track_map::TrackMapPanel;
use crate::panels::utils::{apply_channel_preferences, create_plotted_channel};
use crate::state::{
    CHANNEL_COLORS, ChannelId, GraphMode, GraphXAxis, SharedState, compute_channel_stats,
};

// ---------------------------------------------------------------------------
// Serializable workspace types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub worksheets: Vec<WorksheetConfig>,
    pub active_worksheet: usize,
    pub last_file_path: Option<String>,
    #[serde(default)]
    pub math_channels: Vec<MathChannelConfig>,
    #[serde(default)]
    pub channel_aliases: Vec<ChannelAliasConfig>,
    #[serde(default)]
    pub sectors: Vec<Sector>,
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
    pub name: String,
    pub panels: Vec<PanelConfig>,
}

#[derive(Serialize, Deserialize)]
pub enum PanelConfig {
    Graph(GraphPanelConfig),
    TrackMap(TrackMapPanelConfig),
    MotorcycleChassis(MotorcycleChassisPanelConfig),
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
pub struct GraphPanelConfig {
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
    #[serde(default)]
    pub embedded_track: Option<GraphEmbeddedTrackConfig>,
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

fn gauge_style_to_string(style: GaugeStyle) -> &'static str {
    match style {
        GaugeStyle::Analog => "Analog",
        GaugeStyle::Bar => "Bar",
        GaugeStyle::Digital => "Digital",
        GaugeStyle::SteeringWheel => "SteeringWheel",
    }
}

fn gauge_style_from_string(style: &str) -> GaugeStyle {
    match style {
        "Bar" => GaugeStyle::Bar,
        "Digital" => GaugeStyle::Digital,
        "SteeringWheel" => GaugeStyle::SteeringWheel,
        _ => GaugeStyle::Analog,
    }
}

#[derive(Serialize, Deserialize)]
pub struct GraphOverlayConfig {
    pub file_path: Option<String>,
    pub lap_index: usize,
    pub manual_offset: f64,
    pub stretch_to_reference: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GraphGaugeConfig {
    pub channel_name: String,
    pub is_math: bool,
    pub style: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GraphEmbeddedTrackConfig {
    pub color_channel_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GraphDisplayTransformConfig {
    pub scale: f64,
    pub offset: f64,
    pub unit: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ReportPanelConfig {
    pub id: u64,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
pub struct TrackMapPanelConfig {
    pub id: u64,
    pub title: String,
    pub color_channel_name: Option<String>,
}

/// Workspace configuration for the Motorcycle Chassis panel.
#[derive(Serialize, Deserialize)]
pub struct MotorcycleChassisPanelConfig {
    pub id: u64,
    pub title: String,
    /// Path to the MotoSPEC MS1/MS3 chassis definition file, if one was loaded.
    #[serde(default)]
    pub motospec_path: Option<String>,
    /// Setup column to load from the chassis file (1, 2, or 3).
    #[serde(default = "default_motospec_column")]
    pub motospec_column: u8,
    /// MoTeC channel name for the rear suspension pot.
    #[serde(default = "default_rr_pot_channel")]
    pub rr_pot_channel: String,
    /// MoTeC channel name for the front suspension pot.
    #[serde(default = "default_fr_pot_channel")]
    pub fr_pot_channel: String,
    /// MoTeC channel name for lean angle.
    #[serde(default = "default_lean_channel")]
    pub lean_channel: String,
}

fn default_motospec_column() -> u8 { 1 }
fn default_rr_pot_channel() -> String { "s_susp_rr".into() }
fn default_fr_pot_channel() -> String { "s_susp_fr".into() }
fn default_lean_channel() -> String { "phi_lean".into() }

#[derive(Serialize, Deserialize)]
pub struct HistogramPanelConfig {
    pub id: u64,
    pub title: String,
    pub bin_count: usize,
    pub per_lap: bool,
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
    pub id: u64,
    pub title: String,
    pub log_scale: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GaugePanelConfig {
    pub id: u64,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
pub struct MixtureMapPanelConfig {
    pub id: u64,
    pub title: String,
    pub bins: usize,
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
    pub name: String,
    pub expression: String,
    pub unit: String,
    pub dec_places: i16,
}

fn channel_name_and_kind(channel_id: ChannelId, shared: &SharedState) -> Option<(String, bool)> {
    match channel_id {
        ChannelId::Physical(idx) => {
            Some((shared.ld_file.as_ref()?.channels[idx].name.clone(), false))
        }
        ChannelId::Math(idx) => Some((shared.math_channels.get(idx)?.name.clone(), true)),
    }
}

fn resolve_saved_plotted_channel(
    shared: &SharedState,
    channel_name: &str,
    is_math: bool,
    color: eframe::egui::Color32,
    tile_group: usize,
) -> Option<crate::state::PlottedChannel> {
    if is_math {
        let idx = shared
            .math_channels
            .iter()
            .position(|mc| mc.name == channel_name)?;
        let data = shared.math_channels.get(idx)?.data.clone()?;
        let stats = compute_channel_stats(&data);
        let mut plotted = crate::state::PlottedChannel {
            channel_id: ChannelId::Math(idx),
            color,
            data,
            tile_group,
            y_axis: crate::state::YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            cached_min: stats.min,
            cached_max: stats.max,
            cached_avg: stats.avg,
        };
        apply_channel_preferences(&mut plotted, shared);
        Some(plotted)
    } else {
        let channel_idx = shared
            .ld_file
            .as_ref()?
            .channels
            .iter()
            .find(|channel| channel.name == channel_name)?
            .index;
        let mut plotted =
            create_plotted_channel(ChannelId::Physical(channel_idx), shared, tile_group)?;
        plotted.color = color;
        plotted.tile_group = tile_group;
        plotted.y_axis = crate::state::YAxis::Left;
        apply_channel_preferences(&mut plotted, shared);
        Some(plotted)
    }
}

// ---------------------------------------------------------------------------
// Conversion: App state → WorkspaceFile
// ---------------------------------------------------------------------------

pub fn save_workspace(
    worksheets: &[(String, &DockState<PanelTab>)],
    active_worksheet: usize,
    shared: &SharedState,
) -> WorkspaceFile {
    let ws_configs: Vec<WorksheetConfig> = worksheets
        .iter()
        .map(|(name, dock)| {
            let mut panels = Vec::new();
            for (_path, tab) in dock.iter_all_tabs() {
                let config = match tab {
                    PanelTab::Graph(g) => {
                        let mut channel_names = Vec::new();
                        let mut is_math = Vec::new();
                        for pc in &g.plotted_channels {
                            match pc.channel_id {
                                ChannelId::Physical(idx) => {
                                    if let Some(ld) = &shared.ld_file {
                                        channel_names.push(ld.channels[idx].name.clone());
                                    }
                                    is_math.push(false);
                                }
                                ChannelId::Math(idx) => {
                                    if let Some(mc) = shared.math_channels.get(idx) {
                                        channel_names.push(mc.name.clone());
                                    }
                                    is_math.push(true);
                                }
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
                            .map(|pc| GraphDisplayTransformConfig {
                                scale: pc.display_scale,
                                offset: pc.display_offset,
                                unit: pc.display_unit.clone(),
                            })
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
                                let (channel_name, is_math) = match gauge.channel.channel_id {
                                    ChannelId::Physical(idx) => {
                                        (shared.ld_file.as_ref()?.channels[idx].name.clone(), false)
                                    }
                                    ChannelId::Math(idx) => {
                                        (shared.math_channels.get(idx)?.name.clone(), true)
                                    }
                                };
                                Some(GraphGaugeConfig {
                                    channel_name,
                                    is_math,
                                    style: gauge_style_to_string(gauge.style).into(),
                                })
                            })
                            .collect();
                        let embedded_track = g.embedded_track.as_ref().map(|track| {
                            GraphEmbeddedTrackConfig {
                                color_channel_name: track.color_channel_name.clone(),
                            }
                        });
                        PanelConfig::Graph(GraphPanelConfig {
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
                            embedded_track,
                            embedded_gauge_height: g.embedded_gauge_height,
                            is_math,
                            display_transforms,
                            tile_heights: g.tile_heights.clone(),
                        })
                    }
                    PanelTab::TrackMap(t) => {
                        let color_channel_name = t.color_channel_idx().and_then(|idx| {
                            shared
                                .ld_file
                                .as_ref()
                                .and_then(|ld| ld.channels.get(idx).map(|ch| ch.name.clone()))
                        });
                        PanelConfig::TrackMap(TrackMapPanelConfig {
                            id: t.id,
                            title: t.title.clone(),
                            color_channel_name,
                        })
                    }
                    PanelTab::MotorcycleChassis(c) => {
                        PanelConfig::MotorcycleChassis(MotorcycleChassisPanelConfig {
                            id: c.id,
                            title: c.title.clone(),
                            motospec_path: c.motospec_path.as_ref()
                                .map(|p| p.to_string_lossy().into_owned()),
                            motospec_column: c.motospec_column,
                            rr_pot_channel: c.rr_pot_channel.clone(),
                            fr_pot_channel: c.fr_pot_channel.clone(),
                            lean_channel: c.lean_channel.clone(),
                        })
                    }
                    PanelTab::ChannelBrowser => PanelConfig::ChannelBrowser,
                    PanelTab::CursorReadout => PanelConfig::CursorReadout,
                    PanelTab::Report(r) => PanelConfig::Report(ReportPanelConfig {
                        id: r.id,
                        title: r.title.clone(),
                    }),
                    PanelTab::Histogram(h) => PanelConfig::Histogram(HistogramPanelConfig {
                        id: h.id,
                        title: h.title.clone(),
                        bin_count: h.bin_count,
                        per_lap: h.per_lap,
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
                                channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                            })
                            .collect(),
                        is_math: h
                            .channels
                            .iter()
                            .filter_map(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
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
                        id: s.id,
                        title: s.title.clone(),
                        x_channel_name: s.x_channel.as_ref().and_then(|pc| {
                            channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                        }),
                        y_channel_name: s.y_channel.as_ref().and_then(|pc| {
                            channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                        }),
                        x_is_math: s
                            .x_channel
                            .as_ref()
                            .and_then(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        y_is_math: s
                            .y_channel
                            .as_ref()
                            .and_then(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
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
                        id: f.id,
                        title: f.title.clone(),
                        log_scale: f.log_scale,
                    }),
                    PanelTab::Gauge(g) => PanelConfig::Gauge(GaugePanelConfig {
                        id: g.id,
                        title: g.title.clone(),
                    }),
                    PanelTab::MixtureMap(m) => PanelConfig::MixtureMap(MixtureMapPanelConfig {
                        id: m.id,
                        title: m.title.clone(),
                        bins: m.bins,
                        x_channel_name: m.x_channel.as_ref().and_then(|pc| {
                            channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                        }),
                        y_channel_name: m.y_channel.as_ref().and_then(|pc| {
                            channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                        }),
                        value_channel_name: m.value_channel.as_ref().and_then(|pc| {
                            channel_name_and_kind(pc.channel_id, shared).map(|(name, _)| name)
                        }),
                        x_is_math: m
                            .x_channel
                            .as_ref()
                            .and_then(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        y_is_math: m
                            .y_channel
                            .as_ref()
                            .and_then(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                        value_is_math: m
                            .value_channel
                            .as_ref()
                            .and_then(|pc| {
                                channel_name_and_kind(pc.channel_id, shared)
                                    .map(|(_, is_math)| is_math)
                            })
                            .unwrap_or(false),
                    }),
                };
                panels.push(config);
            }
            WorksheetConfig {
                name: name.clone(),
                panels,
            }
        })
        .collect();

    let math_channels: Vec<MathChannelConfig> = shared
        .math_channels
        .iter()
        .map(|mc| MathChannelConfig {
            name: mc.name.clone(),
            expression: mc.expression.clone(),
            unit: mc.unit.clone(),
            dec_places: mc.dec_places,
        })
        .collect();

    let channel_aliases: Vec<ChannelAliasConfig> = shared
        .channel_aliases
        .iter()
        .map(|(alias, target)| ChannelAliasConfig {
            alias: alias.clone(),
            target: target.clone(),
        })
        .collect();

    let sectors: Vec<Sector> = shared.sectors.clone();

    WorkspaceFile {
        worksheets: ws_configs,
        active_worksheet,
        last_file_path: shared
            .ld_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        math_channels,
        channel_aliases,
        sectors,
        reference_lap: shared.reference_lap,
    }
}

// ---------------------------------------------------------------------------
// Conversion: WorkspaceFile → App state
// ---------------------------------------------------------------------------

pub fn load_workspace(
    workspace: &WorkspaceFile,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    workspace
        .worksheets
        .iter()
        .enumerate()
        .map(|(worksheet_idx, ws_config)| {
            let tabs: Vec<PanelTab> = ws_config
                .panels
                .iter()
                .map(|panel| match panel {
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
                        graph.reference_lap = gc.reference_lap;
                        graph.embedded_gauge_height = gc.embedded_gauge_height;
                        graph.tile_heights = gc.tile_heights.clone();

                        // Resolve channels by name
                        for (i, name) in gc.channel_names.iter().enumerate() {
                            let is_math = gc.is_math.get(i).copied().unwrap_or(false);
                            let color = gc
                                .colors
                                .get(i)
                                .map(|c| eframe::egui::Color32::from_rgb(c[0], c[1], c[2]))
                                .unwrap_or(CHANNEL_COLORS[i % CHANNEL_COLORS.len()]);
                            let tile_group = gc.tile_groups.get(i).copied().unwrap_or(i);
                            let display_transform = gc.display_transforms.get(i);
                            let display_scale =
                                display_transform.map(|cfg| cfg.scale).unwrap_or(1.0);
                            let display_offset =
                                display_transform.map(|cfg| cfg.offset).unwrap_or(0.0);
                            let display_unit = display_transform.and_then(|cfg| cfg.unit.clone());

                            if is_math {
                                // Find math channel by name
                                if let Some(mc_idx) =
                                    shared.math_channels.iter().position(|mc| mc.name == *name)
                                    && let Some(data) = &shared.math_channels[mc_idx].data
                                {
                                    let stats = compute_channel_stats(data);
                                    graph.plotted_channels.push(crate::state::PlottedChannel {
                                        channel_id: ChannelId::Math(mc_idx),
                                        color,
                                        data: data.clone(),
                                        tile_group,
                                        y_axis: crate::state::YAxis::Left,
                                        display_scale,
                                        display_offset,
                                        display_unit: display_unit.clone(),
                                        cached_min: stats.min,
                                        cached_max: stats.max,
                                        cached_avg: stats.avg,
                                    });
                                }
                            } else if let Some(ld) = &shared.ld_file
                                && let Some(ch) = ld.channels.iter().find(|c| &c.name == name)
                                && let Some(mut plotted) = create_plotted_channel(
                                    ChannelId::Physical(ch.index),
                                    shared,
                                    tile_group,
                                )
                            {
                                plotted.color = color;
                                plotted.tile_group = tile_group;
                                plotted.y_axis = crate::state::YAxis::Left;
                                plotted.display_scale = display_scale;
                                plotted.display_offset = display_offset;
                                plotted.display_unit = display_unit.clone();
                                graph.plotted_channels.push(plotted);
                            }
                        }

                        for gauge in &gc.embedded_gauges {
                            let channel_id = if gauge.is_math {
                                shared
                                    .math_channels
                                    .iter()
                                    .position(|mc| mc.name == gauge.channel_name)
                                    .map(ChannelId::Math)
                            } else {
                                shared.ld_file.as_ref().and_then(|ld| {
                                    ld.channels
                                        .iter()
                                        .find(|channel| channel.name == gauge.channel_name)
                                        .map(|channel| ChannelId::Physical(channel.index))
                                })
                            };
                            if let Some(channel_id) = channel_id {
                                graph.add_embedded_gauge_with_style(
                                    channel_id,
                                    shared,
                                    gauge_style_from_string(&gauge.style),
                                );
                            }
                        }

                        if let Some(track_cfg) = &gc.embedded_track {
                            graph.embedded_track = Some(EmbeddedTrack::new(
                                track_cfg.color_channel_name.clone(),
                                shared,
                            ));
                        }

                        for overlay in &gc.overlays {
                            let source = if let Some(path) = &overlay.file_path {
                                let path = std::path::PathBuf::from(path);
                                graph
                                    .load_external_overlay_path(path)
                                    .map(OverlaySource::External)
                            } else {
                                Some(OverlaySource::MainSession)
                            };
                            if let Some(source) = source {
                                graph.lap_overlays.push(crate::panels::graph::LapOverlay {
                                    source,
                                    lap_idx: overlay.lap_index,
                                    manual_offset: overlay.manual_offset,
                                    stretch_to_reference: overlay.stretch_to_reference,
                                });
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
                        track_map.home_worksheet = worksheet_idx;
                        // Resolve color channel by name
                        if let Some(ref color_name) = tc.color_channel_name
                            && let Some(ld) = &shared.ld_file
                            && let Some(idx) =
                                ld.channels.iter().position(|ch| &ch.name == color_name)
                        {
                            track_map.set_color_channel_idx(Some(idx));
                        }
                        if tc.id >= shared.next_panel_id {
                            shared.next_panel_id = tc.id + 1;
                        }
                        PanelTab::TrackMap(track_map)
                    }
                    PanelConfig::MotorcycleChassis(cc) => {
                        let mut panel = MotorcycleChassisPanel::new(cc.id, &cc.title);
                        panel.home_worksheet = worksheet_idx;
                        panel.rr_pot_channel = cc.rr_pot_channel.clone();
                        panel.fr_pot_channel = cc.fr_pot_channel.clone();
                        panel.lean_channel = cc.lean_channel.clone();
                        // Reload chassis file if the path is still accessible
                        if let Some(ref path_str) = cc.motospec_path {
                            let path = std::path::PathBuf::from(path_str);
                            if path.exists() {
                                panel.load_motospec(path, cc.motospec_column);
                            }
                        }
                        if cc.id >= shared.next_panel_id {
                            shared.next_panel_id = cc.id + 1;
                        }
                        PanelTab::MotorcycleChassis(panel)
                    }
                    PanelConfig::ChannelBrowser => PanelTab::ChannelBrowser,
                    PanelConfig::CursorReadout => PanelTab::CursorReadout,
                    PanelConfig::Report(rc) => {
                        let report = crate::panels::report::ReportPanel::new(rc.id, &rc.title);
                        if rc.id >= shared.next_panel_id {
                            shared.next_panel_id = rc.id + 1;
                        }
                        PanelTab::Report(report)
                    }
                    PanelConfig::Histogram(hc) => {
                        let mut histogram = HistogramPanel::new(hc.id, &hc.title);
                        histogram.bin_count = hc.bin_count;
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
                        if hc.id >= shared.next_panel_id {
                            shared.next_panel_id = hc.id + 1;
                        }
                        PanelTab::Histogram(histogram)
                    }
                    PanelConfig::Scatter(sc) => {
                        let mut scatter = ScatterPanel::new(sc.id, &sc.title);
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
                        if sc.id >= shared.next_panel_id {
                            shared.next_panel_id = sc.id + 1;
                        }
                        PanelTab::Scatter(scatter)
                    }
                    PanelConfig::Fft(fc) => {
                        let mut fft = FftPanel::new(fc.id, &fc.title);
                        fft.log_scale = fc.log_scale;
                        if fc.id >= shared.next_panel_id {
                            shared.next_panel_id = fc.id + 1;
                        }
                        PanelTab::Fft(fft)
                    }
                    PanelConfig::Gauge(gc) => {
                        let gauge = GaugePanel::new(gc.id, &gc.title);
                        if gc.id >= shared.next_panel_id {
                            shared.next_panel_id = gc.id + 1;
                        }
                        PanelTab::Gauge(gauge)
                    }
                    PanelConfig::MixtureMap(mc) => {
                        let mut mixture_map = MixtureMapPanel::new(mc.id, &mc.title);
                        mixture_map.bins = mc.bins;
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
                        if mc.id >= shared.next_panel_id {
                            shared.next_panel_id = mc.id + 1;
                        }
                        PanelTab::MixtureMap(mixture_map)
                    }
                })
                .collect();

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

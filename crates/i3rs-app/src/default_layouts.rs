//! Default worksheet layouts matching MoTeC i2 Pro's tab structure.
//!
//! When a `.ld` file is opened and no workspace is loaded, we auto-create
//! worksheets with channels matched by name from the loaded file.

use std::sync::Arc;

use eframe::egui;
use egui_dock::{DockState, NodeIndex};
use i3rs_core::LdFile;

use crate::panels::PanelTab;
use crate::panels::gauge::GaugeStyle;
use crate::panels::graph::GraphPanel;
use crate::panels::histogram::HistogramPanel;
use crate::panels::mixture_map::MixtureMapPanel;
use crate::panels::scatter::ScatterPanel;
use crate::state::{
    CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState, YAxis, compute_channel_stats,
};

/// A graph-centric default worksheet.
struct GraphWorksheetTemplate {
    name: &'static str,
    channels: &'static [&'static str],
    tiled_groups: &'static [&'static [&'static str]],
    embedded_gauges: &'static [GaugeTemplate],
}

struct GaugeTemplate {
    names: &'static [&'static str],
    style: GaugeStyle,
}

const DRIVER_WORKSHEET: GraphWorksheetTemplate = GraphWorksheetTemplate {
    name: "Driver",
    channels: &[
        "Engine Speed",
        "Engine Speed Limit",
        "Throttle Pedal",
        "Brake State",
        "Clutch Position",
        "G Force Lat",
        "CP Lotus ESP Lateral Acceleration",
        "Gear",
        "Inlet Air Temperature",
        "Coolant Temperature",
        "Engine Oil Temperature",
    ],
    tiled_groups: &[
        &["Engine Speed", "Engine Speed Limit"],
        &["Throttle Pedal"],
        &["Brake State", "Clutch Position"],
        &["G Force Lat", "CP Lotus ESP Lateral Acceleration"],
        &["Gear"],
        &["Inlet Air Temperature"],
        &["Coolant Temperature", "Engine Oil Temperature"],
    ],
    embedded_gauges: &[
        GaugeTemplate {
            names: &["Engine Speed"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Corr Speed", "Vehicle Speed", "GPS Speed"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Throttle Position", "Throttle Pedal"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["CP Lotus ESP Steering Angle", "Steering Angle"],
            style: GaugeStyle::SteeringWheel,
        },
        GaugeTemplate {
            names: &["G Force Lat", "CP Lotus ESP Lateral Acceleration"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Gear"],
            style: GaugeStyle::Digital,
        },
    ],
};

const BRAKING_WORKSHEET: GraphWorksheetTemplate = GraphWorksheetTemplate {
    name: "Braking",
    channels: &[
        "Engine Speed",
        "Corr Speed",
        "Vehicle Speed",
        "GPS Speed",
        "G Force Lat",
        "Throttle Position",
        "Throttle Pedal",
        "Brake State",
        "CP Lotus Wheel Speed Front Left",
        "CP Lotus Wheel Speed Front Right",
        "CP Lotus Wheel Speed Rear Left",
        "CP Lotus Wheel Speed Rear Right",
        "Front Wheels Diff",
        "Rear Wheel Diff",
        "CP Lotus ESP Steering Angle",
    ],
    tiled_groups: &[
        &["Engine Speed"],
        &["Corr Speed", "Vehicle Speed", "GPS Speed"],
        &["G Force Lat"],
        &["Throttle Position", "Throttle Pedal"],
        &["Brake State"],
        &[
            "CP Lotus Wheel Speed Front Left",
            "CP Lotus Wheel Speed Front Right",
        ],
        &[
            "CP Lotus Wheel Speed Rear Left",
            "CP Lotus Wheel Speed Rear Right",
        ],
        &["Front Wheels Diff", "Rear Wheel Diff"],
        &["CP Lotus ESP Steering Angle"],
    ],
    embedded_gauges: &[],
};

const ENGINE_WORKSHEET: GraphWorksheetTemplate = GraphWorksheetTemplate {
    name: "Engine",
    channels: &[
        "Engine Speed",
        "Inlet Air Temperature",
        "Coolant Temperature",
        "Engine Oil Temperature",
        "CP Lotus Gauge Pack Ambient Air Temperature",
        "CP Lotus Gauge Pack Ambient Air",
        "Engine Oil Pressure",
        "Fuel Pressure",
        "ECU Battery Voltage",
        "Warning Source",
    ],
    tiled_groups: &[
        &["Engine Speed"],
        &[
            "Inlet Air Temperature",
            "Coolant Temperature",
            "Engine Oil Temperature",
            "CP Lotus Gauge Pack Ambient Air Temperature",
            "CP Lotus Gauge Pack Ambient Air",
        ],
        &["Engine Oil Pressure", "Fuel Pressure"],
        &["ECU Battery Voltage"],
        &["Warning Source"],
    ],
    embedded_gauges: &[
        GaugeTemplate {
            names: &["Engine Speed"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Inlet Air Temperature"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Coolant Temperature"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Engine Oil Temperature"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Engine Oil Pressure"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Fuel Pressure"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["ECU Battery Voltage"],
            style: GaugeStyle::Bar,
        },
    ],
};

const FUEL_IGN_WORKSHEET: GraphWorksheetTemplate = GraphWorksheetTemplate {
    name: "Fuel / Ign",
    channels: &[
        "Engine Speed",
        "Throttle Position",
        "Throttle Pedal",
        "Inlet Manifold Pressure",
        "Fuel Pressure",
        "Fuel Injector Primary Duty Cycle",
        "Exhaust Lambda Bank 1",
        "Exhaust Lambda Bank 2",
        "Exhaust Lambda",
        "Ignition Timing",
        "Ignition Timing Compensation",
        "Exhaust Camshaft Aim",
        "Exhaust Camshaft Bank 1 Position",
        "Exhaust Camshaft Bank 2 Position",
        "Inlet Camshaft Aim",
        "Inlet Camshaft Bank 1 Position",
        "Inlet Camshaft Bank 2 Position",
    ],
    tiled_groups: &[
        &["Engine Speed"],
        &["Throttle Position", "Throttle Pedal"],
        &["Inlet Manifold Pressure"],
        &["Fuel Pressure"],
        &["Fuel Injector Primary Duty Cycle"],
        &[
            "Exhaust Lambda Bank 1",
            "Exhaust Lambda Bank 2",
            "Exhaust Lambda",
        ],
        &["Ignition Timing", "Ignition Timing Compensation"],
        &[
            "Exhaust Camshaft Aim",
            "Exhaust Camshaft Bank 1 Position",
            "Exhaust Camshaft Bank 2 Position",
        ],
        &[
            "Inlet Camshaft Aim",
            "Inlet Camshaft Bank 1 Position",
            "Inlet Camshaft Bank 2 Position",
        ],
    ],
    embedded_gauges: &[
        GaugeTemplate {
            names: &["Engine Speed"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Throttle Position", "Throttle Pedal"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Inlet Manifold Pressure"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["Fuel Pressure"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Fuel Injector Primary Duty Cycle"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Exhaust Lambda Bank 1"],
            style: GaugeStyle::Digital,
        },
        GaugeTemplate {
            names: &["Exhaust Lambda Bank 2"],
            style: GaugeStyle::Digital,
        },
        GaugeTemplate {
            names: &["Ignition Timing"],
            style: GaugeStyle::Bar,
        },
    ],
};

const SPARE_WORKSHEET: GraphWorksheetTemplate = GraphWorksheetTemplate {
    name: "Spare",
    channels: &[
        "GPS Speed",
        "Corr Speed",
        "Vehicle Speed",
        "Torque Control Cut",
        "Throttle Pedal",
        "CP Engine Speed Targeting Throttle Control Aim",
        "CP Engine Speed Targeting Throttle Control Forced Forward",
        "Torque Limit State",
        "Engine Speed Limit State",
        "ECU Acceleration X",
        "ECU Acceleration Y",
        "CP Lotus ESP Lateral Acceleration",
        "CP Engine Speed Targeting Target",
        "CP Rev Match Engine Speed Target",
    ],
    tiled_groups: &[
        &["GPS Speed", "Corr Speed", "Vehicle Speed"],
        &[
            "Torque Control Cut",
            "Throttle Pedal",
            "CP Engine Speed Targeting Throttle Control Aim",
            "CP Engine Speed Targeting Throttle Control Forced Forward",
        ],
        &["Torque Limit State", "Engine Speed Limit State"],
        &[
            "ECU Acceleration X",
            "ECU Acceleration Y",
            "CP Lotus ESP Lateral Acceleration",
        ],
        &[
            "CP Engine Speed Targeting Target",
            "CP Rev Match Engine Speed Target",
        ],
    ],
    embedded_gauges: &[],
};

fn channel_name_matches(actual: &str, candidate: &str) -> bool {
    actual.eq_ignore_ascii_case(candidate)
        || actual
            .get(..candidate.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(candidate))
        || candidate
            .get(..actual.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(actual))
}

/// Find a channel index by name (case-insensitive, partial match with prefix).
fn find_channel(ld: &LdFile, name: &str) -> Option<usize> {
    if let Some(idx) = ld
        .channels
        .iter()
        .position(|ch| ch.name.eq_ignore_ascii_case(name))
    {
        return Some(idx);
    }

    ld.channels
        .iter()
        .position(|ch| channel_name_matches(&ch.name, name))
}

fn find_first_channel(ld: &LdFile, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| find_channel(ld, name))
}

fn next_panel_id(shared: &mut SharedState) -> u64 {
    let id = shared.next_panel_id;
    shared.next_panel_id += 1;
    id
}

fn make_plotted_channel(
    ld: &LdFile,
    channel_idx: usize,
    color: egui::Color32,
    tile_group: usize,
) -> Option<PlottedChannel> {
    let channel = ld.channels.get(channel_idx)?;
    let data = ld.read_channel_data(channel)?;
    let (cached_min, cached_max, cached_avg, _) = compute_channel_stats(&data);
    Some(PlottedChannel {
        channel_id: ChannelId::Physical(channel_idx),
        color,
        data: Arc::new(data),
        tile_group,
        y_axis: YAxis::Left,
        display_scale: 1.0,
        display_offset: 0.0,
        display_unit: None,
        cached_min,
        cached_max,
        cached_avg,
    })
}

fn build_graph_panel(
    ld: &LdFile,
    shared: &mut SharedState,
    template: &GraphWorksheetTemplate,
) -> Option<GraphPanel> {
    let mut graph = GraphPanel::new(next_panel_id(shared), template.name);
    let mut matched = Vec::new();

    for (i, chan_name) in template.channels.iter().enumerate() {
        let Some(chan_idx) = find_channel(ld, chan_name) else {
            continue;
        };
        if matched.contains(&chan_idx) {
            continue;
        }
        matched.push(chan_idx);

        let channel_name = &ld.channels[chan_idx].name;
        let tile_group = template
            .tiled_groups
            .iter()
            .position(|group| {
                group
                    .iter()
                    .any(|candidate| channel_name_matches(channel_name, candidate))
            })
            .unwrap_or(i);
        if let Some(pc) = make_plotted_channel(
            ld,
            chan_idx,
            CHANNEL_COLORS[i % CHANNEL_COLORS.len()],
            tile_group,
        ) {
            graph.plotted_channels.push(pc);
        }
    }

    for gauge in template.embedded_gauges {
        if let Some(idx) = find_first_channel(ld, gauge.names) {
            graph.add_embedded_gauge_with_style(ChannelId::Physical(idx), shared, gauge.style);
        }
    }

    (!graph.plotted_channels.is_empty()).then_some(graph)
}

fn build_graph_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
    template: &GraphWorksheetTemplate,
) -> Option<(String, DockState<PanelTab>)> {
    let graph = build_graph_panel(ld, shared, template)?;
    Some((
        template.name.to_string(),
        DockState::new(vec![PanelTab::Graph(graph)]),
    ))
}

fn build_scatter_panel(
    ld: &LdFile,
    shared: &mut SharedState,
    title: &str,
    x_names: &[&str],
    y_names: &[&str],
    point_size: f32,
) -> Option<ScatterPanel> {
    let x_idx = find_first_channel(ld, x_names)?;
    let y_idx = find_first_channel(ld, y_names)?;
    let mut panel = ScatterPanel::new(next_panel_id(shared), title);
    panel.point_size = point_size;
    panel.x_channel = make_plotted_channel(ld, x_idx, CHANNEL_COLORS[0], 0);
    panel.y_channel = make_plotted_channel(ld, y_idx, CHANNEL_COLORS[4], 1);
    (panel.x_channel.is_some() && panel.y_channel.is_some()).then_some(panel)
}

#[allow(clippy::too_many_arguments)]
fn build_histogram_panel(
    ld: &LdFile,
    shared: &mut SharedState,
    title: &str,
    channel_names: &[&[&str]],
    bin_count: usize,
    default_x_range: Option<(f64, f64)>,
    default_y_range: Option<(f64, f64)>,
    default_y_headroom_pct: f64,
) -> Option<HistogramPanel> {
    let mut panel = HistogramPanel::new(next_panel_id(shared), title);
    panel.bin_count = bin_count;
    if let Some((x_min, x_max)) = default_x_range {
        panel.lock_x_range = true;
        panel.x_min = x_min;
        panel.x_max = x_max;
    }
    if let Some((y_min, _)) = default_y_range {
        panel.lock_y_range = true;
        panel.y_min = y_min;
        panel.y_headroom_pct = default_y_headroom_pct;
    }
    for (i, candidates) in channel_names.iter().enumerate() {
        if let Some(idx) = find_first_channel(ld, candidates)
            && let Some(pc) =
                make_plotted_channel(ld, idx, CHANNEL_COLORS[i % CHANNEL_COLORS.len()], i)
        {
            panel.channels.push(pc);
        }
    }
    (!panel.channels.is_empty()).then_some(panel)
}

fn build_mixture_map_panel(
    ld: &LdFile,
    shared: &mut SharedState,
    title: &str,
    x_names: &[&str],
    y_names: &[&str],
    value_names: &[&str],
    bins: usize,
) -> Option<MixtureMapPanel> {
    let x_idx = find_first_channel(ld, x_names)?;
    let y_idx = find_first_channel(ld, y_names)?;
    let value_idx = find_first_channel(ld, value_names)?;
    let mut panel = MixtureMapPanel::new(next_panel_id(shared), title);
    panel.bins = bins;
    panel.x_channel = make_plotted_channel(ld, x_idx, CHANNEL_COLORS[0], 0);
    panel.y_channel = make_plotted_channel(ld, y_idx, CHANNEL_COLORS[1], 1);
    panel.value_channel = make_plotted_channel(ld, value_idx, CHANNEL_COLORS[2], 2);
    (panel.x_channel.is_some() && panel.y_channel.is_some() && panel.value_channel.is_some())
        .then_some(panel)
}

fn build_mixture_map_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
) -> Option<(String, DockState<PanelTab>)> {
    let left = build_mixture_map_panel(
        ld,
        shared,
        "Lambda Bank 1",
        &["Engine Speed"],
        &["Exhaust Lambda Bank 1", "Exhaust Lambda"],
        &["Throttle Position", "Throttle Pedal"],
        32,
    );
    let right = build_mixture_map_panel(
        ld,
        shared,
        "Lambda Bank 2",
        &["Engine Speed"],
        &["Exhaust Lambda Bank 2", "Exhaust Lambda"],
        &["Throttle Position", "Throttle Pedal"],
        32,
    );

    match (left, right) {
        (None, None) => None,
        (Some(panel), None) | (None, Some(panel)) => Some((
            "Mixture Map".to_string(),
            DockState::new(vec![PanelTab::MixtureMap(panel)]),
        )),
        (Some(left_panel), Some(right_panel)) => {
            let mut dock = DockState::new(vec![PanelTab::MixtureMap(left_panel)]);
            dock.main_surface_mut().split_right(
                NodeIndex::root(),
                0.5,
                vec![PanelTab::MixtureMap(right_panel)],
            );
            Some(("Mixture Map".to_string(), dock))
        }
    }
}

fn build_oil_pressure_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
) -> Option<(String, DockState<PanelTab>)> {
    let scatter = build_scatter_panel(
        ld,
        shared,
        "Engine Oil Pressure",
        &["CP Lotus ESP Lateral Acceleration", "G Force Lat"],
        &["Engine Oil Pressure"],
        1.75,
    );
    let graph = build_graph_panel(
        ld,
        shared,
        &GraphWorksheetTemplate {
            name: "Engine Oil Pressure",
            channels: &["Engine Oil Pressure"],
            tiled_groups: &[],
            embedded_gauges: &[],
        },
    );

    match (scatter, graph) {
        (None, None) => None,
        (Some(scatter_panel), None) => Some((
            "Oil Pressure".to_string(),
            DockState::new(vec![PanelTab::Scatter(scatter_panel)]),
        )),
        (None, Some(graph_panel)) => Some((
            "Oil Pressure".to_string(),
            DockState::new(vec![PanelTab::Graph(graph_panel)]),
        )),
        (Some(scatter_panel), Some(graph_panel)) => {
            let mut dock = DockState::new(vec![PanelTab::Scatter(scatter_panel)]);
            dock.main_surface_mut().split_below(
                NodeIndex::root(),
                0.82,
                vec![PanelTab::Graph(graph_panel)],
            );
            Some(("Oil Pressure".to_string(), dock))
        }
    }
}

fn build_rpm_histo_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
) -> Option<(String, DockState<PanelTab>)> {
    let histogram = build_histogram_panel(
        ld,
        shared,
        "Engine Speed",
        &[&["Engine Speed"]],
        48,
        Some((0.0, 9000.0)),
        Some((0.0, 0.0)),
        10.0,
    );
    let graph = build_graph_panel(
        ld,
        shared,
        &GraphWorksheetTemplate {
            name: "Engine RPM",
            channels: &["Engine Speed"],
            tiled_groups: &[],
            embedded_gauges: &[],
        },
    );

    match (histogram, graph) {
        (None, None) => None,
        (Some(hist_panel), None) => Some((
            "RPM Histo".to_string(),
            DockState::new(vec![PanelTab::Histogram(hist_panel)]),
        )),
        (None, Some(graph_panel)) => Some((
            "RPM Histo".to_string(),
            DockState::new(vec![PanelTab::Graph(graph_panel)]),
        )),
        (Some(hist_panel), Some(graph_panel)) => {
            let mut dock = DockState::new(vec![PanelTab::Histogram(hist_panel)]);
            dock.main_surface_mut().split_below(
                NodeIndex::root(),
                0.78,
                vec![PanelTab::Graph(graph_panel)],
            );
            Some(("RPM Histo".to_string(), dock))
        }
    }
}

/// Create default worksheets from the loaded file's available channels.
pub fn create_default_worksheets(
    ld: &Arc<LdFile>,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    let mut worksheets = Vec::new();

    if let Some(sheet) = build_graph_worksheet(ld, shared, &DRIVER_WORKSHEET) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_graph_worksheet(ld, shared, &BRAKING_WORKSHEET) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_graph_worksheet(ld, shared, &ENGINE_WORKSHEET) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_graph_worksheet(ld, shared, &FUEL_IGN_WORKSHEET) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_mixture_map_worksheet(ld, shared) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_oil_pressure_worksheet(ld, shared) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_rpm_histo_worksheet(ld, shared) {
        worksheets.push(sheet);
    }
    if let Some(sheet) = build_graph_worksheet(ld, shared, &SPARE_WORKSHEET) {
        worksheets.push(sheet);
    }

    worksheets
}

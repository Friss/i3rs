//! Circuit worksheet composition from the audited configured i2 reference.
//!
//! When a `.ld` file is opened and no workspace is loaded, we auto-create
//! worksheets with channels matched by name from the loaded file.

use std::sync::Arc;

mod profiles;
pub use profiles::{WorkspaceProfile, create_profile_worksheets};

use eframe::egui;
use egui_dock::{DockState, NodeIndex};
use i3rs_core::LdFile;

use crate::panels::PanelTab;
use crate::panels::distribution::ColorBands;
use crate::panels::gauge::GaugeStyle;
use crate::panels::graph::GraphPanel;
use crate::panels::histogram::HistogramPanel;
use crate::panels::mixture_map::MixtureMapPanel;
use crate::panels::report::ReportPanel;
use crate::panels::scatter::ScatterPanel;
use crate::panels::track_map::TrackMapPanel;
use crate::panels::utils::create_plotted_channel;
use crate::state::{CHANNEL_COLORS, PlottedChannel, SharedState};

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
        "Gear",
        "Oversteer",
        "Oversteer (Speed Weighted)",
        "CP Lotus Gauge Pack Ambient Air Temperature",
        "Inlet Air Temperature",
        "Coolant Temperature",
        "Engine Oil Temperature",
    ],
    tiled_groups: &[
        &["Engine Speed", "Engine Speed Limit"],
        &["Throttle Pedal"],
        &["Brake State", "Clutch Position"],
        &["Oversteer", "Oversteer (Speed Weighted)"],
        &["G Force Lat"],
        &[
            "Gear",
            "Inlet Air Temperature",
            "CP Lotus Gauge Pack Ambient Air Temperature",
        ],
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
            names: &["Oversteer"],
            style: GaugeStyle::Analog,
        },
        GaugeTemplate {
            names: &["G Force Lat", "CP Lotus ESP Lateral Acceleration"],
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["G Force Long"],
            style: GaugeStyle::Bar,
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
        "Front Wheels Diff (Left - Right)",
        "Rear Wheel Diff (Left - Right)",
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
        &[
            "Front Wheels Diff (Left - Right)",
            "Rear Wheel Diff (Left - Right)",
        ],
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
            style: GaugeStyle::Bar,
        },
        GaugeTemplate {
            names: &["Exhaust Lambda Bank 2"],
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
        "CP Engine Speed Targeting Throttle Control Feed Forward",
        "Torque Limit State",
        "Engine Speed Limit State",
        "ECU Acceleration X",
        "ECU Acceleration Y",
        "ECU Acceleration Z",
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
            "CP Engine Speed Targeting Throttle Control Feed Forward",
        ],
        &["Torque Limit State", "Engine Speed Limit State"],
        &[
            "ECU Acceleration X",
            "ECU Acceleration Y",
            "ECU Acceleration Z",
            "CP Lotus ESP Lateral Acceleration",
        ],
        &[
            "CP Engine Speed Targeting Target",
            "CP Rev Match Engine Speed Target",
        ],
    ],
    embedded_gauges: &[],
};

/// Match the parser's exact/normalized names; alternative names use W02 aliases.
fn find_channel(ld: &LdFile, name: &str) -> Option<usize> {
    if let Some(channel) = ld.find_channel_by_name(name) {
        return Some(channel.index);
    }

    None
}

fn find_first_channel(ld: &LdFile, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| find_channel(ld, name))
}

fn next_panel_id(shared: &mut SharedState) -> u64 {
    let id = shared.next_panel_id;
    shared.next_panel_id += 1;
    id
}

fn named_slot(
    ld: &LdFile,
    shared: &mut SharedState,
    names: &[&str],
    color: egui::Color32,
    group: usize,
) -> Option<PlottedChannel> {
    let requested = *names.first()?;
    let found = find_first_channel(ld, names);
    let mut alternatives: Vec<String> = names.iter().map(|n| (*n).into()).collect();
    if let Some(index) = found
        && !alternatives.contains(&ld.channels[index].name)
    {
        alternatives.push(ld.channels[index].name.clone());
    }
    if alternatives.len() > 1 {
        shared
            .channel_settings
            .aliases
            .entry(requested.into())
            .or_insert(alternatives);
    }
    let id = shared
        .resolve_slot(requested, false)
        .unwrap_or_else(|| shared.unresolved_channel(requested, false));
    let mut channel = create_plotted_channel(id, shared, group)?;
    channel.requested_reference = Some((requested.into(), false));
    channel.color = color;
    channel.tile_group = group;
    Some(channel)
}

fn build_graph_panel(
    ld: &LdFile,
    shared: &mut SharedState,
    template: &GraphWorksheetTemplate,
) -> Option<GraphPanel> {
    let mut graph = GraphPanel::new(next_panel_id(shared), template.name);
    for gauge in template.embedded_gauges {
        if let Some(channel) = named_slot(
            ld,
            shared,
            gauge.names,
            CHANNEL_COLORS[graph.embedded_gauges.len() % CHANNEL_COLORS.len()],
            0,
        ) {
            graph
                .embedded_gauges
                .push(crate::panels::gauge::GaugeChannel {
                    last_unit: None,
                    channel,
                    style: gauge.style,
                    settings: crate::panels::gauge::GaugeSettings {
                        range: gauge.names[0].contains("Lambda").then_some((0.7, 1.1)),
                        ..Default::default()
                    },
                });
        }
    }

    for (i, name) in template.channels.iter().enumerate() {
        let group = template
            .tiled_groups
            .iter()
            .position(|members| members.contains(name))
            .unwrap_or(i);
        if let Some(channel) = named_slot(
            ld,
            shared,
            &[name],
            CHANNEL_COLORS[i % CHANNEL_COLORS.len()],
            group,
        ) {
            graph.plotted_channels.push(channel);
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
        if let Some(pc) = named_slot(
            ld,
            shared,
            candidates,
            CHANNEL_COLORS[i % CHANNEL_COLORS.len()],
            i,
        ) {
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
    let mut panel = MixtureMapPanel::new(next_panel_id(shared), title);
    panel.bins = bins;
    panel.scatter_mode = true;
    panel.sensor_delay = 0.15;
    panel.gear_filter.enabled = true;
    panel.minimum_y = Some(0.5);
    panel.x_channel = named_slot(ld, shared, x_names, CHANNEL_COLORS[0], 0);
    panel.y_channel = named_slot(ld, shared, y_names, CHANNEL_COLORS[1], 1);
    panel.value_channel = named_slot(ld, shared, value_names, CHANNEL_COLORS[2], 2);
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
            DockState::new(vec![PanelTab::MixtureMap(Box::new(panel))]),
        )),
        (Some(left_panel), Some(right_panel)) => {
            let mut dock = DockState::new(vec![PanelTab::MixtureMap(Box::new(left_panel))]);
            dock.main_surface_mut().split_right(
                NodeIndex::root(),
                0.5,
                vec![PanelTab::MixtureMap(Box::new(right_panel))],
            );
            Some(("Mixture Map".to_string(), dock))
        }
    }
}

fn build_oil_pressure_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
) -> Option<(String, DockState<PanelTab>)> {
    let mut panel = ScatterPanel::new(next_panel_id(shared), "Engine Oil Pressure");
    panel.point_size = 1.75;
    panel.bounds_padding_frac = 0.10;
    panel.lock_bounds = true;
    panel.x_channel = named_slot(
        ld,
        shared,
        &["G Force Lat", "CP Lotus ESP Lateral Acceleration"],
        CHANNEL_COLORS[0],
        0,
    );
    panel.y_channel = named_slot(
        ld,
        shared,
        &["Engine Oil Pressure", "Oil Pressure", "Eng Oil Pres"],
        CHANNEL_COLORS[4],
        1,
    );
    let scatter = Some(panel);
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
        (Some(panel), Some(graph)) => {
            let mut dock = DockState::new(vec![PanelTab::Scatter(Box::new(panel))]);
            dock.main_surface_mut().split_below(
                NodeIndex::root(),
                0.78,
                vec![PanelTab::Graph(graph)],
            );
            Some(("Oil Pressure".into(), dock))
        }
        (Some(panel), None) => Some((
            "Oil Pressure".into(),
            DockState::new(vec![PanelTab::Scatter(Box::new(panel))]),
        )),
        (None, Some(panel)) => Some((
            "Oil Pressure".to_string(),
            DockState::new(vec![PanelTab::Graph(panel)]),
        )),
        (None, None) => None,
    }
}

fn build_rpm_histo_worksheet(
    ld: &LdFile,
    shared: &mut SharedState,
) -> Option<(String, DockState<PanelTab>)> {
    let mut histogram = build_histogram_panel(
        ld,
        shared,
        "Engine Speed",
        &[&["Engine Speed"]],
        90,
        Some((0.0, 9000.0)),
        Some((0.0, 0.0)),
        10.0,
    );
    if let Some(panel) = &mut histogram {
        panel.percentage = true;
        panel.color_bands = ColorBands {
            min: 0.0,
            max: 100.0,
            count: 10,
            centered: false,
        };
        panel.color_channel = named_slot(
            ld,
            shared,
            &["Throttle Position", "Throttle Pedal"],
            CHANNEL_COLORS[2],
            0,
        );
    }
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
fn create_circuit_worksheets(
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
    {
        let mut map = TrackMapPanel::new(next_panel_id(shared), "Track Report");
        map.home_worksheet = worksheets.len();
        worksheets.push((
            "Track Report".into(),
            DockState::new(vec![PanelTab::TrackMap(map)]),
        ));
    }
    let mut report = ReportPanel::new(next_panel_id(shared), "Channel Report");
    for name in [
        "Engine Speed",
        "Vehicle Speed",
        "Throttle Position",
        "Engine Oil Pressure",
        "Fuel Pressure",
        "Coolant Temperature",
        "Engine Oil Temperature",
        "Exhaust Lambda Bank 1",
        "Exhaust Lambda Bank 2",
    ] {
        if let Some(pc) = named_slot(
            ld,
            shared,
            &[name],
            CHANNEL_COLORS[report.channels.len() % CHANNEL_COLORS.len()],
            0,
        ) {
            report.channels.push(pc);
        }
    }
    worksheets.push((
        "Channel Report".into(),
        DockState::new(vec![PanelTab::Report(report)]),
    ));
    {
        let mut sections = TrackMapPanel::new(next_panel_id(shared), "Section Times");
        sections.section_times_only = true;
        sections.home_worksheet = worksheets.len();
        worksheets.push((
            "Section Times".into(),
            DockState::new(vec![PanelTab::TrackMap(sections)]),
        ));
    }
    if let Some(sheet) = build_graph_worksheet(ld, shared, &SPARE_WORKSHEET) {
        worksheets.push(sheet);
    }

    worksheets
}

pub fn create_default_worksheets(
    ld: &Arc<LdFile>,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    create_profile_worksheets(shared.workspace_profile.unwrap_or_default(), ld, shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ChannelId;

    fn load_file(path: &str) -> (Arc<LdFile>, SharedState) {
        let ld = Arc::new(LdFile::open(path).unwrap());
        let mut shared = SharedState::new();
        shared.ld_file = Some(Arc::clone(&ld));
        shared.data_duration = Some(ld.duration_secs());
        (ld, shared)
    }

    #[test]
    fn rpm_defaults_are_independent_of_centered_mixture_bands() {
        let (ld, mut shared) = load_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ));
        let (_, dock) = build_rpm_histo_worksheet(&ld, &mut shared).unwrap();
        let histogram = dock
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::Histogram(h) => Some(h),
                _ => None,
            })
            .unwrap();
        assert_eq!(histogram.bin_count, 90);
        assert!(histogram.percentage && histogram.lock_x_range);
        assert_eq!((histogram.x_min, histogram.x_max), (0.0, 9000.0));
        assert_eq!(histogram.color_bands.count, 10);
        assert!(!histogram.color_bands.centered);
        assert!(histogram.color_channel.is_some());
        let (_, mixture_dock) = build_mixture_map_worksheet(&ld, &mut shared).unwrap();
        let mixture = mixture_dock
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::MixtureMap(m) => Some(m),
                _ => None,
            })
            .unwrap();
        assert_eq!(mixture.color_bands.count, 11);
        assert!(mixture.color_bands.centered);
    }

    #[test]
    fn spare_feed_forward_and_acceleration_z_resolve_and_use_correct_groups() {
        let mut ld = LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        // The audit recording lacks these configured channels. Supply their
        // names on real fixture metadata to exercise resolution and grouping.
        let names = [
            "CP Engine Speed Targeting Throttle Control Feed Forward",
            "ECU Acceleration X",
            "ECU Acceleration Y",
            "ECU Acceleration Z",
        ];
        for (channel, name) in ld.channels.iter_mut().zip(names) {
            channel.name = name.into();
        }
        let ld = Arc::new(ld);
        let mut shared = SharedState::new();
        shared.ld_file = Some(Arc::clone(&ld));
        let graph = build_graph_panel(&ld, &mut shared, &SPARE_WORKSHEET).unwrap();
        for (index, name) in names.into_iter().enumerate() {
            let pc = graph
                .plotted_channels
                .iter()
                .find(|pc| pc.channel_id == ChannelId::Physical(index))
                .unwrap_or_else(|| panic!("Spare is missing {name}"));
            assert_eq!(pc.tile_group, if index == 0 { 1 } else { 3 });
        }
    }

    #[test]
    #[ignore = "requires the local May 8 S1 reference log"]
    fn spare_reference_log_missing_channels_are_not_misbound() {
        let (ld, mut shared) = load_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        ));
        let graph = build_graph_panel(&ld, &mut shared, &SPARE_WORKSHEET).unwrap();
        for name in [
            "CP Engine Speed Targeting Throttle Control Feed Forward",
            "ECU Acceleration X",
            "ECU Acceleration Y",
            "ECU Acceleration Z",
        ] {
            assert!(
                find_channel(&ld, name).is_none(),
                "unexpected match for {name}"
            );
        }
        let lateral = ld
            .find_channel_by_name("CP Lotus ESP Lateral Acceleration")
            .unwrap();
        assert!(graph.plotted_channels.iter().any(|pc|
            pc.channel_id == ChannelId::Physical(lateral.index) && pc.tile_group == 3
        ));
    }
}

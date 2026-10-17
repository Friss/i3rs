//! Audited reference compositions; every requested slot survives absent inputs.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceProfile {
    #[default]
    Circuit,
    Drag,
    Engine,
    Rally,
}
impl WorkspaceProfile {
    pub const ALL: [Self; 4] = [Self::Circuit, Self::Drag, Self::Engine, Self::Rally];
    pub fn label(self) -> &'static str {
        match self {
            Self::Circuit => "Circuit (configured reference)",
            Self::Engine => "Engine (configured reference)",
            Self::Drag => "Drag",
            Self::Rally => "Rally",
        }
    }
}

fn aliases(shared: &mut SharedState) {
    for names in [
        &["Engine Speed", "Engine RPM", "RPM"][..],
        &["Throttle Position", "Throttle Pos", "Throttle Pedal"],
        &[
            "Inlet Air Temperature",
            "Air Temp Inlet",
            "Air Temp - Inlet",
        ],
        &["Coolant Temperature", "Engine Temp"],
        &["Engine Oil Temperature", "Eng Oil Temp"],
        &["Engine Oil Pressure", "Eng Oil Pres", "Oil Pressure"],
        &["Fuel Pressure", "Fuel Pres"],
        &[
            "ECU Battery Voltage",
            "Bat Volts ADL",
            "Battery Volts",
            "Battery Voltage at ADL",
        ],
        &[
            "Inlet Manifold Pressure",
            "Manifold Pres",
            "Air Pressure - Manifold",
        ],
        &["Fuel Injector Primary Duty Cycle", "Fuel Inj Duty"],
        &["Ignition Timing", "Ign Advance"],
        &["Exhaust Lambda Bank 1", "Lambda 1", "Exhaust Lambda"],
        &["Exhaust Lambda Bank 2", "Lambda 2", "Exhaust Lambda"],
        &[
            "CP Lotus Gauge Pack Ambient Air Temperature",
            "CP Lotus Gauge Pack Ambient Air",
            "Ambient Air Temp",
        ],
        &[
            "G Force Lat",
            "CP Lotus ESP Lateral Acceleration",
            "Lateral Acceleration",
        ],
        &[
            "G Force Long",
            "Longitudinal Acceleration",
            "G Force - Longitudinal",
        ],
        &["Brake State", "Brake Status"],
        &[
            "Input Shaft Speed",
            "Input Shaft RPM",
            "Gearbox Input Shaft Speed",
        ],
        &["Drive Shaft Speed", "Drive Shaft RPM"],
        &[
            "CP Lotus Wheel Speed Front Left Hub",
            "Wheel Speed FL",
            "Wheel Speed Front Left",
        ],
        &[
            "CP Lotus Wheel Speed Front Right Hub",
            "Wheel Speed FR",
            "Wheel Speed Front Right",
        ],
        &[
            "CP Lotus Wheel Speed Rear Left Hub",
            "Wheel Speed RL",
            "Wheel Speed Rear Left",
        ],
        &[
            "CP Lotus Wheel Speed Rear Right Hub",
            "Wheel Speed RR",
            "Wheel Speed Rear Right",
        ],
        &["CP Lotus ESP Steering Angle", "Steering Angle"],
    ] {
        shared
            .channel_settings
            .aliases
            .entry(names[0].into())
            .or_insert_with(|| names.iter().map(|n| (*n).into()).collect());
    }
    for (name, alternatives) in [
        (
            "Ambient Air Temp",
            vec!["Ambient Air Temp", "Air Temp - Ambient"],
        ),
        (
            "Wheel Speed Front",
            vec!["Wheel Speed Front", "Wheel Speed - Front"],
        ),
        (
            "Fuel Pump Pres",
            vec!["Fuel Pump Pres", "Fuel Pump Pressure"],
        ),
        (
            "Centre Diff Control",
            vec!["Centre Diff Control", "Centre Diff Duty"],
        ),
        (
            "Centre Diff Current",
            vec!["Centre Diff Current", "Centre Diff Avg Current"],
        ),
    ] {
        shared
            .channel_settings
            .aliases
            .entry(name.into())
            .or_insert_with(|| alternatives.into_iter().map(str::to_string).collect());
    }
    for cylinder in 1..=8 {
        shared
            .channel_settings
            .aliases
            .entry(format!("Exhaust Temp {cylinder}"))
            .or_insert_with(|| {
                vec![
                    format!("Exhaust Temp {cylinder}"),
                    format!("Exhaust Temp - Cyl {cylinder}"),
                ]
            });
    }
    // Older Circuit templates used truncated wheel references. Use explicit
    // alternatives so a sensor-voltage or hub-speed channel cannot prefix-match.
    for (short, long) in [
        (
            "CP Lotus Wheel Speed Front Left",
            "CP Lotus Wheel Speed Front Left Hub",
        ),
        (
            "CP Lotus Wheel Speed Front Right",
            "CP Lotus Wheel Speed Front Right Hub",
        ),
        (
            "CP Lotus Wheel Speed Rear Left",
            "CP Lotus Wheel Speed Rear Left Hub",
        ),
        (
            "CP Lotus Wheel Speed Rear Right",
            "CP Lotus Wheel Speed Rear Right Hub",
        ),
    ] {
        let mut targets = vec![short.into()];
        targets.extend(shared.channel_settings.aliases[long].clone());
        shared
            .channel_settings
            .aliases
            .entry(short.into())
            .or_insert(targets);
    }
}

fn add_math(
    shared: &mut SharedState,
    name: &str,
    expression: &str,
    unit: &str,
    display: Option<&str>,
) {
    // A user's definition or logged definition remains authoritative.
    if shared.resolve_slot(name, false).is_some() {
        return;
    }
    let mut mc = shared.create_math_channel_def(name.into(), expression.into(), unit.into(), 2);
    mc.options.source = "MoTeC".into();
    mc.options.display_unit = display.map(str::to_string);
    mc.options.quantity = if unit == "rad" { "Angle" } else { "" }.into();
    mc.options.comment = "Audited workspace profile calculation".into();
    shared.math_channels.push(mc);
}

pub fn create_profile_worksheets(
    profile: WorkspaceProfile,
    ld: &Arc<LdFile>,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    shared.workspace_profile = Some(profile);
    aliases(shared);
    if profile == WorkspaceProfile::Circuit {
        crate::panels::math_editor::install_oversteer_template(shared);
        add_math(
            shared,
            "Oversteer (Speed Weighted)",
            "'Corr Speed'[km/h] / 80 * 'Oversteer'[rad]",
            "rad",
            Some("deg"),
        );
        add_math(
            shared,
            "Front Wheels Diff (Left - Right)",
            "'CP Lotus Wheel Speed Front Left'[rpm] - 'CP Lotus Wheel Speed Front Right'[rpm]",
            "rpm",
            None,
        );
        add_math(
            shared,
            "Rear Wheel Diff (Left - Right)",
            "'CP Lotus Wheel Speed Rear Left'[rpm] - 'CP Lotus Wheel Speed Rear Right'[rpm]",
            "rpm",
            None,
        );
    }
    shared.sync_drag_math();
    let sheets = match profile {
        WorkspaceProfile::Circuit => create_circuit_worksheets(ld, shared),
        WorkspaceProfile::Engine => engine(ld, shared),
        WorkspaceProfile::Drag => drag(ld, shared),
        WorkspaceProfile::Rally => rally(ld, shared),
    };
    crate::panels::math_editor::evaluate_all_math_channels(shared);
    sheets
}

/// Each inner list is one tiled graph group, with exact slot identities.
fn graph(
    ld: &LdFile,
    shared: &mut SharedState,
    name: &str,
    groups: &[&[&str]],
    gauges: &[(&str, GaugeStyle)],
) -> GraphPanel {
    let mut p = GraphPanel::new(next_panel_id(shared), name);
    for (group, names) in groups.iter().enumerate() {
        for name in *names {
            let color = CHANNEL_COLORS[p.plotted_channels.len() % CHANNEL_COLORS.len()];
            p.plotted_channels
                .push(named_slot(ld, shared, &[name], color, group).unwrap());
        }
    }
    for (name, style) in gauges {
        let color = CHANNEL_COLORS[p.embedded_gauges.len() % CHANNEL_COLORS.len()];
        let channel = named_slot(ld, shared, &[name], color, 0).unwrap();
        let range = if name.contains("Lambda") {
            Some((0.7, 1.1))
        } else if *name == "Throttle Position" || name.contains("Diff Lock") {
            Some((0., 100.))
        } else if *name == "G Force Lat" {
            Some((-2., 2.))
        } else if *name == "G Force Long" {
            Some((-1.5, 1.5))
        } else {
            None
        };
        p.embedded_gauges.push(crate::panels::gauge::GaugeChannel {
            channel,
            style: *style,
            last_unit: None,
            settings: crate::panels::gauge::GaugeSettings {
                range,
                ..Default::default()
            },
        });
    }
    p
}
fn sheet(name: &str, panel: PanelTab) -> (String, DockState<PanelTab>) {
    (name.into(), DockState::new(vec![panel]))
}
fn graph_sheet(
    ld: &LdFile,
    shared: &mut SharedState,
    name: &str,
    groups: &[&[&str]],
    gauges: &[(&str, GaugeStyle)],
) -> (String, DockState<PanelTab>) {
    sheet(
        name,
        PanelTab::Graph(graph(ld, shared, name, groups, gauges)),
    )
}
fn boost(ld: &LdFile, shared: &mut SharedState) -> (String, DockState<PanelTab>) {
    let mut p = ScatterPanel::new(next_panel_id(shared), "Boost");
    p.x_channel = named_slot(ld, shared, &["Engine Speed"], CHANNEL_COLORS[0], 0);
    p.y_channel = named_slot(
        ld,
        shared,
        &["Inlet Manifold Pressure"],
        CHANNEL_COLORS[1],
        0,
    );
    p.color_channel = named_slot(ld, shared, &["Throttle Position"], CHANNEL_COLORS[2], 0);
    p.color_bands = ColorBands {
        min: 0.,
        max: 100.,
        count: 11,
        centered: true,
    };
    sheet("Boost", PanelTab::Scatter(Box::new(p)))
}
fn report(ld: &LdFile, shared: &mut SharedState, title: &str, names: &[&str]) -> ReportPanel {
    let mut p = ReportPanel::new(next_panel_id(shared), title);
    p.settings.boundary = i3rs_core::report::ReportBoundary::Interpolate;
    for name in names {
        let color = CHANNEL_COLORS[p.channels.len() % CHANNEL_COLORS.len()];
        p.channels
            .push(named_slot(ld, shared, &[name], color, 0).unwrap());
    }
    p
}
fn specialty_rpm(
    ld: &LdFile,
    shared: &mut SharedState,
    drag: bool,
) -> (String, DockState<PanelTab>) {
    let mut h = build_histogram_panel(
        ld,
        shared,
        "Engine Speed",
        &[&["Engine Speed"]],
        if drag { 60 } else { 80 },
        Some(if drag { (4000., 10000.) } else { (0., 8000.) }),
        Some((0., 0.)),
        10.,
    )
    .unwrap();
    h.percentage = true;
    h.color_channel = named_slot(
        ld,
        shared,
        &[if drag {
            "Throttle Position"
        } else {
            "Inlet Manifold Pressure"
        }],
        CHANNEL_COLORS[2],
        0,
    );
    h.color_bands = ColorBands {
        min: 0.,
        max: if drag { 100. } else { 250. },
        count: 11,
        centered: true,
    };
    if drag {
        return sheet("RPM Histogram", PanelTab::Histogram(h));
    }
    let mut overview = graph(
        ld,
        shared,
        "Stage / transport overview",
        &[&["Engine Speed"]],
        &[],
    );
    overview.view.mode = crate::comparison::ViewMode::Overview;
    let mut dock = DockState::new(vec![PanelTab::Graph(overview)]);
    dock.main_surface_mut()
        .split_below(NodeIndex::root(), 0.25, vec![PanelTab::Histogram(h)]);
    ("RPM Histogram".into(), dock)
}
const HEALTH_GAUGES: &[(&str, GaugeStyle)] = &[
    ("Engine Speed", GaugeStyle::Analog),
    ("Inlet Air Temperature", GaugeStyle::Bar),
    ("Coolant Temperature", GaugeStyle::Bar),
    ("Engine Oil Temperature", GaugeStyle::Bar),
    ("Engine Oil Pressure", GaugeStyle::Analog),
    ("Fuel Pressure", GaugeStyle::Analog),
    ("ECU Battery Voltage", GaugeStyle::Bar),
];
const FUEL_GAUGES: &[(&str, GaugeStyle)] = &[
    ("Engine Speed", GaugeStyle::Analog),
    ("Throttle Position", GaugeStyle::Bar),
    ("Inlet Manifold Pressure", GaugeStyle::Analog),
    ("Fuel Pressure", GaugeStyle::Bar),
    ("Fuel Injector Primary Duty Cycle", GaugeStyle::Bar),
    ("Exhaust Lambda Bank 1", GaugeStyle::Bar),
    ("Exhaust Lambda Bank 1", GaugeStyle::Digital),
    ("Exhaust Lambda Bank 2", GaugeStyle::Bar),
    ("Exhaust Lambda Bank 2", GaugeStyle::Digital),
    ("Ignition Timing", GaugeStyle::Bar),
];
const CAM_GAUGES: &[(&str, GaugeStyle)] = &[
    ("Engine Speed", GaugeStyle::Analog),
    ("Throttle Position", GaugeStyle::Bar),
    ("Inlet Manifold Pressure", GaugeStyle::Analog),
    ("Engine Oil Pressure", GaugeStyle::Analog),
    ("Coolant Temperature", GaugeStyle::Analog),
    ("Fuel Pressure", GaugeStyle::Analog),
    ("Fuel Injector Primary Duty Cycle", GaugeStyle::Bar),
    ("Exhaust Lambda Bank 1", GaugeStyle::Bar),
    ("Exhaust Lambda Bank 2", GaugeStyle::Bar),
];
fn engine(ld: &LdFile, shared: &mut SharedState) -> Vec<(String, DockState<PanelTab>)> {
    vec![
        graph_sheet(
            ld,
            shared,
            "General",
            &[
                &["Engine Speed", "Engine Speed Limit", "Idle Aim"],
                &["Gear"],
                &["Engine Load Normalised", "Inlet Manifold Pressure"],
                &[
                    "Exhaust Lambda",
                    "Exhaust Lambda Bank 1",
                    "Exhaust Lambda Bank 2",
                    "Fuel Mixture Aim",
                ],
                &[
                    "Fuel Closed Loop Control Trim",
                    "Fuel Closed Loop Control Bank 1 Trim",
                    "Fuel Closed Loop Control Bank 2 Trim",
                ],
                &["Coolant Temperature", "Inlet Air Temperature"],
                &["Clutch Position", "Throttle Pedal"],
                &["Vehicle Speed"],
                &["Warning Source", "Engine Speed Limit State"],
            ],
            HEALTH_GAUGES,
        ),
        graph_sheet(
            ld,
            shared,
            "Cam Control",
            &[
                &["Throttle Position"],
                &["Engine Oil Pressure"],
                &["Engine Speed"],
                &[
                    "Inlet Camshaft Aim",
                    "Inlet Camshaft Bank 1 Position",
                    "Inlet Camshaft Bank 2 Position",
                ],
                &[
                    "Exhaust Camshaft Aim",
                    "Exhaust Camshaft Bank 1 Position",
                    "Exhaust Camshaft Bank 2 Position",
                ],
                &[
                    "Exhaust Camshaft Bank 2 Control",
                    "Exhaust Camshaft Bank 1 Control",
                ],
                &["Engine Oil Pressure Warning"],
            ],
            CAM_GAUGES,
        ),
        graph_sheet(
            ld,
            shared,
            "Knock",
            &[
                &["Engine Speed", "Engine Speed Limit"],
                &["Ignition Timing"],
                &[
                    "Ignition Timing Compensation",
                    "Coolant Temperature Ignition Timing Compensation",
                    "Inlet Air Temperature Ignition Timing Compensation",
                ],
                &[
                    "Inlet Camshaft Aim",
                    "Inlet Camshaft Bank 1 Position",
                    "Inlet Camshaft Bank 2 Position",
                ],
                &[
                    "Knock Threshold",
                    "Ignition Cylinder 1 Knock Level",
                    "Ignition Cylinder 2 Knock Level",
                    "Ignition Cylinder 3 Knock Level",
                    "Ignition Cylinder 4 Knock Level",
                    "Ignition Cylinder 5 Knock Level",
                    "Ignition Cylinder 6 Knock Level",
                ],
                &["Fuel Pressure", "Inlet Manifold Pressure"],
                &[
                    "Fuel Closed Loop Control Bank 1 Trim",
                    "Fuel Closed Loop Control Bank 2 Trim",
                ],
            ],
            &[],
        ),
        graph_sheet(
            ld,
            shared,
            "Fuel Usage",
            &[
                &["CP Lotus Fuel Tank Level", "Fuel Used"],
                &["CP Lotus Fuel Tank Level Sensor Voltage"],
                &["Engine Oil Temperature"],
                &["Fuel Pressure", "Engine Oil Pressure"],
                &["Fuel Injector Primary Duty Cycle"],
                &["Engine Speed"],
                &[
                    "Ignition Cylinder 1 Trim Knock",
                    "Ignition Cylinder 2 Trim Knock",
                    "Ignition Cylinder 3 Trim Knock",
                    "Ignition Cylinder 4 Trim Knock",
                    "Ignition Cylinder 5 Trim Knock",
                    "Ignition Cylinder 6 Trim Knock",
                ],
            ],
            &[],
        ),
        graph_sheet(
            ld,
            shared,
            "Wheel Speeds",
            &[
                &[
                    "CP Lotus Gauge Pack Ambient Air Temperature",
                    "Inlet Air Temperature",
                ],
                &["Engine Oil Temperature", "Coolant Temperature"],
                &["Engine Speed"],
                &["Vehicle Speed", "GPS Speed"],
                &[
                    "CP Lotus Wheel Speed Front Left Hub",
                    "CP Lotus Wheel Speed Front Right Hub",
                    "CP Lotus Wheel Speed Rear Left Hub",
                    "CP Lotus Wheel Speed Rear Right Hub",
                ],
                &["Throttle Pedal"],
                &["Traction Control Cut"],
                &["Inlet Manifold Pressure", "Boost Pressure"],
                &["CP Lotus Coolant PWM Fan Duty Cycle"],
            ],
            &[],
        ),
    ]
}
fn drag(ld: &LdFile, shared: &mut SharedState) -> Vec<(String, DockState<PanelTab>)> {
    use crate::panels::report::{ReportKind, ReportScope, ReportView};
    let shafts: &[&str] = &["Engine Speed", "Input Shaft Speed", "Drive Shaft Speed"];
    let mut sheets = vec![
        graph_sheet(
            ld,
            shared,
            "EGTs",
            &[
                shafts,
                &[
                    "Exhaust Temp 1",
                    "Exhaust Temp 2",
                    "Exhaust Temp 3",
                    "Exhaust Temp 4",
                    "Exhaust Temp 5",
                    "Exhaust Temp 6",
                    "Exhaust Temp 7",
                    "Exhaust Temp 8",
                ],
                &["Throttle Position"],
            ],
            &[
                ("Exhaust Temp 1", GaugeStyle::Bar),
                ("Exhaust Temp 3", GaugeStyle::Bar),
                ("Exhaust Temp 5", GaugeStyle::Bar),
                ("Exhaust Temp 7", GaugeStyle::Bar),
                ("Engine Speed", GaugeStyle::Analog),
                ("Corr Speed", GaugeStyle::Analog),
                ("Exhaust Temp 2", GaugeStyle::Bar),
                ("Exhaust Temp 4", GaugeStyle::Bar),
                ("Exhaust Temp 6", GaugeStyle::Bar),
                ("Exhaust Temp 8", GaugeStyle::Bar),
            ],
        ),
        graph_sheet(
            ld,
            shared,
            "Drive Line",
            &[
                shafts,
                &["Clutch Slip"],
                &["G Force Long"],
                &["Wheel Speed Front", "Corr Speed"],
                &["Gear"],
                &["Throttle Position"],
            ],
            &[],
        ),
        graph_sheet(
            ld,
            shared,
            "Fuel",
            &[
                shafts,
                &["Fuel Pump Pres"],
                &["Corr Speed"],
                &["Throttle Position"],
            ],
            &[],
        ),
        graph_sheet(
            ld,
            shared,
            "Engine Health",
            &[
                shafts,
                &["Engine Oil Pressure"],
                &["Inlet Manifold Pressure"],
                &["Ambient Air Temp"],
                &["Inlet Air Temperature"],
                &["ECU Battery Voltage"],
                &["Throttle Position"],
            ],
            &[],
        ),
        graph_sheet(
            ld,
            shared,
            "Acceleration",
            &[
                shafts,
                &["G Force Long"],
                &["Wheel Speed Front", "Corr Speed"],
                &["Throttle Position"],
            ],
            &[("G Force Long", GaugeStyle::Bar)],
        ),
        boost(ld, shared),
    ];
    // An independent full-outing graph accompanies the run-focused acceleration view.
    let mut overview = graph(ld, shared, "Staging overview", &[&["Engine Speed"]], &[]);
    overview.view.mode = crate::comparison::ViewMode::Overview;
    sheets[4].1.main_surface_mut().split_below(
        NodeIndex::root(),
        0.82,
        vec![PanelTab::Graph(overview)],
    );
    let mut channels = report(
        ld,
        shared,
        "Run distance channels",
        &[
            "Engine Speed",
            "G Force Long",
            "Drive Shaft Speed",
            "Engine Oil Pressure",
            "ECU Battery Voltage",
            "Corr Speed",
        ],
    );
    channels.settings.scope = ReportScope::SelectedRanges;
    channels.settings.drag_milestones = true;
    channels.settings.range_kind = Some(i3rs_core::ranges::RangeKind::Run);
    channels.settings.view = ReportView::Split;
    channels.settings.chart_statistics = vec![
        i3rs_core::report::ReportStatistic::Min,
        i3rs_core::report::ReportStatistic::Max,
        i3rs_core::report::ReportStatistic::Average,
    ];
    channels.settings.split_horizontal = true;
    channels.settings.statistics = vec![
        i3rs_core::report::ReportStatistic::Min,
        i3rs_core::report::ReportStatistic::Max,
        i3rs_core::report::ReportStatistic::Average,
        i3rs_core::report::ReportStatistic::Start,
        i3rs_core::report::ReportStatistic::End,
        i3rs_core::report::ReportStatistic::Change,
    ];
    let mut times = report(ld, shared, "Run milestone times", &[]);
    times.settings = channels.settings.clone();
    times.settings.kind = ReportKind::Time;
    let mut dock = DockState::new(vec![PanelTab::Report(channels)]);
    dock.main_surface_mut()
        .split_below(NodeIndex::root(), 0.65, vec![PanelTab::Report(times)]);
    sheets.push(("Reports".into(), dock));
    sheets.push(specialty_rpm(ld, shared, true));
    sheets
}
fn rally(ld: &LdFile, shared: &mut SharedState) -> Vec<(String, DockState<PanelTab>)> {
    let mut sheets = vec![
        graph_sheet(
            ld,
            shared,
            "Driver",
            &[
                &["Engine Speed"],
                &["Corr Speed"],
                &["Throttle Position"],
                &["Steered Angle"],
                &["G Force Lat", "G Force Long"],
                &["Gear", "Brake State"],
            ],
            &[
                ("Engine Speed", GaugeStyle::Analog),
                ("Corr Speed", GaugeStyle::Analog),
                ("Throttle Position", GaugeStyle::Bar),
                ("Steered Angle", GaugeStyle::SteeringWheel),
                ("G Force Long", GaugeStyle::Bar),
                ("G Force Lat", GaugeStyle::Bar),
                ("Gear", GaugeStyle::Digital),
                ("Brake State", GaugeStyle::Digital),
            ],
        ),
        graph_sheet(
            ld,
            shared,
            "Engine",
            &[
                &["Engine Speed"],
                &[
                    "Inlet Air Temperature",
                    "Coolant Temperature",
                    "Engine Oil Temperature",
                ],
                &["Engine Oil Pressure", "Fuel Pressure"],
                &["ECU Battery Voltage"],
            ],
            HEALTH_GAUGES,
        ),
        graph_sheet(
            ld,
            shared,
            "Fuel / Ign",
            &[
                &["Engine Speed"],
                &["Throttle Position"],
                &["Inlet Manifold Pressure"],
                &["Fuel Pressure"],
                &["Fuel Injector Primary Duty Cycle"],
                &["Exhaust Lambda Bank 1", "Exhaust Lambda Bank 2"],
                &["Ignition Timing"],
            ],
            FUEL_GAUGES,
        ),
        build_mixture_map_worksheet(ld, shared).unwrap(),
        boost(ld, shared),
    ];
    sheets.push(specialty_rpm(ld, shared, false));
    sheets.push(graph_sheet(
        ld,
        shared,
        "Diff Controller",
        &[
            &["Ground Speed", "Wheel Speed Front", "Wheel Speed Rear"],
            &[
                "Wheel Speed FL",
                "Wheel Speed FR",
                "Wheel Speed RL",
                "Wheel Speed RR",
            ],
            &["Wheel Slip"],
            &["Throttle Position"],
            &["Centre Diff Lock"],
            &["SDC Mode", "Centre Diff Control", "Centre Diff Current"],
        ],
        &[
            ("Ground Speed", GaugeStyle::Analog),
            ("Wheel Speed Front", GaugeStyle::Bar),
            ("Wheel Speed Rear", GaugeStyle::Bar),
            ("Wheel Slip", GaugeStyle::Analog),
            ("Throttle Position", GaugeStyle::Bar),
            ("Centre Diff Lock", GaugeStyle::Bar),
            ("Gear", GaugeStyle::Digital),
            ("Brake State", GaugeStyle::Digital),
            ("SDC Mode", GaugeStyle::Digital),
        ],
    ));
    use crate::panels::report::{ReportScope, ReportView};
    use i3rs_core::report::ReportStatistic as Stat;
    let mut selected = report(
        ld,
        shared,
        "Selected stage",
        &["Engine Oil Pressure", "Engine Speed", "Coolant Temperature"],
    );
    selected.settings.scope = ReportScope::SelectedRanges;
    selected.settings.range_kind = Some(i3rs_core::ranges::RangeKind::Stage);
    selected.settings.view = ReportView::Split;
    selected.settings.chart_statistics = vec![Stat::Min, Stat::Max, Stat::Average];
    selected.settings.split_horizontal = true;
    selected.settings.statistics = vec![Stat::Min, Stat::Max, Stat::Average];
    selected.settings.channel_statistics = vec![Some(vec![Stat::Min]), None, Some(vec![Stat::Max])];
    let mut all = report(
        ld,
        shared,
        "All stages",
        &["Engine Oil Pressure", "Engine Speed", "Coolant Temperature"],
    );
    all.settings = selected.settings.clone();
    all.settings.scope = ReportScope::Ranges;
    let mut dock = DockState::new(vec![PanelTab::Report(selected)]);
    dock.main_surface_mut()
        .split_below(NodeIndex::root(), 0.5, vec![PanelTab::Report(all)]);
    sheets.push(("Channel Report".into(), dock));
    sheets.push(graph_sheet(ld, shared, "Spare", &[&["Engine Speed"]], &[]));
    sheets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{load_workspace, save_workspace};
    fn state(missing: bool) -> (Arc<LdFile>, SharedState) {
        let mut ld = LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        if missing {
            for c in &mut ld.channels {
                c.name = format!("Absent {}", c.index);
            }
        }
        let ld = Arc::new(ld);
        let mut shared = SharedState::new();
        shared.ld_file = Some(ld.clone());
        shared.data_duration = Some(ld.duration_secs());
        shared.activate_recording_analysis(&i3rs_core::detect_laps(&ld));
        (ld, shared)
    }
    #[test]
    fn drag_and_rally_boost_color_by_throttle_even_when_the_slot_is_missing() {
        for missing in [false, true] {
            for profile in [WorkspaceProfile::Drag, WorkspaceProfile::Rally] {
                let (ld, mut shared) = state(missing);
                let sheets = create_profile_worksheets(profile, &ld, &mut shared);
                let (_, dock) = sheets.iter().find(|(name, _)| name == "Boost").unwrap();
                let (_, PanelTab::Scatter(panel)) = dock.iter_all_tabs().next().unwrap() else {
                    panic!()
                };
                assert_eq!(
                    panel
                        .color_channel
                        .as_ref()
                        .unwrap()
                        .requested_reference
                        .as_ref()
                        .unwrap()
                        .0,
                    "Throttle Position"
                );
                assert_eq!((panel.color_bands.min, panel.color_bands.max), (0., 100.));
                assert_eq!(panel.color_bands.count, 11);
                assert!(panel.color_bands.centered);
            }
        }
    }

    #[test]
    fn every_audited_sheet_renders_with_missing_specialty_inputs() {
        let (ld, mut shared) = state(false);
        let context = egui::Context::default();
        for profile in WorkspaceProfile::ALL {
            for (_, mut dock) in create_profile_worksheets(profile, &ld, &mut shared) {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1800., 1000.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui_dock::DockArea::new(&mut dock).show_inside(
                            ui,
                            &mut crate::panels::AppTabViewer {
                                shared: &mut shared,
                            },
                        );
                    },
                );
                output.platform_output.commands.clear();
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn all_33_sheets_keep_every_component_and_slot_with_missing_channels_and_round_trip() {
        for (profile, expected) in [
            (
                WorkspaceProfile::Circuit,
                vec![
                    "Driver",
                    "Braking",
                    "Engine",
                    "Fuel / Ign",
                    "Mixture Map",
                    "Oil Pressure",
                    "RPM Histo",
                    "Track Report",
                    "Channel Report",
                    "Section Times",
                    "Spare",
                ],
            ),
            (
                WorkspaceProfile::Drag,
                vec![
                    "EGTs",
                    "Drive Line",
                    "Fuel",
                    "Engine Health",
                    "Acceleration",
                    "Boost",
                    "Reports",
                    "RPM Histogram",
                ],
            ),
            (
                WorkspaceProfile::Engine,
                vec![
                    "General",
                    "Cam Control",
                    "Knock",
                    "Fuel Usage",
                    "Wheel Speeds",
                ],
            ),
            (
                WorkspaceProfile::Rally,
                vec![
                    "Driver",
                    "Engine",
                    "Fuel / Ign",
                    "Mixture Map",
                    "Boost",
                    "RPM Histogram",
                    "Diff Controller",
                    "Channel Report",
                    "Spare",
                ],
            ),
        ] {
            let (ld, mut shared) = state(false);
            let sheets = create_profile_worksheets(profile, &ld, &mut shared);
            assert_eq!(
                sheets.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
                expected
            );
            let refs: Vec<_> = sheets.iter().map(|s| (s.0.clone(), &s.1)).collect();
            let saved = save_workspace(&refs, 0, &shared);
            let json = serde_json::to_string(&saved).unwrap();
            let saved: crate::workspace::WorkspaceFile = serde_json::from_str(&json).unwrap();
            let (_, mut other) = state(true);
            for c in &saved.math_channels {
                let mc = c.create_definition(&mut other);
                other.math_channels.push(mc);
            }
            let restored = load_workspace(&saved, &mut other);
            assert_eq!(other.workspace_profile, Some(profile));
            let refs: Vec<_> = restored.iter().map(|s| (s.0.clone(), &s.1)).collect();
            let again = save_workspace(&refs, 0, &other);
            // Every panel configuration includes its missing slot references,
            // grouping, gauges, report policies and serialized dock topology.
            assert_eq!(
                serde_json::to_value(&saved.worksheets).unwrap(),
                serde_json::to_value(&again.worksheets).unwrap(),
                "{profile:?}"
            );
            let (empty, mut missing) = state(true);
            let generated = create_profile_worksheets(profile, &empty, &mut missing);
            assert_eq!(generated.len(), expected.len());
            for ((name, dock), (_, expected_dock)) in generated.iter().zip(&sheets) {
                assert_eq!(
                    dock.iter_all_tabs().count(),
                    expected_dock.iter_all_tabs().count(),
                    "{name}"
                );
            }
        }
    }
    #[test]
    fn profile_math_is_reused_and_lambda_has_both_presentations() {
        let (ld, mut shared) = state(false);
        let sheets = create_profile_worksheets(WorkspaceProfile::Circuit, &ld, &mut shared);
        let count = shared.math_channels.len();
        create_profile_worksheets(WorkspaceProfile::Circuit, &ld, &mut shared);
        assert_eq!(shared.math_channels.len(), count);
        for reference in [
            "Oversteer",
            "Oversteer (Speed Weighted)",
            "Front Wheels Diff (Left - Right)",
            "Rear Wheel Diff (Left - Right)",
        ] {
            assert!(matches!(
                shared.resolve_slot(reference, false),
                Some(crate::state::ChannelId::Math(_))
            ));
        }
        let (_, PanelTab::Graph(fuel)) = sheets[3].1.iter_all_tabs().next().unwrap() else {
            panic!()
        };
        for bank in ["Exhaust Lambda Bank 1", "Exhaust Lambda Bank 2"] {
            for style in [GaugeStyle::Bar, GaugeStyle::Digital] {
                assert!(fuel.embedded_gauges.iter().any(|g| g.style == style
                    && g.channel.requested_reference.as_ref().unwrap().0 == bank));
            }
        }
    }
}

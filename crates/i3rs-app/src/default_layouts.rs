//! Default worksheet layouts matching MoTeC i2 Pro's tab structure.
//!
//! When a .ld file is opened and no workspace is loaded, we auto-create
//! worksheets with channels matched by name from the loaded file.

use std::sync::Arc;

use egui_dock::DockState;
use i3rs_core::LdFile;

use crate::panels::PanelTab;
use crate::panels::graph::GraphPanel;
use crate::state::{
    CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState, YAxis, compute_channel_stats,
};

/// A template for a default worksheet.
struct WorksheetTemplate {
    name: &'static str,
    /// Channel names to look for (matched case-insensitively).
    channels: &'static [&'static str],
}

const DEFAULT_WORKSHEETS: &[WorksheetTemplate] = &[
    WorksheetTemplate {
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
    },
    WorksheetTemplate {
        name: "Braking",
        channels: &[
            "Engine Speed",
            "Vehicle Speed",
            "Corr Speed",
            "GPS Speed",
            "G Force Lat",
            "CP Lotus ESP Lateral Acceleration",
            "Throttle Position",
            "Brake State",
            "Wheel Speed Front Left",
            "CP Lotus Wheel Speed Front Left",
            "Wheel Speed Front Right",
            "CP Lotus Wheel Speed Front Right",
            "Wheel Speed Rear Left",
            "CP Lotus Wheel Speed Rear Left",
            "Wheel Speed Rear Right",
            "CP Lotus Wheel Speed Rear Right",
            "CP Lotus ESP Steering Angle",
        ],
    },
    WorksheetTemplate {
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
    },
    WorksheetTemplate {
        name: "Fuel / Ign",
        channels: &[
            "Engine Speed",
            "Throttle Position",
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
    },
    WorksheetTemplate {
        name: "Spare",
        channels: &[
            "GPS Speed",
            "Vehicle Speed",
            "Corr Speed",
            "Torque Control Cut",
            "Throttle Pedal",
            "Torque Limit State",
            "Engine Speed Limit State",
            "CP Lotus ESP Lateral Acceleration",
            "ECU Acceleration X",
            "ECU Acceleration Y",
            "CP Engine Speed Targeting Target",
            "CP Rev Match Engine Speed Target",
        ],
    },
];

/// Find a channel index by name (case-insensitive, partial match with prefix).
fn find_channel(ld: &LdFile, name: &str) -> Option<usize> {
    // Exact match first (case-insensitive)
    if let Some(idx) = ld
        .channels
        .iter()
        .position(|ch| ch.name.eq_ignore_ascii_case(name))
    {
        return Some(idx);
    }

    // Partial match: channel name starts with the search name (handles truncated names)
    // e.g. "CP Lotus Gauge Pack Ambient Air" matches "CP Lotus Gauge Pack Ambient Air Temperature"
    ld.channels.iter().position(|ch| {
        ch.name
            .get(..name.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
            || name
                .get(..ch.name.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&ch.name))
    })
}

/// Create default worksheets from the loaded file's available channels.
/// Returns a list of (name, dock_state) pairs for worksheets that have
/// at least one matching channel.
pub fn create_default_worksheets(
    ld: &Arc<LdFile>,
    shared: &mut SharedState,
) -> Vec<(String, DockState<PanelTab>)> {
    let mut result = Vec::new();

    for template in DEFAULT_WORKSHEETS {
        let mut matched: Vec<usize> = Vec::new();

        for &chan_name in template.channels {
            if let Some(idx) = find_channel(ld, chan_name) {
                // Avoid duplicates (e.g. both "G Force Lat" and "CP Lotus ESP Lateral Acceleration")
                if !matched.contains(&idx) {
                    matched.push(idx);
                }
            }
        }

        if matched.is_empty() {
            continue;
        }

        let panel_id = shared.next_panel_id;
        shared.next_panel_id += 1;
        let mut graph = GraphPanel::new(panel_id, template.name);

        for (i, &chan_idx) in matched.iter().enumerate() {
            let ch = &ld.channels[chan_idx];
            if let Some(data) = ld.read_channel_data(ch) {
                let (cached_min, cached_max, cached_avg, _) = compute_channel_stats(&data);
                let color = CHANNEL_COLORS[i % CHANNEL_COLORS.len()];
                graph.plotted_channels.push(PlottedChannel {
                    channel_id: ChannelId::Physical(chan_idx),
                    color,
                    data: Arc::new(data),
                    y_axis: YAxis::Left,
                    cached_min,
                    cached_max,
                    cached_avg,
                });
            }
        }

        if !graph.plotted_channels.is_empty() {
            let dock = DockState::new(vec![PanelTab::Graph(graph)]);
            result.push((template.name.to_string(), dock));
        }
    }

    result
}

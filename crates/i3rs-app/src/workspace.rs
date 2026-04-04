//! Workspace save/load: serializes panel layout and channel configuration.

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use i3rs_core::Sector;

use crate::panels::PanelTab;
use crate::panels::graph::GraphPanel;
use crate::panels::track_map::TrackMapPanel;
use crate::state::{CHANNEL_COLORS, ChannelId, GraphMode, SharedState};

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
    ChannelBrowser,
    CursorReadout,
    Report(ReportPanelConfig),
}

#[derive(Serialize, Deserialize)]
pub struct GraphPanelConfig {
    pub id: u64,
    pub title: String,
    pub channel_names: Vec<String>,
    pub colors: Vec<[u8; 3]>,
    pub graph_mode: String, // "Tiled" or "Overlay"
    /// Whether each channel is a math channel (true) or physical (false).
    #[serde(default)]
    pub is_math: Vec<bool>,
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


#[derive(Serialize, Deserialize, Clone)]
pub struct MathChannelConfig {
    pub name: String,
    pub expression: String,
    pub unit: String,
    pub dec_places: i16,
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
                        PanelConfig::Graph(GraphPanelConfig {
                            id: g.id,
                            title: g.title.clone(),
                            channel_names,
                            colors,
                            graph_mode: match g.graph_mode {
                                GraphMode::Tiled => "Tiled".into(),
                                GraphMode::Overlay => "Overlay".into(),
                            },
                            is_math,
                        })
                    }
                    PanelTab::TrackMap(t) => {
                        let color_channel_name = t.color_channel_idx.and_then(|idx| {
                            shared.ld_file.as_ref().and_then(|ld| {
                                ld.channels.get(idx).map(|ch| ch.name.clone())
                            })
                        });
                        PanelConfig::TrackMap(TrackMapPanelConfig {
                            id: t.id,
                            title: t.title.clone(),
                            color_channel_name,
                        })
                    }
                    PanelTab::ChannelBrowser => PanelConfig::ChannelBrowser,
                    PanelTab::CursorReadout => PanelConfig::CursorReadout,
                    PanelTab::Report(r) => PanelConfig::Report(ReportPanelConfig {
                        id: r.id,
                        title: r.title.clone(),
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
        .map(|ws_config| {
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

                        // Resolve channels by name
                        for (i, name) in gc.channel_names.iter().enumerate() {
                            let is_math = gc.is_math.get(i).copied().unwrap_or(false);
                            let color = gc
                                .colors
                                .get(i)
                                .map(|c| eframe::egui::Color32::from_rgb(c[0], c[1], c[2]))
                                .unwrap_or(CHANNEL_COLORS[i % CHANNEL_COLORS.len()]);

                            if is_math {
                                // Find math channel by name
                                if let Some(mc_idx) = shared
                                    .math_channels
                                    .iter()
                                    .position(|mc| mc.name == *name)
                                {
                                    if let Some(data) = &shared.math_channels[mc_idx].data {
                                        let (cached_min, cached_max, cached_avg) =
                                            GraphPanel::compute_stats(data);
                                        graph.plotted_channels.push(crate::state::PlottedChannel {
                                            channel_id: ChannelId::Math(mc_idx),
                                            color,
                                            data: data.clone(),
                                            y_axis: crate::state::YAxis::Left,
                                            cached_min,
                                            cached_max,
                                            cached_avg,
                                        });
                                    }
                                }
                            } else if let Some(ld) = &shared.ld_file {
                                if let Some(ch) = ld.channels.iter().find(|c| &c.name == name) {
                                    if let Some(data) = ld.read_channel_data(ch) {
                                        let (cached_min, cached_max, cached_avg) =
                                            GraphPanel::compute_stats(&data);
                                        graph.plotted_channels.push(crate::state::PlottedChannel {
                                            channel_id: ChannelId::Physical(ch.index),
                                            color,
                                            data: std::sync::Arc::new(data),
                                            y_axis: crate::state::YAxis::Left,
                                            cached_min,
                                            cached_max,
                                            cached_avg,
                                        });
                                    }
                                }
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
                        // Resolve color channel by name
                        if let Some(ref color_name) = tc.color_channel_name
                            && let Some(ld) = &shared.ld_file
                            && let Some(idx) = ld
                                .channels
                                .iter()
                                .position(|ch| &ch.name == color_name)
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
                        let report = crate::panels::report::ReportPanel::new(rc.id, &rc.title);
                        if rc.id >= shared.next_panel_id {
                            shared.next_panel_id = rc.id + 1;
                        }
                        PanelTab::Report(report)
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

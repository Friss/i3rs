//! Workspace save/load: serializes panel layout and channel configuration.

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::panels::PanelTab;
use crate::panels::graph::GraphPanel;
use crate::state::{CHANNEL_COLORS, GraphMode, SharedState};

// ---------------------------------------------------------------------------
// Serializable workspace types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub worksheets: Vec<WorksheetConfig>,
    pub active_worksheet: usize,
    pub last_file_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct WorksheetConfig {
    pub name: String,
    pub panels: Vec<PanelConfig>,
}

#[derive(Serialize, Deserialize)]
pub enum PanelConfig {
    Graph(GraphPanelConfig),
    ChannelBrowser,
    CursorReadout,
}

#[derive(Serialize, Deserialize)]
pub struct GraphPanelConfig {
    pub id: u64,
    pub title: String,
    pub channel_names: Vec<String>,
    pub colors: Vec<[u8; 3]>,
    pub graph_mode: String, // "Tiled" or "Overlay"
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
                        let channel_names: Vec<String> = if let Some(ld) = &shared.ld_file {
                            g.plotted_channels
                                .iter()
                                .map(|pc| ld.channels[pc.channel_index].name.clone())
                                .collect()
                        } else {
                            Vec::new()
                        };
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
                        })
                    }
                    PanelTab::ChannelBrowser => PanelConfig::ChannelBrowser,
                    PanelTab::CursorReadout => PanelConfig::CursorReadout,
                };
                panels.push(config);
            }
            WorksheetConfig {
                name: name.clone(),
                panels,
            }
        })
        .collect();

    WorkspaceFile {
        worksheets: ws_configs,
        active_worksheet,
        last_file_path: shared
            .ld_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
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
                        if let Some(ld) = &shared.ld_file {
                            for (i, name) in gc.channel_names.iter().enumerate() {
                                if let Some(ch) = ld.channels.iter().find(|c| &c.name == name) {
                                    let color = gc
                                        .colors
                                        .get(i)
                                        .map(|c| eframe::egui::Color32::from_rgb(c[0], c[1], c[2]))
                                        .unwrap_or(CHANNEL_COLORS[i % CHANNEL_COLORS.len()]);
                                    if let Some(data) = ld.read_channel_data(ch) {
                                        let (cached_min, cached_max, cached_avg) =
                                            crate::panels::graph::GraphPanel::compute_stats(&data);
                                        graph.plotted_channels.push(crate::state::PlottedChannel {
                                            channel_index: ch.index,
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
                    PanelConfig::ChannelBrowser => PanelTab::ChannelBrowser,
                    PanelConfig::CursorReadout => PanelTab::CursorReadout,
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

            // Note: We can't easily reconstruct the exact split layout from
            // just a list of panels. For a proper implementation, we'd serialize
            // the DockState tree itself. For now, we rebuild a reasonable default.
            // If there's a channel browser, split it to the left.
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

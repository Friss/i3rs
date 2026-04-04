//! UI panels for the application.

pub mod channel_browser;
pub mod cursor_readout;
pub mod graph;
pub mod math_editor;
pub mod report;
pub mod timeline;
pub mod track_map;

use eframe::egui;
use egui_dock::TabViewer;

use crate::state::SharedState;
use graph::GraphPanel;
use report::ReportPanel;
use track_map::TrackMapPanel;

/// Each dockable tab in the workspace.
pub enum PanelTab {
    Graph(GraphPanel),
    TrackMap(TrackMapPanel),
    ChannelBrowser,
    CursorReadout,
    Report(ReportPanel),
}

/// Viewer that bridges shared state to individual panel tabs.
pub struct AppTabViewer<'a> {
    pub shared: &'a mut SharedState,
}

impl TabViewer for AppTabViewer<'_> {
    type Tab = PanelTab;

    fn title(&mut self, tab: &mut PanelTab) -> egui::WidgetText {
        match tab {
            PanelTab::Graph(g) => g.title.clone().into(),
            PanelTab::TrackMap(t) => t.title.clone().into(),
            PanelTab::ChannelBrowser => "Channels".into(),
            PanelTab::CursorReadout => "Readout".into(),
            PanelTab::Report(r) => r.title.clone().into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut PanelTab) {
        match tab {
            PanelTab::Graph(graph) => {
                graph.ui(ui, self.shared);
            }
            PanelTab::TrackMap(track_map) => {
                track_map.ui(ui, self.shared);
            }
            PanelTab::ChannelBrowser => {
                channel_browser::show_standalone(ui, self.shared);
            }
            PanelTab::CursorReadout => {
                cursor_readout::show(ui, self.shared);
            }
            PanelTab::Report(report) => {
                report.ui(ui, self.shared);
            }
        }
    }

    fn id(&mut self, tab: &mut PanelTab) -> egui::Id {
        match tab {
            PanelTab::Graph(g) => egui::Id::new(format!("graph_{}", g.id)),
            PanelTab::TrackMap(t) => egui::Id::new(format!("trackmap_{}", t.id)),
            PanelTab::ChannelBrowser => egui::Id::new("channel_browser"),
            PanelTab::CursorReadout => egui::Id::new("cursor_readout"),
            PanelTab::Report(r) => egui::Id::new(format!("report_{}", r.id)),
        }
    }

    fn is_closeable(&self, tab: &PanelTab) -> bool {
        matches!(
            tab,
            PanelTab::Graph(_) | PanelTab::TrackMap(_) | PanelTab::Report(_)
        )
    }

    fn scroll_bars(&self, _tab: &PanelTab) -> [bool; 2] {
        [false, false]
    }
}

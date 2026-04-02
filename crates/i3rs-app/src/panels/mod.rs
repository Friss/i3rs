//! UI panels for the application.

pub mod channel_browser;
pub mod cursor_readout;
pub mod graph;
pub mod timeline;

use eframe::egui;
use egui_dock::TabViewer;

use crate::state::SharedState;
use graph::GraphPanel;

/// Each dockable tab in the workspace.
pub enum PanelTab {
    Graph(GraphPanel),
    ChannelBrowser,
    CursorReadout,
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
            PanelTab::ChannelBrowser => "Channels".into(),
            PanelTab::CursorReadout => "Readout".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut PanelTab) {
        match tab {
            PanelTab::Graph(graph) => {
                graph.ui(ui, self.shared);
            }
            PanelTab::ChannelBrowser => {
                channel_browser::show_standalone(ui, self.shared);
            }
            PanelTab::CursorReadout => {
                cursor_readout::show(ui, self.shared);
            }
        }
    }

    fn id(&mut self, tab: &mut PanelTab) -> egui::Id {
        match tab {
            PanelTab::Graph(g) => egui::Id::new(format!("graph_{}", g.id)),
            PanelTab::ChannelBrowser => egui::Id::new("channel_browser"),
            PanelTab::CursorReadout => egui::Id::new("cursor_readout"),
        }
    }

    fn is_closeable(&self, tab: &PanelTab) -> bool {
        matches!(tab, PanelTab::Graph(_))
    }

    fn scroll_bars(&self, _tab: &PanelTab) -> [bool; 2] {
        [false, false]
    }
}

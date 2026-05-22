//! UI panels for the application.

pub mod channel_browser;
pub mod cursor_readout;
pub mod fft;
pub mod gauge;
pub mod graph;
pub mod histogram;
pub mod math_editor;
pub mod mixture_map;
pub mod motorcycle_chassis;
pub mod report;
pub mod scatter;
pub mod timeline;
pub mod track_map;
pub mod track_widget;
pub mod utils;

use eframe::egui;
use egui_dock::TabViewer;

use crate::state::SharedState;
use fft::FftPanel;
use gauge::GaugePanel;
use graph::GraphPanel;
use histogram::HistogramPanel;
use mixture_map::MixtureMapPanel;
use motorcycle_chassis::MotorcycleChassisPanel;
use report::ReportPanel;
use scatter::ScatterPanel;
use track_map::TrackMapPanel;

/// Each dockable tab in the workspace.
pub enum PanelTab {
    Graph(GraphPanel),
    TrackMap(TrackMapPanel),
    MotorcycleChassis(MotorcycleChassisPanel),
    ChannelBrowser,
    CursorReadout,
    Report(ReportPanel),
    Histogram(HistogramPanel),
    Scatter(ScatterPanel),
    Fft(FftPanel),
    Gauge(GaugePanel),
    MixtureMap(MixtureMapPanel),
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
            PanelTab::MotorcycleChassis(c) => c.title.clone().into(),
            PanelTab::ChannelBrowser => "Channels".into(),
            PanelTab::CursorReadout => "Readout".into(),
            PanelTab::Report(r) => r.title.clone().into(),
            PanelTab::Histogram(h) => h.title.clone().into(),
            PanelTab::Scatter(s) => s.title.clone().into(),
            PanelTab::Fft(f) => f.title.clone().into(),
            PanelTab::Gauge(g) => g.title.clone().into(),
            PanelTab::MixtureMap(m) => m.title.clone().into(),
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
            PanelTab::MotorcycleChassis(chassis) => {
                chassis.ui(ui, self.shared);
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
            PanelTab::Histogram(histogram) => {
                histogram.ui(ui, self.shared);
            }
            PanelTab::Scatter(scatter) => {
                scatter.ui(ui, self.shared);
            }
            PanelTab::Fft(fft) => {
                fft.ui(ui, self.shared);
            }
            PanelTab::Gauge(gauge) => {
                gauge.ui(ui, self.shared);
            }
            PanelTab::MixtureMap(mixture_map) => {
                mixture_map.ui(ui, self.shared);
            }
        }
    }

    fn id(&mut self, tab: &mut PanelTab) -> egui::Id {
        match tab {
            PanelTab::Graph(g) => egui::Id::new(format!("graph_{}", g.id)),
            PanelTab::TrackMap(t) => egui::Id::new(format!("trackmap_{}", t.id)),
            PanelTab::MotorcycleChassis(c) => egui::Id::new(format!("moto_chassis_{}", c.id)),
            PanelTab::ChannelBrowser => egui::Id::new("channel_browser"),
            PanelTab::CursorReadout => egui::Id::new("cursor_readout"),
            PanelTab::Report(r) => egui::Id::new(format!("report_{}", r.id)),
            PanelTab::Histogram(h) => egui::Id::new(format!("histogram_{}", h.id)),
            PanelTab::Scatter(s) => egui::Id::new(format!("scatter_{}", s.id)),
            PanelTab::Fft(f) => egui::Id::new(format!("fft_{}", f.id)),
            PanelTab::Gauge(g) => egui::Id::new(format!("gauge_{}", g.id)),
            PanelTab::MixtureMap(m) => egui::Id::new(format!("mixture_map_{}", m.id)),
        }
    }

    fn is_closeable(&self, tab: &PanelTab) -> bool {
        matches!(
            tab,
            PanelTab::Graph(_)
                | PanelTab::TrackMap(_)
                | PanelTab::MotorcycleChassis(_)
                | PanelTab::Report(_)
                | PanelTab::Histogram(_)
                | PanelTab::Scatter(_)
                | PanelTab::Fft(_)
                | PanelTab::Gauge(_)
                | PanelTab::MixtureMap(_)
        )
    }

    fn scroll_bars(&self, _tab: &PanelTab) -> [bool; 2] {
        [false, false]
    }
}

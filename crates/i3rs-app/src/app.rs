//! Main application: coordinates panels and shared state.

use eframe::egui;
use egui_dock::{DockArea, DockState};
use i3rs_core::{ExportChannel, LdFile, detect_laps, export_csv, find_ldx_for_ld};
use std::path::PathBuf;
use std::sync::Arc;

use crate::panels::graph::GraphPanel;
use crate::panels::math_editor::{self, MathEditorState};
use crate::panels::report::ReportPanel;
use crate::panels::timeline::TimelinePanel;
use crate::panels::{AppTabViewer, PanelTab};
use crate::state::SharedState;

/// A named workspace layout.
struct Worksheet {
    name: String,
    dock_state: DockState<PanelTab>,
}

pub struct App {
    shared: SharedState,
    worksheets: Vec<Worksheet>,
    active_worksheet: usize,
    show_channel_browser: bool,
    show_cursor_readout: bool,
    show_math_editor: bool,
    timeline: TimelinePanel,
    math_editor_state: MathEditorState,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        let mut shared = SharedState::new();
        let dock_state = Self::default_dock_state(&mut shared);
        let worksheets = vec![Worksheet {
            name: "Sheet 1".into(),
            dock_state,
        }];
        Self {
            shared,
            worksheets,
            active_worksheet: 0,
            show_channel_browser: true,
            show_cursor_readout: true,
            show_math_editor: false,
            timeline: TimelinePanel::new(),
            math_editor_state: MathEditorState::new(),
        }
    }

    fn default_dock_state(shared: &mut SharedState) -> DockState<PanelTab> {
        let graph = GraphPanel::new(shared.next_panel_id, "Graph 1");
        shared.next_panel_id += 1;
        DockState::new(vec![PanelTab::Graph(graph)])
    }

    pub fn open_file(&mut self, path: PathBuf) {
        match LdFile::open(&path) {
            Ok(ld) => {
                self.shared.file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                self.shared.ldx = find_ldx_for_ld(&path);

                let ld = Arc::new(ld);
                self.shared.laps = detect_laps(&ld);
                self.shared.data_duration = Some(ld.duration_secs());
                self.shared.ld_file = Some(ld);
                self.shared.ld_path = Some(path);
                self.shared.selected_lap = None;
                self.shared.zoom_range = None;

                // Clear all graph panels' channels across all worksheets
                for ws in &mut self.worksheets {
                    for (_path, tab) in ws.dock_state.iter_all_tabs_mut() {
                        if let PanelTab::Graph(g) = tab {
                            g.plotted_channels.clear();
                        }
                    }
                }

                // Re-evaluate math channels with new file data
                math_editor::evaluate_all_math_channels(&mut self.shared);
            }
            Err(e) => {
                eprintln!("Failed to open file: {}", e);
            }
        }
    }

    fn add_graph_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let graph = GraphPanel::new(id, format!("Graph {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Graph(graph));
    }

    fn add_report_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let report = ReportPanel::new(id, format!("Report {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Report(report));
    }

    fn add_worksheet(&mut self) {
        let idx = self.worksheets.len() + 1;
        let dock_state = Self::default_dock_state(&mut self.shared);
        self.worksheets.push(Worksheet {
            name: format!("Sheet {}", idx),
            dock_state,
        });
        self.active_worksheet = self.worksheets.len() - 1;
    }

    fn save_workspace(&self) {
        let ws_refs: Vec<(String, &egui_dock::DockState<PanelTab>)> = self
            .worksheets
            .iter()
            .map(|ws| (ws.name.clone(), &ws.dock_state))
            .collect();
        let workspace =
            crate::workspace::save_workspace(&ws_refs, self.active_worksheet, &self.shared);
        if let Ok(json) = serde_json::to_string_pretty(&workspace)
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Workspace", &["json"])
                .set_file_name("workspace.json")
                .save_file()
            && let Err(e) = std::fs::write(&path, json)
        {
            eprintln!("Failed to save workspace: {}", e);
        }
    }

    fn load_workspace(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Workspace", &["json"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(json) => match serde_json::from_str::<crate::workspace::WorkspaceFile>(&json) {
                    Ok(workspace) => {
                        if self.shared.ld_file.is_none()
                            && let Some(ref ld_path) = workspace.last_file_path
                        {
                            let p = std::path::PathBuf::from(ld_path);
                            if p.exists() {
                                self.open_file(p);
                            }
                        }

                        // Load channel aliases from workspace
                        self.shared.channel_aliases.clear();
                        for alias_config in &workspace.channel_aliases {
                            self.shared.channel_aliases.insert(
                                alias_config.alias.clone(),
                                alias_config.target.clone(),
                            );
                        }

                        // Load math channels from workspace
                        self.shared.math_channels.clear();
                        for config in &workspace.math_channels {
                            self.shared
                                .math_channels
                                .push(crate::state::MathChannelDef::new(
                                    config.name.clone(),
                                    config.expression.clone(),
                                    config.unit.clone(),
                                    config.dec_places,
                                ));
                        }
                        math_editor::evaluate_all_math_channels(&mut self.shared);

                        let loaded = crate::workspace::load_workspace(&workspace, &mut self.shared);
                        self.worksheets = loaded
                            .into_iter()
                            .map(|(name, dock_state)| Worksheet { name, dock_state })
                            .collect();
                        self.active_worksheet = workspace
                            .active_worksheet
                            .min(self.worksheets.len().saturating_sub(1));
                    }
                    Err(e) => eprintln!("Failed to parse workspace: {}", e),
                },
                Err(e) => eprintln!("Failed to read workspace file: {}", e),
            }
        }
    }

    fn export_csv(&self) {
        let registry = &self.shared.display_channel_registry;
        if registry.is_empty() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("export.csv")
            .save_file()
        {
            let channels: Vec<ExportChannel<'_>> = registry
                .iter()
                .map(|info| ExportChannel {
                    name: &info.name,
                    data: &info.data,
                    freq: info.freq,
                    dec_places: info.dec_places,
                })
                .collect();

            if let Err(e) = export_csv(&path, &channels, self.shared.zoom_range) {
                eprintln!("Failed to export CSV: {}", e);
            }
        }
    }

    fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open .ld file...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("MoTeC Log", &["ld"])
                        .pick_file()
                    {
                        self.open_file(path);
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Save Workspace...").clicked() {
                    self.save_workspace();
                    ui.close();
                }
                if ui.button("Load Workspace...").clicked() {
                    self.load_workspace();
                    ui.close();
                }
                ui.separator();
                if ui.button("Save Math Channels...").clicked() {
                    math_editor::save_math_channels(&self.shared);
                    ui.close();
                }
                if ui.button("Load Math Channels...").clicked() {
                    math_editor::load_math_channels(&mut self.shared);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        !self.shared.display_channel_registry.is_empty(),
                        egui::Button::new("Export CSV..."),
                    )
                    .clicked()
                {
                    self.export_csv();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Add Graph Panel").clicked() {
                    self.add_graph_panel();
                    ui.close();
                }
                if ui.button("Add Report Panel").clicked() {
                    self.add_report_panel();
                    ui.close();
                }
                ui.separator();
                if ui.button("Add Worksheet").clicked() {
                    self.add_worksheet();
                    ui.close();
                }
                ui.separator();

                // Graph mode
                let dock = &mut self.worksheets[self.active_worksheet].dock_state;
                let mut current_mode = None;
                for (_path, tab) in dock.iter_all_tabs() {
                    if let PanelTab::Graph(g) = tab {
                        current_mode = Some(g.graph_mode);
                        break;
                    }
                }
                if let Some(mut mode) = current_mode {
                    let changed_tiled = ui
                        .radio_value(&mut mode, crate::state::GraphMode::Tiled, "Tiled")
                        .clicked();
                    let changed_overlay = ui
                        .radio_value(&mut mode, crate::state::GraphMode::Overlay, "Overlay")
                        .clicked();
                    if changed_tiled || changed_overlay {
                        for (_path, tab) in dock.iter_all_tabs_mut() {
                            if let PanelTab::Graph(g) = tab {
                                g.graph_mode = mode;
                            }
                        }
                        ui.close();
                    }
                }

                ui.separator();
                ui.checkbox(&mut self.shared.show_lap_markers, "Show lap markers");
                ui.separator();
                ui.checkbox(&mut self.show_channel_browser, "Channel Browser");
                ui.checkbox(&mut self.show_cursor_readout, "Cursor Readout");
                ui.checkbox(&mut self.show_math_editor, "Math Editor");
            });
        });
    }

    fn show_session_info(&self, ui: &mut egui::Ui) {
        if let Some(ld) = &self.shared.ld_file {
            let s = &ld.session;
            let dur = ld.duration_secs();
            let mins = (dur / 60.0) as u32;
            let secs = dur - (mins as f64 * 60.0);

            ui.horizontal(|ui| {
                ui.strong(&self.shared.file_name);
                ui.separator();
                ui.label(format!("{} {}", s.date, s.time));
                ui.separator();
                ui.label(&s.venue);
                ui.separator();
                ui.label(&s.vehicle_id);
                ui.separator();
                ui.label(format!("{}m {:.0}s", mins, secs));
                ui.separator();
                ui.label(format!("{} channels", ld.channels.len()));
                if !self.shared.laps.is_empty() {
                    ui.separator();
                    ui.label(format!("{} laps", self.shared.laps.len()));
                }
                if !self.shared.math_channels.is_empty() {
                    ui.separator();
                    ui.label(format!(
                        "{} math",
                        self.shared.math_channels.len()
                    ));
                }
            });
        }
    }

    fn show_worksheet_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut new_active = self.active_worksheet;
            for (i, ws) in self.worksheets.iter().enumerate() {
                let selected = i == self.active_worksheet;
                if ui.selectable_label(selected, &ws.name).clicked() {
                    new_active = i;
                }
            }
            self.active_worksheet = new_active;

            if ui.small_button("+").clicked() {
                self.add_worksheet();
            }
        });
    }

    /// Draw a collapsed panel strip with vertical text. Returns true if clicked to expand.
    fn collapsed_panel_strip(ui: &mut egui::Ui, label: &str) -> bool {
        let size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let painter = ui.painter();
        let font_id = egui::FontId::proportional(11.0);
        let color = ui.visuals().text_color();
        let char_height = 13.0;
        let total_height = label.len() as f32 * char_height;
        let start_y = rect.center().y - total_height / 2.0;
        for (i, c) in label.chars().enumerate() {
            let pos = egui::pos2(rect.center().x, start_y + i as f32 * char_height);
            painter.text(
                pos,
                egui::Align2::CENTER_TOP,
                c.to_string(),
                font_id.clone(),
                color,
            );
        }

        response.clicked()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Handle file drops
        let dropped_path = ui.ctx().input(|i| {
            i.raw.dropped_files.first().and_then(|dropped| {
                dropped.path.as_ref().and_then(|path| {
                    if path.extension().is_some_and(|ext| ext == "ld") {
                        Some(path.clone())
                    } else {
                        None
                    }
                })
            })
        });
        if let Some(path) = dropped_path {
            self.open_file(path);
        }

        // Swap channel registries: move current frame's data to display buffer,
        // then clear current for the new frame's graph panels to populate.
        std::mem::swap(
            &mut self.shared.plotted_channel_registry,
            &mut self.shared.display_channel_registry,
        );
        self.shared.plotted_channel_registry.clear();

        // Top menu bar
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            self.show_menu_bar(ui);
        });

        // Session info bar
        if self.shared.ld_file.is_some() {
            egui::Panel::top("session_info").show_inside(ui, |ui| {
                self.show_session_info(ui);
            });

            // Timeline overview strip
            egui::Panel::top("timeline").show_inside(ui, |ui| {
                self.timeline.ui(ui, &mut self.shared);
            });
        }

        // Worksheet tabs (only show if more than one)
        if self.worksheets.len() > 1 {
            egui::Panel::top("worksheet_tabs").show_inside(ui, |ui| {
                self.show_worksheet_tabs(ui);
            });
        }

        // Channel browser — collapsible left panel
        if self.show_channel_browser {
            egui::Panel::left("channel_browser")
                .default_size(280.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("\u{25C0}")
                            .on_hover_text("Collapse")
                            .clicked()
                        {
                            self.show_channel_browser = false;
                        }
                        ui.strong("Channels");
                    });
                    ui.separator();
                    crate::panels::channel_browser::show_standalone(ui, &mut self.shared);
                });
        } else {
            egui::Panel::left("channel_browser_collapsed")
                .exact_size(18.0)
                .resizable(false)
                .show_inside(ui, |ui| {
                    if Self::collapsed_panel_strip(ui, "Channels") {
                        self.show_channel_browser = true;
                    }
                });
        }

        // Math editor — collapsible left panel (after browser)
        if self.show_math_editor {
            egui::Panel::left("math_editor")
                .default_size(300.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("\u{25C0}")
                            .on_hover_text("Collapse")
                            .clicked()
                        {
                            self.show_math_editor = false;
                        }
                        ui.strong("Math Editor");
                    });
                    ui.separator();
                    math_editor::show(ui, &mut self.shared, &mut self.math_editor_state);
                });
        }

        // Cursor readout — collapsible right panel
        if self.show_cursor_readout {
            egui::Panel::right("cursor_readout")
                .default_size(200.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Readout");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("\u{25B6}")
                                .on_hover_text("Collapse")
                                .clicked()
                            {
                                self.show_cursor_readout = false;
                            }
                        });
                    });
                    ui.separator();
                    crate::panels::cursor_readout::show(ui, &self.shared);
                });
        } else {
            egui::Panel::right("cursor_readout_collapsed")
                .exact_size(18.0)
                .resizable(false)
                .show_inside(ui, |ui| {
                    if Self::collapsed_panel_strip(ui, "Readout") {
                        self.show_cursor_readout = true;
                    }
                });
        }

        // Dock area fills the rest (graph + report panels)
        let dock = &mut self.worksheets[self.active_worksheet].dock_state;
        let mut viewer = AppTabViewer {
            shared: &mut self.shared,
        };
        DockArea::new(dock)
            .show_close_buttons(true)
            .show_leaf_collapse_buttons(false)
            .draggable_tabs(true)
            .show_inside(ui, &mut viewer);

        // Clear per-frame flags
        self.shared.zoom_from_timeline = false;
    }
}

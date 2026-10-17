//! Main application: coordinates panels and shared state.

use eframe::egui;
use egui_dock::{DockArea, DockState};
use i3rs_core::{LdFile, LdxFile, detect_laps};
#[cfg(target_arch = "wasm32")]
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::background_jobs::{
    BackgroundJobs, JobRequest, JobResult, LoadSessionSource, LoadedSession,
    load_session_from_bytes, load_session_from_path,
};
use crate::panels::fft::FftPanel;
use crate::panels::gauge::GaugePanel;
use crate::panels::graph::GraphPanel;
use crate::panels::histogram::HistogramPanel;
use crate::panels::math_editor::{self, MathEditorState};
use crate::panels::mixture_map::MixtureMapPanel;
use crate::panels::report::ReportPanel;
use crate::panels::scatter::ScatterPanel;
use crate::panels::timeline::TimelinePanel;
use crate::panels::track_map::TrackMapPanel;
use crate::panels::utils::resolve_channel_meta;
use crate::panels::{AppTabViewer, PanelTab};
use crate::preferences::{AppPreferences, ThemeChoice};
use crate::state::SharedState;

/// A named workspace layout.
struct Worksheet {
    window: crate::workspace::WorksheetWindow,
    name: String,
    dock_state: DockState<PanelTab>,
}

#[derive(Clone)]
struct ProjectSessionRecord {
    path: PathBuf,
    label: String,
    notes: String,
}

#[derive(Clone)]
struct SessionSummary {
    file_name: String,
    date: String,
    time: String,
    driver: String,
    vehicle_id: String,
    venue: String,
    event_name: String,
    session_name: String,
    comment: String,
    duration_secs: f64,
    channel_count: usize,
    lap_count: usize,
}

#[derive(serde::Serialize)]
pub struct LoadedSessionSummary {
    pub file_name: String,
    pub channel_count: usize,
    pub lap_count: usize,
}

struct PendingSessionLoad {
    request_id: u64,
    workspace_snapshot: Option<crate::workspace::WorkspaceFile>,
    file_label: String,
}

fn is_startup_default_layout(worksheets: &[Worksheet]) -> bool {
    if worksheets.len() != 1 {
        return false;
    }

    let mut tabs = worksheets[0].dock_state.iter_all_tabs();
    matches!(
        (tabs.next(), tabs.next()),
        (Some((_, PanelTab::Graph(graph))), None) if graph.plotted_channels.is_empty()
    )
}

fn session_record_label(record: &ProjectSessionRecord) -> String {
    if !record.label.trim().is_empty() {
        record.label.clone()
    } else {
        record
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| record.path.to_string_lossy().to_string())
    }
}

fn apply_theme_to_ctx(ctx: &egui::Context, theme: ThemeChoice) {
    match theme {
        ThemeChoice::System => ctx.set_theme(egui::ThemePreference::System),
        ThemeChoice::Light => ctx.set_visuals(egui::Visuals::light()),
        ThemeChoice::Dark => ctx.set_visuals(egui::Visuals::dark()),
        ThemeChoice::HighContrast => {
            let mut visuals = egui::Visuals::dark();
            visuals.override_text_color = Some(egui::Color32::WHITE);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(5, 5, 5);
            visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::WHITE;
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(18, 18, 18);
            visuals.widgets.inactive.fg_stroke.color = egui::Color32::WHITE;
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(40, 70, 100);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 100, 130);
            visuals.selection.bg_fill = egui::Color32::from_rgb(255, 215, 0);
            visuals.selection.stroke.color = egui::Color32::BLACK;
            visuals.hyperlink_color = egui::Color32::from_rgb(140, 210, 255);
            ctx.set_visuals(visuals);
        }
    }
}

pub struct App {
    egui_ctx: egui::Context,
    shared: SharedState,
    worksheets: Vec<Worksheet>,
    active_worksheet: usize,
    show_channel_browser: bool,
    show_cursor_readout: bool,
    show_math_editor: bool,
    timeline: TimelinePanel,
    math_editor_state: MathEditorState,
    /// Track map panels that have been popped out into separate OS windows.
    popped_out_track_maps: Vec<TrackMapPanel>,
    project_path: Option<PathBuf>,
    project_name: Option<String>,
    project_sessions: Vec<ProjectSessionRecord>,
    theme_choice: ThemeChoice,
    options: crate::preferences::AppOptions,
    show_options: bool,
    show_help: bool,
    show_channel_preferences: bool,
    show_session_details: bool,
    recording_editor: crate::panels::recording_editor::RecordingEditor,
    output_ui: crate::output_ui::OutputUi,
    compare_session_path: Option<PathBuf>,
    session_summary_cache: HashMap<PathBuf, SessionSummary>,
    load_error: Option<String>,
    background_jobs: BackgroundJobs,
    pending_session_load: Option<PendingSessionLoad>,
    pending_workspace_restore: Option<crate::workspace::WorkspaceFile>,
    next_session_id: u64,
    next_request_id: u64,
    #[cfg(not(target_arch = "wasm32"))]
    native_pick_tx: std::sync::mpsc::Sender<crate::platform::NativePickEvent>,
    #[cfg(not(target_arch = "wasm32"))]
    native_pick_rx: std::sync::mpsc::Receiver<crate::platform::NativePickEvent>,
    #[cfg(target_arch = "wasm32")]
    web_load_tx: std::sync::mpsc::Sender<crate::platform::WebLoadEvent>,
    #[cfg(target_arch = "wasm32")]
    web_load_rx: std::sync::mpsc::Receiver<crate::platform::WebLoadEvent>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        #[cfg(target_arch = "wasm32")]
        let (web_load_tx, web_load_rx) = crate::platform::web_load_channel();
        #[cfg(not(target_arch = "wasm32"))]
        let (native_pick_tx, native_pick_rx) = crate::platform::native_pick_channel();
        crate::preferences::install_fonts(&cc.egui_ctx);
        let preferences = crate::preferences::load_preferences();
        let mut shared = SharedState::new();
        shared.channel_preferences = preferences.channel_preferences.clone();
        let dock_state = Self::default_dock_state(&mut shared);
        let worksheets = vec![Worksheet {
            window: Default::default(),
            name: "Sheet 1".into(),
            dock_state,
        }];
        apply_theme_to_ctx(&cc.egui_ctx, preferences.theme);
        preferences.options.apply(&cc.egui_ctx);
        Self {
            egui_ctx: cc.egui_ctx.clone(),
            shared,
            worksheets,
            active_worksheet: 0,
            show_channel_browser: true,
            show_cursor_readout: true,
            show_math_editor: false,
            timeline: TimelinePanel::new(),
            math_editor_state: MathEditorState::new(),
            popped_out_track_maps: Vec::new(),
            project_path: None,
            project_name: None,
            project_sessions: Vec::new(),
            theme_choice: preferences.theme,
            options: preferences.options,
            show_options: false,
            show_help: false,
            show_channel_preferences: false,
            show_session_details: false,
            recording_editor: Default::default(),
            output_ui: Default::default(),
            compare_session_path: None,
            session_summary_cache: HashMap::new(),
            load_error: None,
            background_jobs: BackgroundJobs::new(),
            pending_session_load: None,
            pending_workspace_restore: None,
            next_session_id: 1,
            next_request_id: 1,
            #[cfg(not(target_arch = "wasm32"))]
            native_pick_tx,
            #[cfg(not(target_arch = "wasm32"))]
            native_pick_rx,
            #[cfg(target_arch = "wasm32")]
            web_load_tx,
            #[cfg(target_arch = "wasm32")]
            web_load_rx,
        }
    }

    fn default_dock_state(shared: &mut SharedState) -> DockState<PanelTab> {
        let graph = GraphPanel::new(shared.next_panel_id, "Graph 1");
        shared.next_panel_id += 1;
        DockState::new(vec![PanelTab::Graph(graph)])
    }

    fn build_workspace_snapshot_with_popped_out_track_maps(
        &self,
        include_popped_out_track_maps: bool,
    ) -> crate::workspace::WorkspaceFile {
        let ws_refs: Vec<(String, &egui_dock::DockState<PanelTab>)> = self
            .worksheets
            .iter()
            .map(|ws| (ws.name.clone(), &ws.dock_state))
            .collect();
        let mut workspace =
            crate::workspace::save_workspace(&ws_refs, self.active_worksheet, &self.shared);

        for (saved, ws) in workspace.worksheets.iter_mut().zip(&self.worksheets) {
            saved.window = ws.window.clone();
        }
        if include_popped_out_track_maps {
            // Preserve popped-out track maps in the worksheet they came from.
            for tm in &self.popped_out_track_maps {
                let color_channel_name = tm.requested_color.clone().or_else(|| {
                    tm.color_channel_idx.and_then(|idx| {
                        self.shared
                            .ld_file
                            .as_ref()
                            .and_then(|ld| ld.channels.get(idx).map(|ch| ch.name.clone()))
                    })
                });
                let worksheet_idx = tm
                    .home_worksheet
                    .min(workspace.worksheets.len().saturating_sub(1));
                if let Some(ws) = workspace.worksheets.get_mut(worksheet_idx) {
                    ws.panels.push(crate::workspace::PanelConfig::TrackMap(
                        crate::workspace::TrackMapPanelConfig {
                            window: crate::workspace::WorksheetWindow {
                                detached: true,
                                ..tm.window.clone()
                            },
                            id: tm.id,
                            title: tm.title.clone(),
                            color_channel_name,
                            section_times_only: tm.section_times_only,
                            show_annotations: tm.show_annotations,
                            annotation_settings: tm.annotation_settings.clone(),
                        },
                    ));
                }
            }
        }

        workspace
    }

    fn workspace_snapshot_for_session_reload(&self) -> Option<crate::workspace::WorkspaceFile> {
        if is_startup_default_layout(&self.worksheets) {
            None
        } else {
            Some(self.build_workspace_snapshot_with_popped_out_track_maps(false))
        }
    }

    fn apply_workspace_snapshot(&mut self, workspace: crate::workspace::WorkspaceFile) {
        let active_worksheet = workspace.active_worksheet;
        let loaded = crate::workspace::load_workspace(&workspace, &mut self.shared);
        self.worksheets = loaded
            .into_iter()
            .map(|(name, dock_state)| Worksheet {
                name,
                dock_state,
                window: Default::default(),
            })
            .collect();
        for (ws, saved) in self.worksheets.iter_mut().zip(&workspace.worksheets) {
            ws.window = saved.window.clone();
            #[cfg(target_arch = "wasm32")]
            {
                ws.window.detached = false;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        for ws in &mut self.worksheets {
            while let Some(path) = ws
                .dock_state
                .find_tab_from(|tab| matches!(tab,PanelTab::TrackMap(t) if t.is_popped_out))
            {
                if let Some(PanelTab::TrackMap(map)) = ws.dock_state.remove_tab(path) {
                    self.popped_out_track_maps.retain(|old| old.id != map.id);
                    self.popped_out_track_maps.push(map);
                } else {
                    break;
                }
            }
        }
        self.active_worksheet = active_worksheet.min(self.worksheets.len().saturating_sub(1));
    }

    fn restore_workspace_when_ready(&mut self, workspace: crate::workspace::WorkspaceFile) {
        if let Some(load) = &mut self.pending_session_load {
            load.workspace_snapshot = Some(workspace);
            return;
        }
        crate::workspace::restore_track_sections(&workspace, &mut self.shared);
        if self.shared.are_math_channels_settled() {
            self.apply_workspace_snapshot(workspace);
        } else {
            self.pending_workspace_restore = Some(workspace);
        }
    }

    fn maybe_apply_pending_workspace_restore(&mut self) {
        if self.shared.are_math_channels_settled()
            && let Some(workspace) = self.pending_workspace_restore.take()
        {
            self.apply_workspace_snapshot(workspace);
        }
    }

    pub fn open_file(&mut self, path: PathBuf) {
        let workspace_snapshot = self.workspace_snapshot_for_session_reload();
        match load_session_from_path(path) {
            Ok(loaded) => self.install_loaded_session(loaded, workspace_snapshot),
            Err(err) => self.load_error = Some(format!("Failed to open file: {err}")),
        }
    }

    pub fn open_bytes(
        &mut self,
        file_name: String,
        bytes: Vec<u8>,
        ldx: Option<LdxFile>,
    ) -> Result<(), String> {
        let workspace_snapshot = self.workspace_snapshot_for_session_reload();
        let loaded = load_session_from_bytes(file_name, bytes, ldx)?;
        self.install_loaded_session(loaded, workspace_snapshot);
        Ok(())
    }

    fn install_loaded_session(
        &mut self,
        loaded: LoadedSession,
        workspace_snapshot: Option<crate::workspace::WorkspaceFile>,
    ) {
        self.shared.remember_recording_analysis();
        self.load_error = None;
        self.pending_session_load = None;
        self.pending_workspace_restore = None;
        self.shared.session_id = self.next_session_id;
        self.next_session_id += 1;
        self.shared.file_name = loaded.file_name;
        self.shared.ldx = loaded.ldx;

        let ld = Arc::new(loaded.ld);
        self.shared.laps = loaded.laps;
        self.shared.data_duration = Some(loaded.data_duration);
        self.shared.ld_file = Some(ld);
        self.shared.ld_path = loaded.ld_path;
        self.shared.selected_lap = None;
        self.shared.cursor_time = None;
        self.shared.measurement_cursor = None;
        self.shared.comparisons = Default::default();
        self.shared.playback_active = false;
        self.shared.zoom_range = Some((0.0, loaded.data_duration));
        self.shared.invalidate_session_caches();
        let detected = self.shared.laps.clone();
        if let Some(analysis) = loaded.analysis {
            self.shared
                .recording_analyses
                .entry(analysis.recording.clone())
                .or_insert(analysis);
        }
        if let Some(warning) = loaded.analysis_warning {
            self.shared.message(warning);
        }
        self.shared.activate_recording_analysis(&detected);
        self.shared.sync_drag_math();
        if let Some(selected) = self.shared.selected_lap {
            self.shared.select_lap(Some(selected));
        }

        // Clear all panels' channels and caches across all worksheets
        for ws in &mut self.worksheets {
            for (_path, tab) in ws.dock_state.iter_all_tabs_mut() {
                match tab {
                    PanelTab::Graph(g) => g.reset_for_new_main_session(),
                    PanelTab::TrackMap(t) => t.clear_cache(),
                    PanelTab::Report(r) => r.clear_channels(),
                    PanelTab::Histogram(h) => h.clear_channels(),
                    PanelTab::Scatter(s) => s.clear_channels(),
                    PanelTab::Fft(f) => f.clear_channels(),
                    PanelTab::Gauge(g) => g.clear_channels(),
                    PanelTab::MixtureMap(m) => m.clear_channels(),
                    PanelTab::Calculations(p) => p.channels.clear(),
                    _ => {}
                }
            }
        }
        for tm in &mut self.popped_out_track_maps {
            tm.clear_cache();
        }

        // Re-evaluate math channels with new file data
        math_editor::evaluate_all_math_channels(&mut self.shared);

        // Auto-populate default worksheets if current layout is empty
        let is_empty_default =
            workspace_snapshot.is_none() && is_startup_default_layout(&self.worksheets);

        if is_empty_default {
            let ld_ref = self.shared.ld_file.clone().unwrap();
            let defaults = {
                let _perf = crate::perf_metrics::scope("default layout creation");
                crate::default_layouts::create_default_worksheets(&ld_ref, &mut self.shared)
            };
            if !defaults.is_empty() {
                self.worksheets = defaults
                    .into_iter()
                    .map(|(name, dock_state)| Worksheet {
                        name,
                        dock_state,
                        window: Default::default(),
                    })
                    .collect();
                self.active_worksheet = 0;
            }
        }

        if let Some(workspace) = workspace_snapshot {
            self.restore_workspace_when_ready(workspace);
        }

        if let Some(path) = self.shared.ld_path.clone() {
            self.register_project_session(&path);
        }

        self.egui_ctx.request_repaint();
    }

    fn submit_load_session(
        &mut self,
        source: LoadSessionSource,
        file_label: String,
        ctx: &egui::Context,
    ) {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        self.load_error = None;
        self.pending_workspace_restore = None;
        self.pending_session_load = Some(PendingSessionLoad {
            request_id,
            workspace_snapshot: self.workspace_snapshot_for_session_reload(),
            file_label,
        });

        if let Err(err) = self
            .background_jobs
            .submit(JobRequest::LoadSession { request_id, source }, ctx)
        {
            self.pending_session_load = None;
            self.load_error = Some(err);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_file_async(&mut self, path: PathBuf, ctx: &egui::Context) {
        let file_label = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        self.submit_load_session(LoadSessionSource::Path(path), file_label, ctx);
    }

    #[cfg(target_arch = "wasm32")]
    fn open_bytes_async(
        &mut self,
        file_name: String,
        bytes: Vec<u8>,
        ldx: Option<LdxFile>,
        ctx: &egui::Context,
    ) {
        self.submit_load_session(
            LoadSessionSource::Bytes {
                file_name: file_name.clone(),
                bytes,
                ldx,
            },
            file_name,
            ctx,
        );
    }

    fn start_open_session(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        {
            crate::platform::begin_pick_session(self.web_load_tx.clone(), ctx.clone());
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::platform::begin_pick_session(self.native_pick_tx.clone(), ctx.clone());
        }
    }

    fn process_platform_events(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        while let Ok(event) = self.native_pick_rx.try_recv() {
            match event {
                crate::platform::NativePickEvent::SessionPath(path) => {
                    self.open_file_async(path, ctx);
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        while let Ok(event) = self.web_load_rx.try_recv() {
            match event {
                crate::platform::WebLoadEvent::SessionData {
                    file_name,
                    ld_bytes,
                    ldx_xml,
                } => {
                    let mut ignored_ldx_error = None;
                    let ldx = match ldx_xml {
                        Some(xml) => match LdxFile::parse(&xml) {
                            Ok(ldx) => Some(ldx),
                            Err(err) => {
                                ignored_ldx_error = Some(format!(
                                    "Optional .ldx file could not be parsed and was ignored: {err}"
                                ));
                                None
                            }
                        },
                        None => None,
                    };

                    self.open_bytes_async(file_name, ld_bytes, ldx, ctx);
                    if let Some(err) = ignored_ldx_error {
                        self.load_error = Some(err);
                    }
                }
                crate::platform::WebLoadEvent::Error(err) => {
                    self.load_error = Some(err);
                }
            }
        }

        while let Some(result) = self.background_jobs.try_recv() {
            match result {
                JobResult::LoadSession { request_id, result } => {
                    let Some(pending) = self.pending_session_load.take() else {
                        continue;
                    };
                    if pending.request_id != request_id {
                        self.pending_session_load = Some(pending);
                        continue;
                    }

                    match *result {
                        Ok(loaded) => {
                            self.install_loaded_session(loaded, pending.workspace_snapshot);
                        }
                        Err(err) => {
                            self.shared.message(format!("Failed to open file: {err}"));
                            self.load_error = Some(format!("Failed to open file: {err}"));
                        }
                    }
                }
                JobResult::DecodePhysicalChannel {
                    session_id,
                    channel_idx,
                    result,
                } => {
                    if self.shared.session_id != session_id {
                        continue;
                    }

                    match result {
                        Ok(decoded) => {
                            self.shared.store_decoded_physical_channel(
                                decoded.channel_idx,
                                decoded.data,
                                decoded.stats,
                            );
                            if !self.shared.math_channels.is_empty() {
                                math_editor::reevaluate_math_channels_waiting_on_inputs(
                                    &mut self.shared,
                                );
                            }
                            self.maybe_apply_pending_workspace_restore();
                        }
                        Err(err) => {
                            self.shared.cancel_physical_channel_decode(channel_idx);
                            self.shared
                                .channel_errors
                                .borrow_mut()
                                .insert(channel_idx, err.clone());
                            self.shared.message(err.clone());
                            math_editor::reevaluate_math_channels_waiting_on_inputs(
                                &mut self.shared,
                            );
                            self.load_error = Some(format!("Failed to decode channel: {err}"));
                        }
                    }
                }
                JobResult::BuildTrackData {
                    session_id,
                    correction_key,
                    result,
                } => {
                    if self.shared.session_id != session_id {
                        continue;
                    }
                    if !self
                        .shared
                        .apply_correction_result(&correction_key, *result)
                    {
                        continue;
                    }
                    math_editor::reevaluate_math_channels_waiting_on_inputs(&mut self.shared);
                }
                JobResult::EvaluateMathChannel {
                    session_id,
                    math_id,
                    expression,
                    evaluation_key,
                    result,
                } => {
                    if self.shared.session_id != session_id {
                        continue;
                    }
                    self.shared.complete_math_channel_evaluation(math_id);
                    if math_editor::apply_math_evaluation_result(
                        &mut self.shared,
                        math_id,
                        &expression,
                        &evaluation_key,
                        result,
                    ) {
                        self.shared.invalidate_derived_caches();
                        if !self.shared.math_channels.is_empty() {
                            math_editor::reevaluate_math_channels_waiting_on_inputs(
                                &mut self.shared,
                            );
                        }
                    } else if let Some(idx) = self.shared.math_channel_index_by_id(math_id) {
                        math_editor::queue_math_channel_evaluation(&mut self.shared, idx);
                    }
                    self.maybe_apply_pending_workspace_restore();
                }
            }
        }
    }

    fn submit_requested_channel_decodes(&mut self, ctx: &egui::Context) {
        let Some(ld) = self.shared.ld_file.clone() else {
            return;
        };

        for channel_idx in self.shared.take_requested_physical_channel_decodes() {
            let request_id = self.next_request_id;
            self.next_request_id += 1;

            if let Err(err) = self.background_jobs.submit(
                JobRequest::DecodePhysicalChannel {
                    request_id,
                    session_id: self.shared.session_id,
                    ld: ld.clone(),
                    channel_idx,
                },
                ctx,
            ) {
                self.shared.cancel_physical_channel_decode(channel_idx);
                self.load_error = Some(err);
            }
        }
    }

    fn submit_requested_math_channel_evaluations(&mut self, ctx: &egui::Context) {
        let requested = self.shared.take_requested_math_channel_evaluations();
        if requested.is_empty() {
            return;
        }

        let topo_order = math_editor::topological_eval_order(&self.shared);
        let mut order_pos = HashMap::new();
        for (position, idx) in topo_order.into_iter().enumerate() {
            if let Some(math_id) = self.shared.math_channels.get(idx).map(|mc| mc.id) {
                order_pos.insert(math_id, position);
            }
        }

        let mut requested = requested;
        requested.sort_by_key(|math_id| order_pos.get(math_id).copied().unwrap_or(usize::MAX));

        let mut source_failed = false;
        for math_id in requested {
            let Some(job) = math_editor::build_math_evaluation_job(&mut self.shared, math_id)
            else {
                self.shared.cancel_math_channel_evaluation(math_id);
                source_failed |= self
                    .shared
                    .math_channel_index_by_id(math_id)
                    .is_some_and(|idx| {
                        self.shared.math_channels[idx].evaluation_state
                            == crate::state::MathEvaluationState::Error
                    });
                continue;
            };

            let request_id = self.next_request_id;
            self.next_request_id += 1;

            if let Err(err) = self.background_jobs.submit(
                JobRequest::EvaluateMathChannel {
                    request_id,
                    session_id: self.shared.session_id,
                    math_id: job.math_id,
                    expression: job.expression,
                    evaluation_key: job.evaluation_key,
                    options: job.options,
                    aliases: job.aliases,
                    channel_data: job.channel_data,
                },
                ctx,
            ) {
                self.shared.cancel_math_channel_evaluation(math_id);
                self.load_error = Some(err);
            }
        }
        if source_failed {
            math_editor::reevaluate_math_channels_waiting_on_inputs(&mut self.shared);
            ctx.request_repaint();
        }
        // Disabled/out-of-recording definitions can all settle locally without
        // emitting a background result. Their completion must also restore the
        // saved components after switching Main recordings.
        self.maybe_apply_pending_workspace_restore();
    }

    fn submit_requested_track_data_build(&mut self, ctx: &egui::Context) {
        let Some(ld) = self.shared.ld_file.clone() else {
            return;
        };

        if !self.shared.take_requested_track_data_build() {
            return;
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        if let Err(err) = self.background_jobs.submit(
            JobRequest::BuildTrackData {
                request_id,
                session_id: self.shared.session_id,
                correction_key: self.shared.correction_key(),
                settings: self.shared.current_corrections(),
                laps: self.shared.competition_laps(),
                ld,
            },
            ctx,
        ) {
            self.shared.cancel_track_data_build();
            self.load_error = Some(err);
        }
    }

    fn clear_loaded_session(&mut self) {
        self.shared.remember_recording_analysis();
        self.pending_workspace_restore = None;
        self.shared.ld_file = None;
        self.shared.ld_path = None;
        self.shared.file_name.clear();
        self.shared.laps.clear();
        self.shared.ldx = None;
        self.shared.selected_lap = None;
        self.shared.cursor_time = None;
        self.shared.playback_active = false;
        self.shared.zoom_range = None;
        self.shared.data_duration = None;
        self.shared.invalidate_session_caches();

        for ws in &mut self.worksheets {
            for (_path, tab) in ws.dock_state.iter_all_tabs_mut() {
                match tab {
                    PanelTab::Graph(g) => g.reset_for_new_main_session(),
                    PanelTab::TrackMap(t) => t.clear_cache(),
                    PanelTab::Report(r) => r.clear_channels(),
                    PanelTab::Histogram(h) => h.clear_channels(),
                    PanelTab::Scatter(s) => s.clear_channels(),
                    PanelTab::Fft(f) => f.clear_channels(),
                    PanelTab::Gauge(g) => g.clear_channels(),
                    PanelTab::MixtureMap(m) => m.clear_channels(),
                    _ => {}
                }
            }
        }
        for tm in &mut self.popped_out_track_maps {
            tm.clear_cache();
        }
    }

    fn register_project_session(&mut self, path: &Path) {
        if !self
            .project_sessions
            .iter()
            .any(|existing| existing.path == path)
        {
            self.project_sessions.push(ProjectSessionRecord {
                path: path.to_path_buf(),
                label: String::new(),
                notes: String::new(),
            });
        }
    }

    fn sync_project_sessions_from_workspace(
        &mut self,
        workspace: &crate::workspace::WorkspaceFile,
    ) {
        for path in crate::project::collect_session_paths(workspace) {
            self.register_project_session(&path);
        }
    }

    fn build_workspace_snapshot(&self) -> crate::workspace::WorkspaceFile {
        self.build_workspace_snapshot_with_popped_out_track_maps(true)
    }

    fn collect_known_project_sessions(
        &self,
        workspace: &crate::workspace::WorkspaceFile,
    ) -> Vec<crate::project::ProjectSessionEntry> {
        let mut sessions: Vec<crate::project::ProjectSessionEntry> = self
            .project_sessions
            .iter()
            .map(|record| crate::project::ProjectSessionEntry {
                path: record.path.to_string_lossy().to_string(),
                label: record.label.clone(),
                notes: record.notes.clone(),
            })
            .collect();

        if let Some(path) = self.shared.ld_path.as_ref()
            && !sessions
                .iter()
                .any(|existing| Path::new(&existing.path) == path)
        {
            sessions.push(crate::project::ProjectSessionEntry {
                path: path.to_string_lossy().to_string(),
                label: String::new(),
                notes: String::new(),
            });
        }

        for path in crate::project::collect_session_paths(workspace) {
            if !sessions
                .iter()
                .any(|existing| Path::new(&existing.path) == path.as_path())
            {
                sessions.push(crate::project::ProjectSessionEntry {
                    path: path.to_string_lossy().to_string(),
                    label: String::new(),
                    notes: String::new(),
                });
            }
        }

        sessions
    }

    fn persist_preferences(&self) {
        let preferences = AppPreferences {
            theme: self.theme_choice,
            options: self.options.clone(),
            channel_preferences: self.shared.channel_preferences.clone(),
        };

        #[cfg(target_arch = "wasm32")]
        {
            if let Err(e) = crate::preferences::save_preferences(&preferences) {
                self.shared
                    .message(format!("Failed to save preferences: {}", e));
            }
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::preferences::queue_save(
                preferences,
                self.shared.analysis_messages.clone(),
                self.shared.session_id,
            );
        }
    }

    fn save_project(&mut self) {
        if let Some(existing_path) = self.project_path.clone() {
            let workspace = self.build_workspace_snapshot();
            self.save_project_to_path(existing_path, workspace);
            return;
        }

        let workspace = self.build_workspace_snapshot();
        let suggested_name = self
            .project_path
            .as_ref()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .or_else(|| {
                self.shared.ld_path.as_ref().and_then(|path| {
                    path.file_stem()
                        .map(|stem| format!("{}.i3rsproj", stem.to_string_lossy()))
                })
            })
            .unwrap_or_else(|| "race-weekend.i3rsproj".into());

        if let Some(path) = crate::platform::save_project_file(&suggested_name) {
            self.save_project_to_path(path, workspace);
        }
    }

    fn save_project_to_path(&mut self, path: PathBuf, workspace: crate::workspace::WorkspaceFile) {
        let sessions = self.collect_known_project_sessions(&workspace);
        let project_name = crate::project::project_name_from_path(&path);
        let project = crate::project::ProjectFile::from_workspace(
            project_name.clone(),
            workspace,
            &sessions,
            self.shared.ld_path.as_deref(),
            &path,
        );

        match serde_json::to_string_pretty(&project) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => {
                    self.project_path = Some(path);
                    self.project_name = Some(project_name);
                    self.project_sessions = sessions
                        .into_iter()
                        .map(|session| ProjectSessionRecord {
                            path: PathBuf::from(session.path),
                            label: session.label,
                            notes: session.notes,
                        })
                        .collect();
                }
                Err(e) => self
                    .shared
                    .message(format!("Failed to save project: {}", e)),
            },
            Err(e) => self
                .shared
                .message(format!("Failed to serialize project: {}", e)),
        }
    }

    fn load_project(&mut self) {
        if let Some(path) = crate::platform::pick_project_file() {
            self.load_project_from_path(path);
        }
    }

    fn load_project_from_path(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<crate::project::ProjectFile>(&json) {
                Ok(mut project) => {
                    crate::project::resolve_project_paths(&mut project, &path);
                    let project_name = project.effective_name(&path);
                    let active_session = project
                        .active_session_path
                        .as_ref()
                        .map(PathBuf::from)
                        .or_else(|| project.workspace.last_file_path.as_ref().map(PathBuf::from));

                    self.project_path = Some(path);
                    self.project_name = Some(project_name);
                    self.project_sessions = project
                        .sessions
                        .iter()
                        .map(|session| ProjectSessionRecord {
                            path: PathBuf::from(&session.path),
                            label: session.label.clone(),
                            notes: session.notes.clone(),
                        })
                        .collect();
                    self.session_summary_cache.clear();

                    self.clear_loaded_session();
                    if let Some(active_session) = active_session {
                        if active_session.exists() {
                            self.open_file(active_session);
                        } else {
                            self.shared.message(format!(
                                "Project session file not found: {}",
                                active_session.display()
                            ));
                            self.register_project_session(&active_session);
                        }
                    }

                    // Load channel aliases from project workspace.
                    self.shared.channel_aliases.clear();
                    for alias_config in &project.workspace.channel_aliases {
                        self.shared
                            .channel_aliases
                            .insert(alias_config.alias.clone(), alias_config.target.clone());
                    }

                    self.shared.channel_settings = project.workspace.channel_settings.clone();
                    // Load math channels from project workspace.
                    self.shared.math_channels.clear();
                    for config in &project.workspace.math_channels {
                        let math_channel = config.create_definition(&mut self.shared);
                        self.shared.math_channels.push(math_channel);
                    }
                    math_editor::evaluate_all_math_channels(&mut self.shared);
                    self.shared.invalidate_derived_caches();

                    let workspace = project.workspace;
                    self.sync_project_sessions_from_workspace(&workspace);
                    self.pending_workspace_restore = None;
                    self.restore_workspace_when_ready(workspace);
                }
                Err(e) => self
                    .shared
                    .message(format!("Failed to parse project: {}", e)),
            },
            Err(e) => self
                .shared
                .message(format!("Failed to read project file: {}", e)),
        }
    }

    fn switch_project_session(&mut self, path: PathBuf) {
        self.open_file(path);
    }

    fn current_session_summary(&self) -> Option<SessionSummary> {
        let ld = self.shared.ld_file.as_ref()?;
        Some(SessionSummary {
            file_name: self.shared.file_name.clone(),
            date: ld.session.date.clone(),
            time: ld.session.time.clone(),
            driver: self
                .shared
                .current_analysis()
                .map_or_else(|| ld.session.driver.clone(), |a| a.details.driver.clone()),
            vehicle_id: self.shared.current_analysis().map_or_else(
                || ld.session.vehicle_id.clone(),
                |a| a.details.vehicle.clone(),
            ),
            venue: self
                .shared
                .current_analysis()
                .map_or_else(|| ld.session.venue.clone(), |a| a.details.venue.clone()),
            event_name: self
                .shared
                .current_analysis()
                .map_or_else(|| ld.event.event_name.clone(), |a| a.details.event.clone()),
            session_name: ld.event.session.clone(),
            comment: if ld.event.comment.is_empty() {
                ld.session.short_comment.clone()
            } else {
                ld.event.comment.clone()
            },
            duration_secs: ld.duration_secs(),
            channel_count: ld.channels.len(),
            lap_count: self.shared.laps.len(),
        })
    }

    pub fn loaded_session_summary(&self) -> Option<LoadedSessionSummary> {
        let summary = self.current_session_summary()?;
        Some(LoadedSessionSummary {
            file_name: summary.file_name,
            channel_count: summary.channel_count,
            lap_count: summary.lap_count,
        })
    }

    /// Prepare the bundled web sample after its background corrections finish.
    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn configure_vir_sample(&mut self) -> Result<bool, String> {
        if self.shared.file_name != "VIR_LAP.ld" {
            return Err("Load the bundled VIR sample before configuring its track".into());
        }
        if let Some(error) = &self.shared.correction_error {
            return Err(error.clone());
        }
        let Some(track) = self.shared.track_data_if_ready() else {
            if self.shared.corrections_settled() {
                return Err("The bundled sample has no usable GPS track".into());
            }
            self.shared.request_track_data_build();
            self.egui_ctx.request_repaint();
            return Ok(false);
        };
        let lap_index = self
            .shared
            .current_analysis()
            .and_then(|analysis| {
                analysis
                    .ranges
                    .iter()
                    .enumerate()
                    .filter(|(_, range)| range.kind == i3rs_core::ranges::RangeKind::Lap)
                    .max_by(|(_, a), (_, b)| (a.end - a.start).total_cmp(&(b.end - b.start)))
                    .map(|(index, _)| index)
            })
            .ok_or("The bundled sample has no complete lap")?;
        let sections = i3rs_core::track::sectors_from_distance_definition(
            &track,
            &self.shared.laps[lap_index],
            &i3rs_core::track::vir_full_definition(),
        )?;
        self.shared.select_lap(Some(lap_index));
        self.shared.sectors = sections;
        for worksheet in &mut self.worksheets {
            for (_, tab) in worksheet.dock_state.iter_all_tabs_mut() {
                if let PanelTab::TrackMap(map) = tab {
                    map.show_annotations = true;
                    map.annotation_settings = Default::default();
                }
            }
        }
        self.egui_ctx.request_repaint();
        Ok(true)
    }

    fn session_summary_for_path(&mut self, path: &Path) -> Option<SessionSummary> {
        if self
            .shared
            .ld_path
            .as_ref()
            .is_some_and(|current| current == path)
        {
            return self.current_session_summary();
        }
        if let Some(summary) = self.session_summary_cache.get(path) {
            return Some(summary.clone());
        }

        let ld = Arc::new(LdFile::open(path).ok()?);
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let lap_count = detect_laps(&ld).len();
        let summary = SessionSummary {
            file_name,
            date: ld.session.date.clone(),
            time: ld.session.time.clone(),
            driver: ld.session.driver.clone(),
            vehicle_id: ld.session.vehicle_id.clone(),
            venue: ld.session.venue.clone(),
            event_name: ld.event.event_name.clone(),
            session_name: ld.event.session.clone(),
            comment: if ld.event.comment.is_empty() {
                ld.session.short_comment.clone()
            } else {
                ld.event.comment.clone()
            },
            duration_secs: ld.duration_secs(),
            channel_count: ld.channels.len(),
            lap_count,
        };
        self.session_summary_cache
            .insert(path.to_path_buf(), summary.clone());
        Some(summary)
    }

    fn show_theme_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut changed = false;
        changed |= ui
            .radio_value(&mut self.theme_choice, ThemeChoice::System, "System theme")
            .clicked();
        changed |= ui
            .radio_value(&mut self.theme_choice, ThemeChoice::Light, "Light theme")
            .clicked();
        changed |= ui
            .radio_value(&mut self.theme_choice, ThemeChoice::Dark, "Dark theme")
            .clicked();
        changed |= ui
            .radio_value(
                &mut self.theme_choice,
                ThemeChoice::HighContrast,
                "High-contrast theme",
            )
            .clicked();
        if changed {
            apply_theme_to_ctx(ctx, self.theme_choice);
            self.options.apply(ctx);
            self.persist_preferences();
        }
    }

    fn show_channel_preferences_window(&mut self, ctx: &egui::Context) {
        if !self.show_channel_preferences {
            return;
        }

        let mut open = self.show_channel_preferences;
        egui::Window::new("Global Channel Preferences")
            .open(&mut open)
            .default_size([720.0, 360.0])
            .show(ctx, |ui| {
                if self.shared.channel_preferences.is_empty() {
                    ui.label("No global channel preferences saved yet.");
                    ui.label(
                        "Right-click a plotted channel and choose \"Save current style as global default\".",
                    );
                    return;
                }

                ui.label("Saved defaults apply to newly plotted channels across sessions and projects.");
                ui.separator();

                let mut keys: Vec<_> = self.shared.channel_preferences.keys().cloned().collect();
                keys.sort();
                let mut to_remove = None;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("channel_preferences_grid")
                        .num_columns(6)
                        .spacing([12.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Channel");
                            ui.strong("Color");
                            ui.strong("Scale");
                            ui.strong("Offset");
                            ui.strong("Unit");
                            ui.strong("");
                            ui.end_row();

                            for key in keys {
                                let Some(pref) = self.shared.channel_preferences.get(&key) else {
                                    continue;
                                };
                                ui.monospace(&key);
                                if let Some(color) = pref.color {
                                    let swatch = egui::Color32::from_rgb(color[0], color[1], color[2]);
                                    ui.colored_label(swatch, "■■■");
                                } else {
                                    ui.label("-");
                                }
                                ui.monospace(format!("{:.4}", pref.display_scale));
                                ui.monospace(format!("{:.4}", pref.display_offset));
                                ui.label(pref.display_unit.as_deref().unwrap_or("raw"));
                                if ui.small_button("Remove").clicked() {
                                    to_remove = Some(key.clone());
                                }
                                ui.end_row();
                            }
                        });
                });

                if let Some(key) = to_remove {
                    self.shared.channel_preferences.remove(&key);
                    self.shared.channel_preferences_dirty = true;
                    self.persist_preferences();
                }
            });
        self.show_channel_preferences = open;
    }

    fn show_session_details_window(&mut self, ctx: &egui::Context) {
        if !self.show_session_details {
            return;
        }

        let current_path = self.shared.ld_path.clone();
        let current_summary = self.current_session_summary();
        let mut open = self.show_session_details;

        egui::Window::new("Session Details")
            .open(&mut open)
            .default_size([900.0, 480.0])
            .show(ctx, |ui| {
                if current_summary.is_none() {
                    ui.label("Open a session to inspect and compare session details.");
                    return;
                }

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.project_sessions.is_empty() || self.project_path.is_some(),
                            egui::Button::new("Save Project"),
                        )
                        .clicked()
                    {
                        self.save_project();
                    }
                    ui.label("Session labels and notes are stored in the project file.");
                });
                if ui
                    .button("Edit log details, ranges and corrections")
                    .clicked()
                {
                    self.shared.show_recording_editor = true;
                }
                ui.separator();

                if let Some(current_path) = current_path.as_ref()
                    && let Some(record) = self
                        .project_sessions
                        .iter_mut()
                        .find(|record| &record.path == current_path)
                {
                    ui.label("Current session metadata and project notes");
                    ui.horizontal(|ui| {
                        ui.label("Display label:");
                        ui.text_edit_singleline(&mut record.label);
                    });
                    ui.label("Notes:");
                    ui.text_edit_multiline(&mut record.notes);
                    ui.separator();
                }

                if self.project_sessions.len() > 1 {
                    let selected_text = self
                        .compare_session_path
                        .as_ref()
                        .and_then(|path| {
                            self.project_sessions
                                .iter()
                                .find(|record| &record.path == path)
                                .map(session_record_label)
                        })
                        .unwrap_or_else(|| "Choose comparison session".into());
                    egui::ComboBox::from_id_salt("session_compare_combo")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for record in &self.project_sessions {
                                if current_path
                                    .as_ref()
                                    .is_some_and(|current| current == &record.path)
                                {
                                    continue;
                                }
                                let label = session_record_label(record);
                                if ui
                                    .selectable_label(
                                        self.compare_session_path
                                            .as_ref()
                                            .is_some_and(|selected| selected == &record.path),
                                        label,
                                    )
                                    .clicked()
                                {
                                    self.compare_session_path = Some(record.path.clone());
                                }
                            }
                        });
                    ui.separator();
                }

                let compare_summary = self.compare_session_path.clone().and_then(|path| {
                    self.session_summary_for_path(&path)
                        .map(|summary| (path, summary))
                });

                ui.columns(2, |columns| {
                    let current_record = current_path.as_ref().and_then(|path| {
                        self.project_sessions
                            .iter()
                            .find(|record| &record.path == path)
                    });
                    Self::show_session_summary_column(
                        &mut columns[0],
                        "Current Session",
                        current_summary.as_ref().unwrap(),
                        current_record,
                    );

                    if let Some((path, summary)) = compare_summary.as_ref() {
                        let compare_record = self
                            .project_sessions
                            .iter()
                            .find(|record| &record.path == path);
                        Self::show_session_summary_column(
                            &mut columns[1],
                            "Comparison Session",
                            summary,
                            compare_record,
                        );
                    } else {
                        columns[1]
                            .label("Choose another project session for side-by-side comparison.");
                    }
                });
            });

        self.show_session_details = open;
    }

    fn show_session_summary_column(
        ui: &mut egui::Ui,
        title: &str,
        summary: &SessionSummary,
        record: Option<&ProjectSessionRecord>,
    ) {
        ui.strong(title);
        ui.separator();
        egui::Grid::new(title)
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                Self::summary_row(ui, "File", &summary.file_name);
                Self::summary_row(
                    ui,
                    "Label",
                    record
                        .map(session_record_label)
                        .unwrap_or_else(|| summary.file_name.clone())
                        .as_str(),
                );
                Self::summary_row(ui, "Date", &summary.date);
                Self::summary_row(ui, "Time", &summary.time);
                Self::summary_row(ui, "Driver", &summary.driver);
                Self::summary_row(ui, "Vehicle", &summary.vehicle_id);
                Self::summary_row(ui, "Venue", &summary.venue);
                Self::summary_row(ui, "Event", &summary.event_name);
                Self::summary_row(ui, "Session", &summary.session_name);
                Self::summary_row(
                    ui,
                    "Duration",
                    &i3rs_core::format_duration(summary.duration_secs),
                );
                Self::summary_row(ui, "Channels", &summary.channel_count.to_string());
                Self::summary_row(ui, "Laps", &summary.lap_count.to_string());
                Self::summary_row(ui, "Comment", &summary.comment);
                Self::summary_row(
                    ui,
                    "Notes",
                    record.map(|record| record.notes.as_str()).unwrap_or(""),
                );
            });
    }

    fn summary_row(ui: &mut egui::Ui, label: &str, value: &str) {
        ui.label(label);
        ui.label(if value.is_empty() { "-" } else { value });
        ui.end_row();
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let mut bindings = self.options.shortcuts.clone();
        bindings.sort_by_key(|b| std::cmp::Reverse((b.shift as u8) + (b.alt as u8)));
        for binding in bindings {
            let Some(shortcut) = binding.shortcut() else {
                continue;
            };
            if ctx.egui_wants_keyboard_input() && !binding.command {
                continue;
            }
            if !ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                continue;
            }
            match binding.action.as_str() {
                "Open" => self.start_open_session(ctx),
                "Save project" => self.save_project(),
                "Load project" => self.load_project(),
                "Save workspace" => self.save_workspace(),
                "Channels" => self.show_channel_browser = !self.show_channel_browser,
                "Readout" => self.show_cursor_readout = !self.show_cursor_readout,
                "Maths" => self.show_math_editor = !self.show_math_editor,
                "Details" => self.show_session_details = true,
                "Options" => self.show_options = true,
                "Help" => self.show_help = true,
                "Graph" => self.add_graph_panel(),
                "Track map" => self.add_track_map_panel(),
                "Histogram" => self.add_histogram_panel(),
                "Scatter" => self.add_scatter_panel(),
                "FFT" => self.add_fft_panel(),
                "Previous worksheet" => {
                    self.active_worksheet =
                        (self.active_worksheet + self.worksheets.len() - 1) % self.worksheets.len()
                }
                "Next worksheet" => {
                    self.active_worksheet = (self.active_worksheet + 1) % self.worksheets.len()
                }
                _ => {}
            }
        }
    }

    fn show_options_and_help(&mut self, ctx: &egui::Context) {
        let l = self.options.language;
        let mut open = self.show_options;
        let before = serde_json::to_string(&self.options).unwrap_or_default();
        egui::Window::new(l.text(
            "Application options",
            "Options de l’application",
            "アプリケーション設定",
        ))
        .id(egui::Id::new("application-options"))
        .open(&mut open)
        .vscroll(true)
        .show(ctx, |ui| {
            self.options.controls(ui);
            ui.separator();
            ui.collapsing(
                l.text(
                    "Analysis and circuit",
                    "Analyse et circuit",
                    "解析とサーキット",
                ),
                |ui| {
                    ui.checkbox(
                        &mut self.shared.show_lap_markers,
                        l.text("Lap markers", "Repères de tours", "ラップマーカー"),
                    );
                    ui.checkbox(
                        &mut self.show_cursor_readout,
                        l.text("Cursor readout", "Valeurs du curseur", "カーソル値"),
                    );
                    if ui
                        .button(l.text(
                            "Ranges, corrections and details",
                            "Plages, corrections et détails",
                            "範囲・補正・詳細",
                        ))
                        .clicked()
                    {
                        self.shared.show_recording_editor = true;
                    }
                },
            );
            if ui
                .button(l.text(
                    "Channel preferences",
                    "Préférences des canaux",
                    "チャンネル設定",
                ))
                .clicked()
            {
                self.show_channel_preferences = true;
            }
            if ui.button(l.text("Help", "Aide", "ヘルプ")).clicked() {
                self.show_help = true;
            }
        });
        self.show_options = open;
        if serde_json::to_string(&self.options).unwrap_or_default() != before {
            apply_theme_to_ctx(ctx, self.theme_choice);
            self.options.apply(ctx);
            self.persist_preferences();
        }
        egui::Window::new(l.text("Help","Aide","ヘルプ")).open(&mut self.show_help).vscroll(true).show(ctx,|ui| {
            ui.label(l.text("Open a log, choose a lap, then drag channels onto a component. Right-click a channel for units, scales and display settings.","Ouvrez un enregistrement, choisissez un tour et glissez des canaux sur un composant. Le clic droit règle les unités, échelles et l’affichage.","ログを開き、ラップを選び、チャンネルをパネルへドラッグします。右クリックで単位・スケール・表示を設定できます。"));
            ui.label(l.text("Graphs share Main and Overlay selections. Drag to zoom; use the graph commands for precise ranges. Pin cursor B in Readout for interval statistics. Overview and local views can follow their own ranges.","Les graphiques partagent les sélections Main et Overlay. Utilisez les commandes pour une plage précise et le curseur B pour les statistiques d’intervalle.","グラフは Main と Overlay を共有します。範囲操作でズームし、Readout の B カーソルで区間統計を表示します。"));
            ui.label(l.text("Distributions use native finite samples in half-open ranges. Positive gate values include samples; missing gates exclude all. Hiding a colour band changes presentation only.","Les distributions utilisent les échantillons natifs finis. Une valeur de filtre positive inclut l’échantillon. Masquer une bande ne change pas les totaux.","分布は範囲内の有効な元サンプルを使用します。ゲートが正なら含め、欠損なら除外します。色帯の非表示は集計値を変えません。"));
            ui.separator();
            for binding in &self.options.shortcuts {ui.label(format!("{}: {}{}{}{}",binding.action,if binding.command{"Command+"}else{""},if binding.shift{"Shift+"}else{""},if binding.alt{"Alt+"}else{""},binding.key));}
        });
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
        let mut report = ReportPanel::new(id, format!("Report {}", id));
        report.copy_visible_channels(&self.shared);
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Report(report));
    }

    fn add_track_map_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let mut track_map = TrackMapPanel::new(id, format!("Track Map {}", id));
        track_map.home_worksheet = self.active_worksheet;
        // Start docked; pop-out remains an explicit toolbar action.
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::TrackMap(track_map));
    }

    fn add_histogram_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let histogram = HistogramPanel::new(id, format!("Histogram {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Histogram(histogram));
    }

    fn add_scatter_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let scatter = ScatterPanel::new(id, format!("Scatter {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Scatter(Box::new(scatter)));
    }

    fn add_fft_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let fft = FftPanel::new(id, format!("FFT {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Fft(fft));
    }

    fn add_gauge_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let gauge = GaugePanel::new(id, format!("Gauges {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::Gauge(gauge));
    }

    fn add_mixture_map_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let mixture_map = MixtureMapPanel::new(id, format!("Mixture Map {}", id));
        self.worksheets[self.active_worksheet]
            .dock_state
            .push_to_focused_leaf(PanelTab::MixtureMap(Box::new(mixture_map)));
    }

    fn add_worksheet(&mut self) {
        let idx = self.worksheets.len() + 1;
        let dock_state = Self::default_dock_state(&mut self.shared);
        self.worksheets.push(Worksheet {
            window: Default::default(),
            name: format!("Sheet {}", idx),
            dock_state,
        });
        self.active_worksheet = self.worksheets.len() - 1;
    }

    fn save_workspace(&self) {
        let workspace = self.build_workspace_snapshot();

        if let Ok(json) = serde_json::to_string_pretty(&workspace)
            && let Some(path) = crate::platform::save_workspace_file()
            && let Err(e) = std::fs::write(&path, json)
        {
            self.shared
                .message(format!("Failed to save workspace: {}", e));
        }
    }

    fn use_workspace_profile(&mut self, profile: crate::default_layouts::WorkspaceProfile) {
        self.shared.workspace_profile = Some(profile);
        let Some(ld) = self.shared.ld_file.clone() else {
            return;
        };
        self.pending_workspace_restore = None;
        self.popped_out_track_maps.clear();
        self.worksheets =
            crate::default_layouts::create_profile_worksheets(profile, &ld, &mut self.shared)
                .into_iter()
                .map(|(name, dock_state)| Worksheet {
                    name,
                    dock_state,
                    window: Default::default(),
                })
                .collect();
        self.active_worksheet = 0;
        let snapshot = self.workspace_snapshot_for_session_reload();
        if let Some(load) = &mut self.pending_session_load {
            load.workspace_snapshot = snapshot;
        }
        self.egui_ctx.request_repaint();
    }

    fn load_workspace(&mut self) {
        if let Some(path) = crate::platform::pick_workspace_file() {
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
                            self.shared
                                .channel_aliases
                                .insert(alias_config.alias.clone(), alias_config.target.clone());
                        }

                        self.shared.channel_settings = workspace.channel_settings.clone();
                        // Load math channels from workspace
                        self.shared.math_channels.clear();
                        for config in &workspace.math_channels {
                            let math_channel = config.create_definition(&mut self.shared);
                            self.shared.math_channels.push(math_channel);
                        }
                        math_editor::evaluate_all_math_channels(&mut self.shared);
                        self.shared.invalidate_derived_caches();

                        self.sync_project_sessions_from_workspace(&workspace);
                        self.pending_workspace_restore = None;
                        self.restore_workspace_when_ready(workspace);
                    }
                    Err(e) => self
                        .shared
                        .message(format!("Failed to parse workspace: {}", e)),
                },
                Err(e) => self
                    .shared
                    .message(format!("Failed to read workspace file: {}", e)),
            }
        }
    }

    fn show_menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(
                self.options.language.text("File", "Fichier", "ファイル"),
                |ui| {
                    if ui.button("Open .ld file...").clicked() {
                        self.start_open_session(ctx);
                        ui.close();
                    }
                    if ui.button("Log Browser…").clicked() {
                        self.output_ui.browser_open = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Workspace Profile", |ui| {
                        for profile in crate::default_layouts::WorkspaceProfile::ALL {
                            if ui
                                .selectable_label(
                                    self.shared.workspace_profile == Some(profile),
                                    profile.label(),
                                )
                                .clicked()
                            {
                                self.use_workspace_profile(profile);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Print / Preview visible worksheet…").clicked() {
                        self.shared.playback_active = false;
                        self.output_ui.capture_requested = true;
                        ctx.request_repaint();
                        ui.close();
                    }
                    if ui.button("Save Workspace...").clicked() {
                        self.save_workspace();
                        ui.close();
                    }
                    if ui.button("Save Project...").clicked() {
                        self.save_project();
                        ui.close();
                    }
                    if ui.button("Load Workspace...").clicked() {
                        self.load_workspace();
                        ui.close();
                    }
                    if ui.button("Load Project...").clicked() {
                        self.load_project();
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
                            self.shared.ld_file.is_some(),
                            egui::Button::new("Export Data / Split Log…"),
                        )
                        .clicked()
                    {
                        self.output_ui.begin_export(&self.shared);
                        ui.close();
                    }
                },
            );
            ui.menu_button(
                self.options.language.text("View", "Affichage", "表示"),
                |ui| {
                    if ui.button("Add Graph Panel").clicked() {
                        self.add_graph_panel();
                        ui.close();
                    }
                    if ui.button("Add Report Panel").clicked() {
                        self.add_report_panel();
                        ui.close();
                    }
                    if ui.button("Add Acceleration Report").clicked() {
                        let id = self.shared.next_panel_id;
                        self.shared.next_panel_id += 1;
                        let mut report = ReportPanel::new(id, "Acceleration Report");
                        report.settings.kind = crate::panels::report::ReportKind::Acceleration;
                        report.settings.view = crate::panels::report::ReportView::Split;
                        self.worksheets[self.active_worksheet]
                            .dock_state
                            .push_to_focused_leaf(PanelTab::Report(report));
                        ui.close();
                    }
                    if ui.button("Add Time / Section Report").clicked() {
                        let id = self.shared.next_panel_id;
                        self.shared.next_panel_id += 1;
                        let mut report = ReportPanel::new(id, "Time / Section Report");
                        report.settings.kind = crate::panels::report::ReportKind::Time;
                        report.settings.scope = crate::panels::report::ReportScope::Ranges;
                        report.settings.statistics =
                            vec![i3rs_core::report::ReportStatistic::Average];
                        self.worksheets[self.active_worksheet]
                            .dock_state
                            .push_to_focused_leaf(PanelTab::Report(report));
                        ui.close();
                    }
                    if ui.button("Add Track Map").clicked() {
                        self.add_track_map_panel();
                        ui.close();
                    }
                    if ui.button("Add Histogram").clicked() {
                        self.add_histogram_panel();
                        ui.close();
                    }
                    if ui.button("Add Scatter Plot").clicked() {
                        self.add_scatter_panel();
                        ui.close();
                    }
                    if ui.button("Add FFT").clicked() {
                        self.add_fft_panel();
                        ui.close();
                    }
                    if ui.button("Add Gauges").clicked() {
                        self.add_gauge_panel();
                        ui.close();
                    }
                    if ui.button("Add Interval / Suspension Analysis").clicked() {
                        let id = self.shared.next_panel_id;
                        self.shared.next_panel_id += 1;
                        self.worksheets[self.active_worksheet]
                            .dock_state
                            .push_to_focused_leaf(PanelTab::Calculations(
                                crate::panels::calculations::CalculationsPanel::new(
                                    id,
                                    "Calculations",
                                ),
                            ));
                        ui.close();
                    }
                    if ui.button("Add Mixture Map").clicked() {
                        self.add_mixture_map_panel();
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
                    let mut current_x_axis = None;
                    for (_path, tab) in dock.iter_all_tabs() {
                        if let PanelTab::Graph(g) = tab {
                            current_mode = Some(g.graph_mode);
                            current_x_axis = Some(g.x_axis_mode);
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

                    if let Some(mut x_axis_mode) = current_x_axis {
                        ui.separator();
                        ui.label("Graph X-axis");
                        let changed_time = ui
                            .radio_value(&mut x_axis_mode, crate::state::GraphXAxis::Time, "Time")
                            .clicked();
                        let changed_distance = ui
                            .radio_value(
                                &mut x_axis_mode,
                                crate::state::GraphXAxis::Distance,
                                "Distance",
                            )
                            .clicked();
                        if changed_time || changed_distance {
                            for (_path, tab) in dock.iter_all_tabs_mut() {
                                if let PanelTab::Graph(g) = tab {
                                    g.x_axis_mode = x_axis_mode;
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
                    ui.checkbox(
                        &mut self.shared.show_channel_status,
                        "Channel Status / Messages",
                    );
                    ui.checkbox(
                        &mut self.shared.show_recording_editor,
                        "Ranges / Corrections / Details",
                    );
                    if ui.button("Drag setup / Run time slips…").clicked() {
                        self.shared.show_recording_editor = true;
                        ui.close();
                    }
                    if ui.button("Session Details").clicked() {
                        self.show_session_details = true;
                        ui.close();
                    }
                    if ui.button("Global Channel Preferences").clicked() {
                        self.show_channel_preferences = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(self.options.language.text(
                            "Application options",
                            "Options",
                            "設定",
                        ))
                        .clicked()
                    {
                        self.show_options = true;
                        ui.close();
                    }
                    if ui
                        .button(self.options.language.text("Help", "Aide", "ヘルプ"))
                        .clicked()
                    {
                        self.show_help = true;
                        ui.close();
                    }
                    ui.label("Theme");
                    self.show_theme_menu(ui, ctx);
                },
            );
        });
    }

    fn show_session_info(&mut self, ui: &mut egui::Ui) {
        if self.shared.ld_file.is_some() {
            ui.horizontal(|ui| {
                if ui
                    .button(if self.shared.playback_active {
                        "Pause"
                    } else {
                        "Play"
                    })
                    .clicked()
                {
                    if !self.shared.playback_active {
                        let range = self.shared.analysis_range();
                        self.shared.clamp_cursor_to(range);
                    }
                    self.shared.playback_active = !self.shared.playback_active;
                }
                ui.add(
                    egui::DragValue::new(&mut self.shared.playback_speed)
                        .range(0.1..=16.0)
                        .speed(0.1)
                        .suffix("×"),
                );
                ui.label("Playback follows lap / zoom");
            });
        }
        if let Some(ld) = &self.shared.ld_file {
            let s = &ld.session;
            let current_session = self.shared.ld_path.as_ref();
            let project_sessions = &self.project_sessions;
            let mut switch_to = None;

            ui.horizontal_wrapped(|ui| {
                if let Some(project_name) = self.project_name.as_ref() {
                    ui.label(format!("Project: {}", project_name));
                    if !project_sessions.is_empty() {
                        ui.separator();
                        ui.label(format!("{} sessions", project_sessions.len()));
                    }
                    ui.separator();
                } else if project_sessions.len() > 1 {
                    ui.label(format!("Session set: {}", project_sessions.len()));
                    ui.separator();
                }

                if project_sessions.len() > 1 {
                    let selected_text = current_session
                        .and_then(|path| {
                            project_sessions
                                .iter()
                                .find(|record| &record.path == path)
                                .map(session_record_label)
                        })
                        .unwrap_or_else(|| "Select session".into());

                    egui::ComboBox::from_id_salt("project_session_switcher")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for record in project_sessions {
                                let label = session_record_label(record);
                                let is_selected =
                                    current_session.is_some_and(|current| current == &record.path);
                                if ui.selectable_label(is_selected, label).clicked() {
                                    switch_to = Some(record.path.clone());
                                    ui.close();
                                }
                            }
                        });
                    ui.separator();
                }

                ui.strong(&self.shared.file_name);
                ui.separator();
                ui.label(format!("{} {}", s.date, s.time));
                ui.separator();
                ui.label(
                    self.shared
                        .current_analysis()
                        .map_or(s.venue.as_str(), |a| a.details.venue.as_str()),
                );
                ui.separator();
                ui.label(
                    self.shared
                        .current_analysis()
                        .map_or(s.vehicle_id.as_str(), |a| a.details.vehicle.as_str()),
                );
                ui.separator();
                ui.label(i3rs_core::format_duration(ld.duration_secs()));
                ui.separator();
                ui.label(format!("{} channels", ld.channels.len()));
                if !self.shared.laps.is_empty() {
                    ui.separator();
                    ui.label(format!("{} ranges", self.shared.laps.len()));
                }
                if !self.shared.math_channels.is_empty() {
                    ui.separator();
                    ui.label(format!("{} math", self.shared.math_channels.len()));
                }
            });

            if let Some(path) = switch_to
                && current_session.is_none_or(|current| current != &path)
            {
                self.switch_project_session(path);
            }
        }
    }

    fn show_worksheet_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut new_active = self.active_worksheet;
            if ui.small_button("‹").clicked() {
                new_active =
                    (self.active_worksheet + self.worksheets.len() - 1) % self.worksheets.len();
            }
            for (i, ws) in self.worksheets.iter_mut().enumerate() {
                let selected = i == self.active_worksheet;
                let response = ui.selectable_label(selected, &ws.name);
                if response.clicked() {
                    new_active = i;
                }
                response.context_menu(|ui| {
                    ui.label("Worksheet name");
                    ui.text_edit_singleline(&mut ws.name);
                });
            }
            if ui.small_button("›").clicked() {
                new_active = (self.active_worksheet + 1) % self.worksheets.len();
            }
            self.active_worksheet = new_active;
            #[cfg(not(target_arch = "wasm32"))]
            if ui
                .small_button(if self.worksheets[new_active].window.detached {
                    "Dock worksheet"
                } else {
                    "Detach worksheet"
                })
                .clicked()
            {
                self.worksheets[new_active].window.detached =
                    !self.worksheets[new_active].window.detached;
            }

            if ui.small_button("+").clicked() {
                self.add_worksheet();
            }
        });
    }

    fn draw_dragging_channel_badge(&self, ctx: &egui::Context) {
        let Some(channel_id) = self.shared.dragging_channel else {
            return;
        };
        let Some(pointer_pos) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };

        let (name, unit, _, _) = resolve_channel_meta(channel_id, &self.shared);
        let label = if unit.is_empty() {
            name
        } else {
            format!("{name} [{unit}]")
        };

        let layer_id = egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("dragging_channel_badge"),
        );
        let painter = ctx.layer_painter(layer_id);
        let font = egui::FontId::proportional(14.0);
        let galley = painter.layout_no_wrap(label, font.clone(), egui::Color32::WHITE);
        let padding = egui::vec2(10.0, 6.0);
        let badge_rect = egui::Rect::from_min_size(
            pointer_pos + egui::vec2(14.0, 14.0),
            galley.size() + padding * 2.0,
        );

        painter.rect_filled(
            badge_rect,
            8.0,
            egui::Color32::from_rgba_premultiplied(18, 18, 24, 230),
        );
        painter.rect_stroke(
            badge_rect,
            8.0,
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(110)),
            egui::StrokeKind::Outside,
        );
        painter.galley(badge_rect.min + padding, galley, egui::Color32::WHITE);
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
        if std::mem::take(&mut self.output_ui.capture_requested) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    "i3rs-print".to_owned(),
                )));
        }
        crate::perf_metrics::maybe_log_summary();
        self.process_platform_events(ui.ctx());
        self.submit_requested_channel_decodes(ui.ctx());
        self.submit_requested_math_channel_evaluations(ui.ctx());
        self.submit_requested_track_data_build(ui.ctx());
        self.handle_shortcuts(ui.ctx());
        self.shared
            .advance_playback(ui.input(|i| i.stable_dt) as f64);
        if self.shared.playback_active {
            ui.ctx().request_repaint();
        }

        // Native drops expose filesystem paths. Web loading uses bytes from the picker;
        // egui's browser drop paths are only filenames, not readable local paths.
        #[cfg(not(target_arch = "wasm32"))]
        let dropped_path = ui.ctx().input(|i| {
            i.raw.dropped_files.first().and_then(|dropped| {
                let path = dropped.path();
                path.extension()
                    .is_some_and(|ext| ext == "ld")
                    .then(|| path.to_path_buf())
            })
        });
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = dropped_path {
            self.open_file_async(path, ui.ctx());
        }

        // Swap channel registries: move current frame's data to display buffer,
        // then clear current for the new frame's graph panels to populate.
        std::mem::swap(
            &mut self.shared.plotted_channel_registry,
            &mut self.shared.display_channel_registry,
        );
        self.shared.plotted_channel_registry.clear();

        if self.shared.channel_preferences_dirty {
            self.persist_preferences();
            self.shared.channel_preferences_dirty = false;
        }

        // Top menu bar
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            self.show_menu_bar(ui, &ctx);
        });

        let mut dismiss_load_error = false;
        if let Some(err) = &self.load_error {
            egui::Panel::top("load_error").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                    if ui.small_button("Dismiss").clicked() {
                        dismiss_load_error = true;
                    }
                });
            });
        }
        if dismiss_load_error {
            self.load_error = None;
        }

        if let Some(pending) = &self.pending_session_load {
            egui::Panel::top("load_status").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading {}…", pending.file_label));
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        if self.shared.ld_file.is_none() {
            egui::Panel::top("web_open_hint").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Web mode:");
                    ui.label("Use");
                    ui.monospace("File > Open .ld file...");
                    ui.label("to choose an .ld file, then optionally a matching .ldx sidecar.");
                });
            });
        }

        // Session info bar
        if self.shared.ld_file.is_some() {
            egui::Panel::top("session_info").show(ui, |ui| {
                self.show_session_info(ui);
            });

            // Timeline overview strip
            egui::Panel::top("timeline").show(ui, |ui| {
                self.timeline.ui(ui, &mut self.shared);
            });
        }

        // Channel browser — collapsible left panel
        if self.show_channel_browser {
            egui::Panel::left("channel_browser")
                .default_size(280.0)
                .resizable(true)
                .show(ui, |ui| {
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
                .show(ui, |ui| {
                    if Self::collapsed_panel_strip(ui, "Channels") {
                        self.show_channel_browser = true;
                    }
                });
        }

        // Math editor — collapsible left panel (after browser)
        if self.show_math_editor {
            egui::Panel::left("math_editor")
                .default_size(460.0)
                .resizable(true)
                .show(ui, |ui| {
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
                .default_size(220.0)
                .min_size(72.0)
                .max_size(320.0)
                .resizable(true)
                .show(ui, |ui| {
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
                    crate::panels::cursor_readout::show(ui, &mut self.shared);
                });
        } else {
            egui::Panel::right("cursor_readout_collapsed")
                .exact_size(18.0)
                .resizable(false)
                .show(ui, |ui| {
                    if Self::collapsed_panel_strip(ui, "Readout") {
                        self.show_cursor_readout = true;
                    }
                });
        }

        if self.worksheets.len() > 1 {
            ui.vertical(|ui| {
                self.show_worksheet_tabs(ui);
                ui.separator();
            });
        }

        let show_inner_tab_bar = self.worksheets[self.active_worksheet]
            .dock_state
            .iter_all_tabs()
            .nth(1)
            .is_some();

        // Each worksheet is rendered once, using either the main surface or its own native viewport.
        if !self.worksheets[self.active_worksheet].window.detached {
            let dock = &mut self.worksheets[self.active_worksheet].dock_state;
            let mut viewer = AppTabViewer {
                shared: &mut self.shared,
            };
            let mut dock_style = egui_dock::Style::from_egui(ui.style().as_ref());
            if !show_inner_tab_bar {
                dock_style.tab_bar.height = 0.0;
            }
            DockArea::new(dock)
                .style(dock_style)
                .show_close_buttons(show_inner_tab_bar)
                .show_leaf_collapse_buttons(false)
                .draggable_tabs(true)
                .show_inside(ui, &mut viewer);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("This worksheet is open in a separate window");
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        for (index, worksheet) in self
            .worksheets
            .iter_mut()
            .enumerate()
            .filter(|(_, w)| w.window.detached)
        {
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("i3rs — {}", worksheet.name))
                .with_inner_size(worksheet.window.size);
            if let Some(position) = worksheet.window.position {
                builder = builder.with_position(position);
            }
            ui.ctx().show_viewport_immediate(
                egui::ViewportId::from_hash_of(("worksheet", index)),
                builder,
                |child, _| {
                    if child.input(|i| i.viewport().close_requested()) {
                        worksheet.window.detached = false;
                    }
                    if let Some(rect) = child.input(|i| i.viewport().inner_rect) {
                        worksheet.window.size = [rect.width(), rect.height()];
                    }
                    if let Some(rect) = child.input(|i| i.viewport().outer_rect) {
                        worksheet.window.position = Some([rect.min.x, rect.min.y]);
                    }
                    if child.button("Dock worksheet").clicked() {
                        worksheet.window.detached = false;
                    }
                    let mut viewer = AppTabViewer {
                        shared: &mut self.shared,
                    };
                    DockArea::new(&mut worksheet.dock_state)
                        .id(egui::Id::new(("detached_dock", index)))
                        .draggable_tabs(true)
                        .show_inside(child, &mut viewer);
                },
            );
        }

        if self.shared.dragging_channel.is_some() {
            if ui.input(|i| i.pointer.any_released()) {
                self.shared.dragging_channel = None;
            } else {
                self.draw_dragging_channel_badge(ui.ctx());
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        // Pop-out: move track map panels that requested pop-out from dock to separate windows
        for (worksheet_index, worksheet) in self.worksheets.iter_mut().enumerate() {
            let dock = &mut worksheet.dock_state;
            while let Some(path) =
                dock.find_tab_from(|t| matches!(t, PanelTab::TrackMap(tm) if tm.pop_out_requested))
            {
                if let Some(PanelTab::TrackMap(mut tm)) = dock.remove_tab(path) {
                    tm.pop_out_requested = false;
                    tm.is_popped_out = true;
                    tm.home_worksheet = worksheet_index;
                    self.popped_out_track_maps.push(tm);
                } else {
                    break;
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        // Render popped-out track maps in separate OS windows
        let shared = &mut self.shared;
        #[cfg(not(target_arch = "wasm32"))]
        let popped_out = &mut self.popped_out_track_maps;
        #[cfg(not(target_arch = "wasm32"))]
        for tm in popped_out.iter_mut() {
            let viewport_id = egui::ViewportId::from_hash_of(format!("track_map_{}", tm.id));
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("Track Map — {}", tm.title))
                .with_inner_size(tm.window.size)
                .with_min_inner_size([320., 240.]);
            if let Some(position) = tm.window.position {
                builder = builder.with_position(position);
            }
            ui.ctx()
                .show_viewport_immediate(viewport_id, builder, |viewport_ui, _class| {
                    if let Some(rect) = viewport_ui.input(|i| i.viewport().inner_rect) {
                        tm.window.size = [rect.width(), rect.height()];
                    }
                    if let Some(rect) = viewport_ui.input(|i| i.viewport().outer_rect) {
                        tm.window.position = Some([rect.min.x, rect.min.y]);
                    }
                    tm.ui(viewport_ui, shared);
                });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Dock-back: return panels that requested docking to the main dock
            let (to_dock, to_keep): (Vec<_>, Vec<_>) = self
                .popped_out_track_maps
                .drain(..)
                .partition(|tm| tm.dock_requested);
            self.popped_out_track_maps = to_keep;
            for mut tm in to_dock {
                tm.dock_requested = false;
                tm.is_popped_out = false;
                let worksheet_idx = tm
                    .home_worksheet
                    .min(self.worksheets.len().saturating_sub(1));
                self.worksheets[worksheet_idx]
                    .dock_state
                    .push_to_focused_leaf(PanelTab::TrackMap(tm));
            }
        }

        crate::panels::channel_status::show(ui.ctx(), &mut self.shared);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(index) = self.shared.comparisons.pending_swap.take() {
            if let Some(current) = self
                .shared
                .ld_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
            {
                let mut workspace = self.build_workspace_snapshot_with_popped_out_track_maps(true);
                if let Some(config) = workspace.comparisons.as_mut() {
                    match crate::comparison::swap_external_main(config, &current, index) {
                        Ok(path) => {
                            workspace.last_file_path = Some(path.clone());
                            self.open_file_async(path.into(), ui.ctx());
                            if let Some(pending) = self.pending_session_load.as_mut() {
                                pending.workspace_snapshot = Some(workspace);
                            }
                        }
                        Err(error) => self.shared.message(error),
                    }
                }
            } else {
                self.shared
                    .message("Cannot swap an uploaded Main recording without a reusable path");
            }
        }
        self.show_channel_preferences_window(ui.ctx());
        self.show_options_and_help(ui.ctx());
        self.show_session_details_window(ui.ctx());
        self.recording_editor.show(ui.ctx(), &mut self.shared);
        if let Some(path) = self.output_ui.show(ui.ctx(), &mut self.shared) {
            self.open_file(path);
        }
        self.submit_requested_channel_decodes(ui.ctx());
        self.submit_requested_math_channel_evaluations(ui.ctx());
        self.submit_requested_track_data_build(ui.ctx());

        // Clear per-frame flags
        self.shared.zoom_from_timeline = false;
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::histogram::HistogramPanel;
    use crate::state::{ChannelId, MathEvaluationState, PlottedChannel, YAxis};

    const TEST_LD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ld");

    fn worksheet_with_tabs(tabs: Vec<PanelTab>) -> Worksheet {
        Worksheet {
            window: Default::default(),
            name: "Test".into(),
            dock_state: DockState::new(tabs),
        }
    }

    fn test_app(shared: SharedState, worksheets: Vec<Worksheet>) -> App {
        #[cfg(not(target_arch = "wasm32"))]
        let (native_pick_tx, native_pick_rx) = crate::platform::native_pick_channel();
        App {
            egui_ctx: egui::Context::default(),
            shared,
            worksheets,
            active_worksheet: 0,
            show_channel_browser: true,
            show_cursor_readout: true,
            show_math_editor: false,
            timeline: TimelinePanel::new(),
            math_editor_state: MathEditorState::new(),
            popped_out_track_maps: Vec::new(),
            project_path: None,
            project_name: None,
            project_sessions: Vec::new(),
            theme_choice: ThemeChoice::System,
            options: Default::default(),
            show_options: false,
            show_help: false,
            show_channel_preferences: false,
            show_session_details: false,
            recording_editor: Default::default(),
            output_ui: Default::default(),
            compare_session_path: None,
            session_summary_cache: HashMap::new(),
            load_error: None,
            background_jobs: BackgroundJobs::new(),
            pending_session_load: None,
            pending_workspace_restore: None,
            next_session_id: 1,
            next_request_id: 1,
            #[cfg(not(target_arch = "wasm32"))]
            native_pick_tx,
            #[cfg(not(target_arch = "wasm32"))]
            native_pick_rx,
            #[cfg(target_arch = "wasm32")]
            web_load_tx: crate::platform::web_load_channel().0,
            #[cfg(target_arch = "wasm32")]
            web_load_rx: crate::platform::web_load_channel().1,
        }
    }

    #[test]
    fn project_analysis_restore_waits_for_the_requested_recording() {
        let mut shared = SharedState::new();
        let ld = LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        shared.data_duration = Some(ld.duration_secs());
        let laps = detect_laps(&ld);
        shared.ld_file = Some(Arc::new(ld));
        shared.activate_recording_analysis(&laps);
        let mut workspace = crate::workspace::save_workspace(&[], 0, &shared);
        workspace
            .recording_analyses
            .get_mut(shared.recording_key())
            .unwrap()
            .corrections
            .gps_enabled = false;
        let mut app = test_app(shared, vec![]);
        app.pending_session_load = Some(PendingSessionLoad {
            request_id: 42,
            file_label: "Requested recording".into(),
            workspace_snapshot: None,
        });
        app.restore_workspace_when_ready(workspace);
        assert!(
            app.shared.current_corrections().gps_enabled,
            "do not apply the incoming project's settings to the old recording"
        );
        let saved = app
            .pending_session_load
            .as_ref()
            .unwrap()
            .workspace_snapshot
            .as_ref()
            .unwrap();
        assert!(
            !saved.recording_analyses[app.shared.recording_key()]
                .corrections
                .gps_enabled
        );
    }

    #[test]
    fn startup_default_layout_is_detected() {
        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(GraphPanel::new(1, "Graph"))]);
        assert!(is_startup_default_layout(&[worksheet]));
    }

    #[test]
    fn bundled_vir_sample_waits_for_corrections_then_prepares_complete_track_annotations() {
        let mut app = test_app(
            SharedState::new(),
            vec![worksheet_with_tabs(vec![PanelTab::Graph(GraphPanel::new(
                1, "Graph",
            ))])],
        );
        app.open_bytes("VIR_LAP.ld".into(), std::fs::read(TEST_LD).unwrap(), None)
            .unwrap();
        assert!(app.shared.selected_lap.is_none());
        assert!(app.shared.sectors.is_empty());
        assert!(!app.configure_vir_sample().unwrap());
        assert!(app.shared.take_requested_track_data_build());

        let corrected = i3rs_core::corrections::build_corrected_recording(
            app.shared.ld_file.as_ref().unwrap(),
            &app.shared.current_corrections(),
            &app.shared.competition_laps(),
        );
        let key = app.shared.correction_key();
        assert!(app.shared.apply_correction_result(&key, corrected));
        assert!(app.configure_vir_sample().unwrap());
        let lap_index = app.shared.selected_lap.unwrap();
        assert_eq!(
            app.shared.current_analysis().unwrap().ranges[lap_index].kind,
            i3rs_core::ranges::RangeKind::Lap
        );
        assert_eq!(app.shared.sectors.len(), 33);
        let track = app.shared.track_data_if_ready().unwrap();
        let ld = app.shared.ld_file.as_ref().unwrap();
        let gear = ld.find_channel_by_name("Gear").unwrap();
        let gear_data = ld.read_channel_data(gear).unwrap();
        let events = i3rs_core::annotations::track_annotations(
            &track,
            &app.shared.laps[lap_index],
            &app.shared.sectors,
            Some((&gear_data, gear.freq)),
            &Default::default(),
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| !event.section.is_empty())
                .count(),
            33
        );
        assert!(events.iter().any(|event| matches!(
            event.kind,
            i3rs_core::annotations::AnnotationKind::GearChange { .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event.kind,
            i3rs_core::annotations::AnnotationKind::MinimumSpeed(_)
        )));
        assert!(events.iter().any(|event| matches!(
            event.kind,
            i3rs_core::annotations::AnnotationKind::MaximumSpeed(_)
        )));
    }

    #[test]
    fn custom_single_sheet_layout_is_not_treated_as_default() {
        let worksheet = worksheet_with_tabs(vec![
            PanelTab::Graph(GraphPanel::new(1, "Graph")),
            PanelTab::Histogram(HistogramPanel::new(2, "Histogram")),
        ]);
        assert!(!is_startup_default_layout(&[worksheet]));
    }

    #[test]
    fn graph_with_channels_is_not_treated_as_default() {
        let mut graph = GraphPanel::new(1, "Graph");
        graph.plotted_channels.push(PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![1.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: crate::state::ScaleMode::Auto,
            manual_min: 1.0,
            manual_max: 1.0,
            cached_min: 1.0,
            cached_max: 1.0,
            cached_avg: 1.0,
        });

        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(graph)]);
        assert!(!is_startup_default_layout(&[worksheet]));
    }

    #[test]
    fn switching_profiles_keeps_the_recording_selection_and_analysis_edits() {
        let mut app = test_app(SharedState::new(), vec![]);
        app.open_file(PathBuf::from(TEST_LD));
        let ld = app.shared.ld_file.clone().unwrap();
        let key = app.shared.recording_key().to_string();
        let mut analysis = app.shared.current_analysis().unwrap().clone();
        analysis.details.notes = "Retain this recording across profiles".into();
        app.shared.apply_recording_analysis(analysis).unwrap();
        app.shared.select_lap(Some(1));
        app.shared.cursor_time = Some(12.5);
        let range = app.shared.zoom_range;
        for profile in crate::default_layouts::WorkspaceProfile::ALL {
            app.use_workspace_profile(profile);
            assert!(Arc::ptr_eq(app.shared.ld_file.as_ref().unwrap(), &ld));
            assert_eq!(app.shared.recording_key(), key);
            assert_eq!(app.shared.selected_lap, Some(1));
            assert_eq!(app.shared.cursor_time, Some(12.5));
            assert_eq!(app.shared.zoom_range, range);
            assert_eq!(
                app.shared.current_analysis().unwrap().details.notes,
                "Retain this recording across profiles"
            );
        }
    }

    #[test]
    fn opening_new_file_reapplies_current_workspace_channels() {
        let path = PathBuf::from(TEST_LD);
        let ld = Arc::new(LdFile::open(&path).expect("failed to open test .ld"));

        let mut graph = GraphPanel::new(1, "Graph");
        graph.plotted_channels.push(PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![1.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: crate::state::ScaleMode::Auto,
            manual_min: 1.0,
            manual_max: 1.0,
            cached_min: 1.0,
            cached_max: 1.0,
            cached_avg: 1.0,
        });

        let mut shared = SharedState::new();
        shared.file_name = "VIR_LAP.ld".into();
        shared.laps = detect_laps(&ld);
        shared.data_duration = Some(ld.duration_secs());
        shared.ld_file = Some(ld);
        shared.ld_path = Some(path.clone());

        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(graph)]);
        let mut app = test_app(shared, vec![worksheet]);

        app.open_file(path);

        let reloaded_graph = app.worksheets[0]
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::Graph(graph) => Some(graph),
                _ => None,
            })
            .expect("graph tab should exist");

        assert_eq!(reloaded_graph.plotted_channels.len(), 1);
    }

    #[test]
    fn opening_new_file_resets_time_window_to_full_session() {
        let path = PathBuf::from(TEST_LD);
        let ld = Arc::new(LdFile::open(&path).expect("failed to open test .ld"));
        let duration = ld.duration_secs();

        let mut shared = SharedState::new();
        shared.file_name = "VIR_LAP.ld".into();
        shared.laps = detect_laps(&ld);
        shared.data_duration = Some(duration);
        shared.ld_file = Some(ld);
        shared.ld_path = Some(path.clone());
        shared.cursor_time = Some(12.34);
        shared.zoom_range = Some((1.0, 2.0));

        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(GraphPanel::new(1, "Graph"))]);
        let mut app = test_app(shared, vec![worksheet]);

        app.open_file(path);

        assert_eq!(app.shared.cursor_time, None);
        assert_eq!(app.shared.zoom_range, Some((0.0, duration)));
    }

    #[test]
    fn session_reload_snapshot_excludes_popped_out_track_maps() {
        let shared = SharedState::new();
        let worksheet = worksheet_with_tabs(vec![
            PanelTab::Graph(GraphPanel::new(1, "Graph")),
            PanelTab::Histogram(HistogramPanel::new(3, "Histogram")),
        ]);
        let mut app = test_app(shared, vec![worksheet]);
        app.popped_out_track_maps
            .push(TrackMapPanel::new(2, "Track Map 2"));

        let workspace = app
            .workspace_snapshot_for_session_reload()
            .expect("custom layout should snapshot");
        app.apply_workspace_snapshot(workspace);

        let docked_track_maps = app.worksheets[0]
            .dock_state
            .iter_all_tabs()
            .filter(|(_, tab)| matches!(tab, PanelTab::TrackMap(_)))
            .count();

        assert_eq!(docked_track_maps, 0);
        assert_eq!(app.popped_out_track_maps.len(), 1);
    }

    #[test]
    fn workspace_restore_waits_for_math_channels_to_finish() {
        let mut shared = SharedState::new();
        shared.ld_path = Some("previous.ld".into());
        shared.sectors = vec![i3rs_core::Sector {
            semantics: i3rs_core::track::SectionSemantics::default(),
            name: "Previous recording".into(),
            start_index: 50,
            end_index: 100,
            distance: None,
        }];
        let mut math_channel =
            shared.create_math_channel_def("Derived".into(), "1".into(), String::new(), 2);
        math_channel.data = Some(Arc::from(vec![1.0]));
        math_channel.freq = 1.0;
        math_channel.evaluation_state = MathEvaluationState::Ready;
        shared.math_channels.push(math_channel);

        let mut graph = GraphPanel::new(1, "Graph");
        graph.plotted_channels.push(PlottedChannel {
            channel_id: ChannelId::Math(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![1.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: crate::state::ScaleMode::Auto,
            manual_min: 1.0,
            manual_max: 1.0,
            cached_min: 1.0,
            cached_max: 1.0,
            cached_avg: 1.0,
        });

        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(graph)]);
        let mut app = test_app(shared, vec![worksheet]);
        let workspace = app
            .workspace_snapshot_for_session_reload()
            .expect("custom layout should snapshot");

        app.worksheets = vec![worksheet_with_tabs(vec![PanelTab::Graph(GraphPanel::new(
            2, "Empty",
        ))])];
        app.shared.math_channels[0].data = None;
        app.shared.math_channels[0].error = Some("Waiting for source channels...".into());
        app.shared.math_channels[0].evaluation_state = MathEvaluationState::WaitingForInputs;

        app.shared.ld_path = Some("current.ld".into());
        app.restore_workspace_when_ready(workspace);

        assert!(
            app.shared.sectors.is_empty(),
            "Old recording indices must not remain active while math loads"
        );
        assert!(app.pending_workspace_restore.is_some());
        let pending_graph = app.worksheets[0]
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::Graph(graph) => Some(graph),
                _ => None,
            })
            .expect("graph tab should exist");
        assert!(pending_graph.plotted_channels.is_empty());

        app.shared.math_channels[0].data = Some(Arc::from(vec![2.0]));
        app.shared.math_channels[0].freq = 1.0;
        app.shared.math_channels[0].error = None;
        app.shared.math_channels[0].evaluation_state = MathEvaluationState::Ready;

        app.maybe_apply_pending_workspace_restore();

        assert!(app.pending_workspace_restore.is_none());
        let restored_graph = app.worksheets[0]
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::Graph(graph) => Some(graph),
                _ => None,
            })
            .expect("graph tab should exist");
        assert_eq!(restored_graph.plotted_channels.len(), 1);
        assert!(matches!(
            restored_graph.plotted_channels[0].channel_id,
            ChannelId::Math(0)
        ));
    }
    #[test]
    fn recording_scoped_math_errors_complete_layout_restore_without_background_results() {
        let mut app = test_app(
            SharedState::new(),
            vec![worksheet_with_tabs(vec![
                PanelTab::Graph(GraphPanel::new(1, "Graph")),
                PanelTab::Histogram(HistogramPanel::new(2, "Histogram")),
            ])],
        );
        app.open_file(TEST_LD.into());
        let index = app
            .shared
            .ld_file
            .as_ref()
            .unwrap()
            .find_channel_by_name("Engine Speed")
            .unwrap()
            .index;
        let channel = crate::panels::utils::create_plotted_channel(
            ChannelId::Physical(index),
            &app.shared,
            0,
        )
        .unwrap();
        if let PanelTab::Graph(graph) = app.worksheets[0]
            .dock_state
            .iter_all_tabs_mut()
            .next()
            .unwrap()
            .1
        {
            graph.plotted_channels.push(channel);
        }
        let mut math = app.shared.create_math_channel_def(
            "Scoped coefficient".into(),
            "1".into(),
            "".into(),
            2,
        );
        math.options.recording = Some("another-recording.ld".into());
        app.shared.math_channels.push(math);
        app.open_file(TEST_LD.into());
        assert!(app.pending_workspace_restore.is_some());
        app.submit_requested_math_channel_evaluations(&egui::Context::default());
        assert!(app.pending_workspace_restore.is_none());
        assert_eq!(
            app.shared.math_channels[0].evaluation_state,
            MathEvaluationState::Error
        );
        let graph = app.worksheets[0]
            .dock_state
            .iter_all_tabs()
            .find_map(|(_, tab)| match tab {
                PanelTab::Graph(graph) => Some(graph),
                _ => None,
            })
            .unwrap();
        assert_eq!(graph.plotted_channels.len(), 1);
        assert_eq!(
            crate::panels::utils::resolve_channel_meta(
                graph.plotted_channels[0].channel_id,
                &app.shared
            )
            .0,
            "Engine Speed"
        );
    }
    #[test]
    #[ignore = "requires local May 8 reference recording"]
    fn external_main_swap_installs_recording_and_rebinds_component_slots() {
        let mut app = test_app(
            SharedState::new(),
            vec![worksheet_with_tabs(vec![
                PanelTab::Graph(GraphPanel::new(1, "Graph")),
                PanelTab::Histogram(HistogramPanel::new(2, "Histogram")),
            ])],
        );
        app.open_file(TEST_LD.into());
        app.shared.comparisons.main_lap = Some(0);
        let main = app
            .shared
            .ld_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let other = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        );
        let mut graph = GraphPanel::new(99, "loader");
        let source = graph.load_external_overlay_path(other.into()).unwrap();
        app.shared.comparisons.sessions = graph.overlay_sessions;
        app.shared
            .comparisons
            .overlays
            .push(crate::panels::graph::LapOverlay {
                alignment_valid: false,
                gps_align: false,
                visible: true,
                color: None,
                source: crate::panels::graph::OverlaySource::External(source),
                lap_idx: 3,
                manual_offset: 0.,
                stretch_to_reference: false,
            });
        let mut workspace = app.build_workspace_snapshot();
        let path = crate::comparison::swap_external_main(
            workspace.comparisons.as_mut().unwrap(),
            &main,
            0,
        )
        .unwrap();
        let loaded = load_session_from_path(path.into()).unwrap();
        app.install_loaded_session(loaded, Some(workspace));
        assert_eq!(app.shared.comparisons.main_lap, Some(3));
        assert_eq!(app.shared.selected_lap, Some(3));
        assert_eq!(app.shared.comparisons.sessions.len(), 1);
        assert_eq!(app.shared.comparisons.sessions[0].path, PathBuf::from(main));
        assert_eq!(app.shared.comparisons.overlays[0].lap_idx, 0);
    }
    #[test]
    fn native_track_window_restore_preserves_geometry_without_duplicate_ownership() {
        let mut app = test_app(
            SharedState::new(),
            vec![worksheet_with_tabs(vec![
                PanelTab::Graph(GraphPanel::new(1, "G")),
                PanelTab::Histogram(HistogramPanel::new(2, "H")),
            ])],
        );
        let mut map = TrackMapPanel::new(3, "Map");
        map.is_popped_out = true;
        map.window.position = Some([120., 140.]);
        map.window.size = [700., 500.];
        app.popped_out_track_maps.push(map);
        let workspace = app.build_workspace_snapshot();
        let json = serde_json::to_string(&workspace).unwrap();
        app.apply_workspace_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(app.popped_out_track_maps.len(), 1);
        assert_eq!(
            app.popped_out_track_maps[0].window.position,
            Some([120., 140.])
        );
        assert_eq!(app.popped_out_track_maps[0].window.size, [700., 500.]);
        assert_eq!(app.worksheets[0].dock_state.iter_all_tabs().count(), 2);
    }
}

//! Main application: coordinates panels and shared state.

use eframe::egui;
use egui_dock::{DockArea, DockState};
use i3rs_core::{ExportChannel, LdFile, LdxFile, detect_laps, export_csv};
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
    show_channel_preferences: bool,
    show_session_details: bool,
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
        let preferences = crate::preferences::load_preferences();
        let mut shared = SharedState::new();
        shared.channel_preferences = preferences.channel_preferences.clone();
        let dock_state = Self::default_dock_state(&mut shared);
        let worksheets = vec![Worksheet {
            name: "Sheet 1".into(),
            dock_state,
        }];
        apply_theme_to_ctx(&cc.egui_ctx, preferences.theme);
        Self {
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
            show_channel_preferences: false,
            show_session_details: false,
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

        if include_popped_out_track_maps {
            // Preserve popped-out track maps in the worksheet they came from.
            for tm in &self.popped_out_track_maps {
                let color_channel_name = tm.color_channel_idx.and_then(|idx| {
                    self.shared
                        .ld_file
                        .as_ref()
                        .and_then(|ld| ld.channels.get(idx).map(|ch| ch.name.clone()))
                });
                let worksheet_idx = tm
                    .home_worksheet
                    .min(workspace.worksheets.len().saturating_sub(1));
                if let Some(ws) = workspace.worksheets.get_mut(worksheet_idx) {
                    ws.panels.push(crate::workspace::PanelConfig::TrackMap(
                        crate::workspace::TrackMapPanelConfig {
                            id: tm.id,
                            title: tm.title.clone(),
                            color_channel_name,
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
            .map(|(name, dock_state)| Worksheet { name, dock_state })
            .collect();
        self.active_worksheet = active_worksheet.min(self.worksheets.len().saturating_sub(1));
    }

    fn restore_workspace_when_ready(&mut self, workspace: crate::workspace::WorkspaceFile) {
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
        self.shared.zoom_range = Some((0.0, loaded.data_duration));
        self.shared.invalidate_session_caches();

        // Clear all panels' channels and caches across all worksheets
        for ws in &mut self.worksheets {
            for (_path, tab) in ws.dock_state.iter_all_tabs_mut() {
                match tab {
                    PanelTab::Graph(g) => g.reset_for_new_main_session(),
                    PanelTab::TrackMap(t) => t.clear_cache(),
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

        // Re-evaluate math channels with new file data
        math_editor::evaluate_all_math_channels(&mut self.shared);

        // Auto-populate default worksheets if current layout is empty
        let is_empty_default = is_startup_default_layout(&self.worksheets);

        if is_empty_default {
            let ld_ref = self.shared.ld_file.clone().unwrap();
            let defaults = {
                let _perf = crate::perf_metrics::scope("default layout creation");
                crate::default_layouts::create_default_worksheets(&ld_ref, &mut self.shared)
            };
            if !defaults.is_empty() {
                self.worksheets = defaults
                    .into_iter()
                    .map(|(name, dock_state)| Worksheet { name, dock_state })
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
                                decoded.freq,
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
                            self.load_error = Some(format!("Failed to decode channel: {err}"));
                        }
                    }
                }
                JobResult::BuildTrackData {
                    session_id,
                    track_data,
                } => {
                    if self.shared.session_id != session_id {
                        continue;
                    }
                    self.shared.store_track_data(track_data);
                }
                JobResult::EvaluateMathChannel {
                    session_id,
                    math_id,
                    expression,
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
                        result,
                    ) {
                        self.shared.invalidate_derived_caches();
                        if !self.shared.math_channels.is_empty() {
                            math_editor::reevaluate_math_channels_waiting_on_inputs(
                                &mut self.shared,
                            );
                        }
                    }
                    self.maybe_apply_pending_workspace_restore();
                }
                JobResult::BuildDownsampledSeries {
                    session_id,
                    key,
                    points,
                } => {
                    if self.shared.session_id != session_id {
                        continue;
                    }
                    self.shared.store_downsampled_series(key, points);
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

        for math_id in requested {
            let Some(job) = math_editor::build_math_evaluation_job(&mut self.shared, math_id)
            else {
                self.shared.cancel_math_channel_evaluation(math_id);
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
                    aliases: job.aliases,
                    channel_data: job.channel_data,
                },
                ctx,
            ) {
                self.shared.cancel_math_channel_evaluation(math_id);
                self.load_error = Some(err);
            }
        }
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
                ld,
            },
            ctx,
        ) {
            self.shared.cancel_track_data_build();
            self.load_error = Some(err);
        }
    }

    fn submit_requested_downsampled_series(&mut self, ctx: &egui::Context) {
        for request in self.shared.take_requested_downsampled_series() {
            let request_id = self.next_request_id;
            self.next_request_id += 1;

            if let Err(err) = self.background_jobs.submit(
                JobRequest::BuildDownsampledSeries {
                    request_id,
                    session_id: self.shared.session_id,
                    key: request.key.clone(),
                    data: request.data,
                    freq: request.freq,
                    start_sample: request.start_sample,
                    end_sample: request.end_sample,
                    target_width: request.target_width,
                },
                ctx,
            ) {
                self.shared.cancel_downsampled_series(&request.key);
                self.load_error = Some(err);
            }
        }
    }

    fn clear_loaded_session(&mut self) {
        self.pending_workspace_restore = None;
        self.shared.ld_file = None;
        self.shared.ld_path = None;
        self.shared.file_name.clear();
        self.shared.laps.clear();
        self.shared.ldx = None;
        self.shared.selected_lap = None;
        self.shared.cursor_time = None;
        self.shared.zoom_range = None;
        self.shared.data_duration = None;
        self.shared.invalidate_session_caches();

        for ws in &mut self.worksheets {
            for (_path, tab) in ws.dock_state.iter_all_tabs_mut() {
                match tab {
                    PanelTab::Graph(g) => g.reset_for_new_main_session(),
                    PanelTab::TrackMap(t) => t.clear_cache(),
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
            channel_preferences: self.shared.channel_preferences.clone(),
        };

        #[cfg(target_arch = "wasm32")]
        {
            if let Err(e) = crate::preferences::save_preferences(&preferences) {
                eprintln!("Failed to save preferences: {}", e);
            }
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            if let Err(e) = crate::preferences::save_preferences(&preferences) {
                eprintln!("Failed to save preferences: {}", e);
            }
        });
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
                Err(e) => eprintln!("Failed to save project: {}", e),
            },
            Err(e) => eprintln!("Failed to serialize project: {}", e),
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
                            eprintln!(
                                "Project session file not found: {}",
                                active_session.display()
                            );
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

                    // Load math channels from project workspace.
                    self.shared.math_channels.clear();
                    for config in &project.workspace.math_channels {
                        let math_channel = self.shared.create_math_channel_def(
                            config.name.clone(),
                            config.expression.clone(),
                            config.unit.clone(),
                            config.dec_places,
                        );
                        self.shared.math_channels.push(math_channel);
                    }
                    math_editor::evaluate_all_math_channels(&mut self.shared);
                    self.shared.invalidate_derived_caches();
                    self.shared.sectors = project.workspace.sectors.clone();
                    self.shared.reference_lap = project.workspace.reference_lap;

                    let workspace = project.workspace;
                    self.sync_project_sessions_from_workspace(&workspace);
                    self.pending_workspace_restore = None;
                    self.restore_workspace_when_ready(workspace);
                }
                Err(e) => eprintln!("Failed to parse project: {}", e),
            },
            Err(e) => eprintln!("Failed to read project file: {}", e),
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
        use egui::{Key, KeyboardShortcut, Modifiers};

        let command = Modifiers::COMMAND;
        let command_shift = Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        };

        let open_file = KeyboardShortcut::new(command, Key::O);
        let save_project = KeyboardShortcut::new(command, Key::S);
        let load_project = KeyboardShortcut::new(command_shift, Key::O);
        let save_workspace = KeyboardShortcut::new(command_shift, Key::S);
        let toggle_browser = KeyboardShortcut::new(command, Key::B);
        let toggle_readout = KeyboardShortcut::new(command, Key::R);
        let toggle_math = KeyboardShortcut::new(command, Key::M);
        let show_details = KeyboardShortcut::new(command, Key::I);
        let show_prefs = KeyboardShortcut::new(command, Key::Comma);
        let add_graph = KeyboardShortcut::new(command, Key::G);
        let add_track_map = KeyboardShortcut::new(command_shift, Key::T);
        let add_histogram = KeyboardShortcut::new(command_shift, Key::H);
        let add_scatter = KeyboardShortcut::new(command_shift, Key::X);
        let add_fft = KeyboardShortcut::new(command_shift, Key::F);

        if ctx.input_mut(|i| i.consume_shortcut(&open_file)) {
            self.start_open_session(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save_project)) {
            self.save_project();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&load_project)) {
            self.load_project();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save_workspace)) {
            self.save_workspace();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&toggle_browser)) {
            self.show_channel_browser = !self.show_channel_browser;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&toggle_readout)) {
            self.show_cursor_readout = !self.show_cursor_readout;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&toggle_math)) {
            self.show_math_editor = !self.show_math_editor;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&show_details)) {
            self.show_session_details = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&show_prefs)) {
            self.show_channel_preferences = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&add_graph)) {
            self.add_graph_panel();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&add_track_map)) {
            self.add_track_map_panel();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&add_histogram)) {
            self.add_histogram_panel();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&add_scatter)) {
            self.add_scatter_panel();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&add_fft)) {
            self.add_fft_panel();
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

    fn add_track_map_panel(&mut self) {
        let id = self.shared.next_panel_id;
        self.shared.next_panel_id += 1;
        let mut track_map = TrackMapPanel::new(id, format!("Track Map {}", id));
        track_map.home_worksheet = self.active_worksheet;
        track_map.pop_out_requested = true;
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
            .push_to_focused_leaf(PanelTab::Scatter(scatter));
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
            .push_to_focused_leaf(PanelTab::MixtureMap(mixture_map));
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
        let workspace = self.build_workspace_snapshot();

        if let Ok(json) = serde_json::to_string_pretty(&workspace)
            && let Some(path) = crate::platform::save_workspace_file()
            && let Err(e) = std::fs::write(&path, json)
        {
            eprintln!("Failed to save workspace: {}", e);
        }
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

                        // Load math channels from workspace
                        self.shared.math_channels.clear();
                        for config in &workspace.math_channels {
                            let math_channel = self.shared.create_math_channel_def(
                                config.name.clone(),
                                config.expression.clone(),
                                config.unit.clone(),
                                config.dec_places,
                            );
                            self.shared.math_channels.push(math_channel);
                        }
                        math_editor::evaluate_all_math_channels(&mut self.shared);
                        self.shared.invalidate_derived_caches();

                        self.shared.sectors = workspace.sectors.clone();
                        self.shared.reference_lap = workspace.reference_lap;

                        self.sync_project_sessions_from_workspace(&workspace);
                        self.pending_workspace_restore = None;
                        self.restore_workspace_when_ready(workspace);
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

        if let Some(path) = crate::platform::save_csv_file() {
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

    fn show_menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open .ld file...").clicked() {
                    self.start_open_session(ctx);
                    ui.close();
                }
                ui.separator();
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
                if ui.button("Session Details").clicked() {
                    self.show_session_details = true;
                    ui.close();
                }
                if ui.button("Global Channel Preferences").clicked() {
                    self.show_channel_preferences = true;
                    ui.close();
                }
                ui.separator();
                ui.label("Theme");
                self.show_theme_menu(ui, ctx);
            });
        });
    }

    fn show_session_info(&mut self, ui: &mut egui::Ui) {
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
                ui.label(&s.venue);
                ui.separator();
                ui.label(&s.vehicle_id);
                ui.separator();
                ui.label(i3rs_core::format_duration(ld.duration_secs()));
                ui.separator();
                ui.label(format!("{} channels", ld.channels.len()));
                if !self.shared.laps.is_empty() {
                    ui.separator();
                    ui.label(format!("{} laps", self.shared.laps.len()));
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
            egui::Stroke::new(1.0, egui::Color32::from_gray(110)),
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
        crate::perf_metrics::maybe_log_summary();
        self.process_platform_events(ui.ctx());
        self.submit_requested_channel_decodes(ui.ctx());
        self.submit_requested_math_channel_evaluations(ui.ctx());
        self.submit_requested_track_data_build(ui.ctx());
        self.submit_requested_downsampled_series(ui.ctx());
        self.handle_shortcuts(ui.ctx());

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
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            self.show_menu_bar(ui, &ctx);
        });

        let mut dismiss_load_error = false;
        if let Some(err) = &self.load_error {
            egui::Panel::top("load_error").show_inside(ui, |ui| {
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
            egui::Panel::top("load_status").show_inside(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading {}…", pending.file_label));
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        if self.shared.ld_file.is_none() {
            egui::Panel::top("web_open_hint").show_inside(ui, |ui| {
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
            egui::Panel::top("session_info").show_inside(ui, |ui| {
                self.show_session_info(ui);
            });

            // Timeline overview strip
            egui::Panel::top("timeline").show_inside(ui, |ui| {
                self.timeline.ui(ui, &mut self.shared);
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
                .default_size(220.0)
                .min_size(72.0)
                .max_size(320.0)
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

        // Dock area fills the rest (graph + report panels)
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

        if self.shared.dragging_channel.is_some() {
            if ui.input(|i| i.pointer.any_released()) {
                self.shared.dragging_channel = None;
            } else {
                self.draw_dragging_channel_badge(ui.ctx());
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        // Pop-out: move track map panels that requested pop-out from dock to separate windows
        let dock = &mut self.worksheets[self.active_worksheet].dock_state;
        #[cfg(not(target_arch = "wasm32"))]
        while let Some(path) =
            dock.find_tab_from(|t| matches!(t, PanelTab::TrackMap(tm) if tm.pop_out_requested))
        {
            if let Some(PanelTab::TrackMap(mut tm)) = dock.remove_tab(path) {
                tm.pop_out_requested = false;
                tm.is_popped_out = true;
                tm.home_worksheet = self.active_worksheet;
                self.popped_out_track_maps.push(tm);
            } else {
                break;
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
            ui.ctx().show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default()
                    .with_title(format!("Track Map — {}", tm.title))
                    .with_inner_size([800.0, 600.0]),
                |viewport_ui, _class| {
                    tm.ui(viewport_ui, shared);
                },
            );
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

        self.show_channel_preferences_window(ui.ctx());
        self.show_session_details_window(ui.ctx());
        self.submit_requested_channel_decodes(ui.ctx());
        self.submit_requested_math_channel_evaluations(ui.ctx());
        self.submit_requested_track_data_build(ui.ctx());
        self.submit_requested_downsampled_series(ui.ctx());

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
            name: "Test".into(),
            dock_state: DockState::new(tabs),
        }
    }

    fn test_app(shared: SharedState, worksheets: Vec<Worksheet>) -> App {
        #[cfg(not(target_arch = "wasm32"))]
        let (native_pick_tx, native_pick_rx) = crate::platform::native_pick_channel();
        App {
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
            show_channel_preferences: false,
            show_session_details: false,
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
    fn startup_default_layout_is_detected() {
        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(GraphPanel::new(1, "Graph"))]);
        assert!(is_startup_default_layout(&[worksheet]));
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
            cached_min: 1.0,
            cached_max: 1.0,
            cached_avg: 1.0,
        });

        let worksheet = worksheet_with_tabs(vec![PanelTab::Graph(graph)]);
        assert!(!is_startup_default_layout(&[worksheet]));
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
        let mut math_channel =
            shared.create_math_channel_def("Derived".into(), "1".into(), String::new(), 2);
        math_channel.data = Some(Arc::from(vec![1.0]));
        math_channel.freq = 1;
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

        app.restore_workspace_when_ready(workspace);

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
        app.shared.math_channels[0].freq = 1;
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
}

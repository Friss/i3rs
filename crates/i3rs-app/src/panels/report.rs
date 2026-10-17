//! Configurable range reports with one result population for table, chart and copy.
use eframe::egui;
use i3rs_core::report::{
    ReportBoundary, ReportSeries, ReportStatistic, ReportSummary, intersect_report_windows,
    report_statistics,
};
use serde::{Deserialize, Serialize};
#[path = "report_acceleration.rs"]
mod acceleration_ui;
#[path = "report_populations.rs"]
mod populations;

use super::utils::{
    build_plotted_channel_info, channel_picker, create_plotted_channel, refresh_plotted_channel,
    resolve_plotted_channel_display_meta, show_plotted_channel_display_menu,
};
use crate::state::{ChannelId, PlottedChannel, PlottedChannelInfo, SharedState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportScope {
    #[default]
    OutingAndRanges,
    Outing,
    Ranges,
    SelectedRange,
    AnalysisWindow,
    SelectedRanges,
    SelectedOutings,
    AllLoaded,
}
impl ReportScope {
    const ALL: [Self; 8] = [
        Self::OutingAndRanges,
        Self::Outing,
        Self::Ranges,
        Self::SelectedRange,
        Self::AnalysisWindow,
        Self::SelectedRanges,
        Self::SelectedOutings,
        Self::AllLoaded,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::OutingAndRanges => "Current outing and ranges",
            Self::Outing => "Current outing",
            Self::Ranges => "All ranges in current outing",
            Self::SelectedRange => "Selected range",
            Self::AnalysisWindow => "Linked analysis window",
            Self::SelectedRanges => "Main and selected overlays",
            Self::SelectedOutings => "Selected outings",
            Self::AllLoaded => "All loaded outings",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportView {
    #[default]
    Table,
    Chart,
    Split,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportKind {
    #[default]
    Channels,
    Time,
    Acceleration,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportReference {
    #[default]
    Absolute,
    Main,
    Overlay0,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccelerationMetric {
    #[default]
    Time,
    Distance,
    AverageG,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportSettings {
    pub range_kind: Option<i3rs_core::ranges::RangeKind>,
    pub drag_milestones: bool,
    pub drag_cumulative: bool,
    pub kind: ReportKind,
    pub reference: ReportReference,
    pub sections_as_headings: bool,
    pub split_horizontal: bool,
    /// Overrides indexed by report-owned channel slot. None inherits common statistics.
    pub channel_statistics: Vec<Option<Vec<ReportStatistic>>>,
    pub acceleration: i3rs_core::acceleration::AccelerationSettings,
    pub acceleration_metric: AccelerationMetric,
    pub scope: ReportScope,
    pub statistics: Vec<ReportStatistic>,
    pub boundary: ReportBoundary,
    pub trusted_only: bool,
    /// Stable section identity, retained when another recording lacks the section.
    pub section_id: Option<String>,
    pub view: ReportView,
    pub channels_in_columns: bool,
    pub reverse: bool,
    pub highlight_min: bool,
    pub highlight_max: bool,
    pub chart_statistic: ReportStatistic,
    /// Empty retains the legacy single-statistic chart.
    pub chart_statistics: Vec<ReportStatistic>,
}
impl Default for ReportSettings {
    fn default() -> Self {
        Self {
            range_kind: None,
            drag_milestones: false,
            drag_cumulative: false,
            kind: ReportKind::Channels,
            reference: ReportReference::Absolute,
            sections_as_headings: false,
            split_horizontal: false,
            channel_statistics: Vec::new(),
            acceleration: Default::default(),
            acceleration_metric: Default::default(),
            scope: ReportScope::default(),
            statistics: vec![
                ReportStatistic::Min,
                ReportStatistic::Max,
                ReportStatistic::Average,
                ReportStatistic::StdDev,
            ],
            boundary: ReportBoundary::Native,
            trusted_only: false,
            section_id: None,
            view: ReportView::Table,
            channels_in_columns: false,
            reverse: false,
            highlight_min: false,
            highlight_max: false,
            chart_statistic: ReportStatistic::Average,
            chart_statistics: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ReportGroup {
    id: String,
    source: Option<usize>,
    section: Option<String>,
    label: String,
    lap_index: Option<usize>,
    windows: Vec<(f64, f64)>,
}
#[derive(Debug, PartialEq)]
struct ChannelKey {
    name: String,
    unit: String,
    data_ptr: usize,
    len: usize,
    freq: f64,
    scale: u64,
    offset: u64,
    interpolation: i3rs_core::Interpolation,
}
impl From<&PlottedChannelInfo> for ChannelKey {
    fn from(i: &PlottedChannelInfo) -> Self {
        Self {
            name: i.name.clone(),
            unit: i.unit.clone(),
            data_ptr: i.data.as_ptr() as usize,
            len: i.data.len(),
            freq: i.freq,
            scale: i.display_scale.to_bits(),
            offset: i.display_offset.to_bits(),
            interpolation: i.interpolation,
        }
    }
}

pub struct ReportPanel {
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    pub settings: ReportSettings,
    filter: String,
    groups: Vec<ReportGroup>,
    channel_keys: Vec<ChannelKey>,
    boundary_key: ReportBoundary,
    // Keep the source allocations alive while their pointer identities are cached.
    sources: Vec<PlottedChannelInfo>,
    results: Vec<Vec<ReportSummary>>,
    grouping_key: String,
    population_contexts: Vec<populations::SourceContext>,
    population_groups: std::collections::HashMap<String, Vec<ReportGroup>>,
    population_diagnostics: Vec<String>,
    external_sources: Vec<Vec<PlottedChannelInfo>>,
    retained_external_sources: Vec<Vec<PlottedChannelInfo>>,
    reference_groups: [Vec<ReportGroup>; 2],
    reference_results: [Vec<ReportSummary>; 2],
    reference_key: String,
    acceleration_key: String,
    acceleration_result: i3rs_core::acceleration::AccelerationReport,
    selected_event: Option<usize>,
    selected_group: Option<String>,
    last_shared_selection: Option<usize>,
    diagnostic: Option<String>,
}
impl ReportPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            channels: Vec::new(),
            settings: Default::default(),
            filter: String::new(),
            groups: Vec::new(),
            channel_keys: Vec::new(),
            boundary_key: ReportBoundary::Native,
            sources: Vec::new(),
            results: Vec::new(),
            grouping_key: String::new(),
            population_contexts: Vec::new(),
            population_groups: Default::default(),
            population_diagnostics: Vec::new(),
            external_sources: Vec::new(),
            retained_external_sources: Vec::new(),
            reference_groups: Default::default(),
            reference_results: Default::default(),
            reference_key: String::new(),
            acceleration_key: String::new(),
            acceleration_result: Default::default(),
            selected_event: None,
            selected_group: None,
            last_shared_selection: None,
            diagnostic: None,
        }
    }
    pub fn add_channel(&mut self, id: ChannelId, shared: &SharedState) {
        if !self.channels.iter().any(|c| c.channel_id == id)
            && let Some(pc) = create_plotted_channel(id, shared, self.channels.len())
        {
            self.channels.push(pc);
            self.settings.channel_statistics.push(None);
        }
    }
    pub fn clear_channels(&mut self) {
        self.settings.channel_statistics.clear();
        self.channels.clear();
        self.sources.clear();
        self.external_sources.clear();
        self.retained_external_sources.clear();
        self.channel_keys.clear();
        self.results.clear();
    }
    pub fn copy_visible_channels(&mut self, shared: &SharedState) {
        for info in &shared.display_channel_registry {
            self.add_channel(info.channel_id, shared);
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui, shared: &SharedState) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("report_range_kind", self.id))
                .selected_text(
                    self.settings
                        .range_kind
                        .map_or("All range types", |k| k.label()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.range_kind, None, "All range types");
                    for kind in [
                        i3rs_core::ranges::RangeKind::Lap,
                        i3rs_core::ranges::RangeKind::Run,
                        i3rs_core::ranges::RangeKind::Stage,
                        i3rs_core::ranges::RangeKind::Transport,
                    ] {
                        ui.selectable_value(
                            &mut self.settings.range_kind,
                            Some(kind),
                            kind.label(),
                        );
                    }
                });
            ui.checkbox(
                &mut self.settings.drag_milestones,
                "Drag distance milestones",
            );
            if self.settings.drag_milestones {
                ui.checkbox(&mut self.settings.drag_cumulative, "Cumulative from launch");
            }
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("report_kind", self.id))
                .selected_text(format!("{:?} report", self.settings.kind))
                .show_ui(ui, |ui| {
                    for (kind, label) in [
                        (ReportKind::Channels, "Channel / Track / Section report"),
                        (ReportKind::Time, "Time report"),
                        (ReportKind::Acceleration, "Acceleration report"),
                    ] {
                        ui.selectable_value(&mut self.settings.kind, kind, label);
                    }
                });
            if self.settings.kind != ReportKind::Acceleration {
                egui::ComboBox::from_id_salt(("report_reference", self.id))
                    .selected_text(format!("{:?}", self.settings.reference))
                    .show_ui(ui, |ui| {
                        for (r, label) in [
                            (ReportReference::Absolute, "Absolute values"),
                            (ReportReference::Main, "Delta to Main"),
                            (ReportReference::Overlay0, "Delta to Overlay 0"),
                        ] {
                            ui.selectable_value(&mut self.settings.reference, r, label);
                        }
                    });
                ui.checkbox(&mut self.settings.sections_as_headings, "Section headings");
            }
        });
        if self.settings.kind == ReportKind::Acceleration {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("report_scope", self.id))
                .selected_text(self.settings.scope.label())
                .show_ui(ui, |ui| {
                    for scope in ReportScope::ALL {
                        ui.selectable_value(&mut self.settings.scope, scope, scope.label());
                    }
                });
            ui.checkbox(&mut self.settings.trusted_only, "Trusted ranges only");
            let mut interpolate = self.settings.boundary == ReportBoundary::Interpolate;
            if ui
                .checkbox(&mut interpolate, "Interpolate boundaries")
                .changed()
            {
                self.settings.boundary = if interpolate {
                    ReportBoundary::Interpolate
                } else {
                    ReportBoundary::Native
                };
            }
            let name = self
                .settings
                .section_id
                .as_ref()
                .map(|id| {
                    shared
                        .sectors
                        .iter()
                        .find(|s| &s.semantics.id == id)
                        .map_or("Unavailable section", |s| s.name.as_str())
                })
                .unwrap_or("Whole range");
            egui::ComboBox::from_id_salt(("report_section", self.id))
                .selected_text(name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.section_id, None, "Whole range");
                    for section in &shared.sectors {
                        ui.selectable_value(
                            &mut self.settings.section_id,
                            Some(section.semantics.id.clone()),
                            &section.name,
                        );
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            if self.settings.kind != ReportKind::Time {
                ui.menu_button("Statistics", |ui| {
                    for stat in ReportStatistic::ALL {
                        let mut enabled = self.settings.statistics.contains(&stat);
                        if ui.checkbox(&mut enabled, stat.label()).changed() {
                            self.settings.statistics.retain(|s| *s != stat);
                            if enabled {
                                self.settings.statistics.push(stat);
                            }
                        }
                    }
                });
            }
            for (view, label) in [
                (ReportView::Table, "Table"),
                (ReportView::Chart, "Chart"),
                (ReportView::Split, "Split"),
            ] {
                ui.selectable_value(&mut self.settings.view, view, label);
            }
            ui.menu_button("Display", |ui| {
                ui.checkbox(
                    &mut self.settings.channels_in_columns,
                    "Channels in columns",
                );
                ui.checkbox(&mut self.settings.reverse, "Reverse range order");
                ui.checkbox(&mut self.settings.split_horizontal, "Side-by-side split");
                ui.checkbox(&mut self.settings.highlight_min, "Highlight minimums");
                ui.checkbox(&mut self.settings.highlight_max, "Highlight maximums");
            });
            if self.settings.view != ReportView::Table {
                ui.menu_button("Chart statistics", |ui| {
                    for stat in self.visible_statistics() {
                        let mut enabled = self.settings.chart_statistics.contains(&stat);
                        if ui.checkbox(&mut enabled, stat.label()).changed() {
                            self.settings.chart_statistics.retain(|s| *s != stat);
                            if enabled {
                                self.settings.chart_statistics.push(stat);
                            }
                        }
                    }
                });
                egui::ComboBox::from_id_salt(("report_chart_stat", self.id))
                    .selected_text(if self.settings.chart_statistics.is_empty() {
                        self.stat_label(self.settings.chart_statistic)
                    } else {
                        "Multiple statistics"
                    })
                    .show_ui(ui, |ui| {
                        for stat in &self.visible_statistics() {
                            if ui
                                .selectable_value(
                                    &mut self.settings.chart_statistic,
                                    *stat,
                                    stat.label(),
                                )
                                .clicked()
                            {
                                self.settings.chart_statistics.clear();
                            }
                        }
                    });
            }
        });
        self.settings.statistics = ReportStatistic::ALL
            .into_iter()
            .filter(|s| self.settings.statistics.contains(s))
            .collect();
        if !self
            .visible_statistics()
            .contains(&self.settings.chart_statistic)
            && let Some(stat) = self.visible_statistics().first()
        {
            self.settings.chart_statistic = *stat;
        }
    }
    fn stat_label(&self, stat: ReportStatistic) -> &'static str {
        if self.settings.kind == ReportKind::Time {
            "Duration"
        } else {
            stat.label()
        }
    }
    fn visible_statistics(&self) -> Vec<ReportStatistic> {
        if self.settings.kind == ReportKind::Time {
            return vec![ReportStatistic::Average];
        }
        ReportStatistic::ALL
            .into_iter()
            .filter(|s| {
                self.settings.statistics.contains(s)
                    || self
                        .settings
                        .channel_statistics
                        .iter()
                        .flatten()
                        .any(|v| v.contains(s))
            })
            .collect()
    }
    fn rebuild(&mut self, registry: &[PlottedChannelInfo], groups: Vec<ReportGroup>) {
        let keys: Vec<_> = registry
            .iter()
            .chain(self.external_sources.iter().flatten())
            .map(ChannelKey::from)
            .collect();
        let reference_key = format!(
            "{:?}:{:?}:{:?}",
            self.reference_groups, self.settings.kind, self.settings.reference
        );
        let references_changed = self.reference_key != reference_key;
        if self.groups != groups
            || self.channel_keys != keys
            || self.boundary_key != self.settings.boundary
            || references_changed
        {
            self.results = registry
                .iter()
                .map(|info| {
                    let c = registry.iter().position(|i| std::ptr::eq(i, info)).unwrap();
                    groups
                        .iter()
                        .map(|g| self.summarize(c, g, registry))
                        .collect()
                })
                .collect();
            self.reference_results = std::array::from_fn(|r| {
                (0..registry.len())
                    .map(|c| {
                        let windows = self.reference_groups[r]
                            .iter()
                            .flat_map(|g| g.windows.iter().copied())
                            .collect::<Vec<_>>();
                        self.reference_groups[r]
                            .first()
                            .map_or_else(ReportSummary::default, |g| {
                                let mut group = g.clone();
                                group.windows = windows;
                                self.summarize(c, &group, registry)
                            })
                    })
                    .collect()
            });
            self.reference_key = reference_key;
            self.channel_keys = keys;
            self.groups = groups;
            self.boundary_key = self.settings.boundary;
        }
        self.sources = registry.to_vec();
        self.retained_external_sources = self.external_sources.clone();
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        if self.last_shared_selection != shared.selected_lap {
            self.selected_group = None;
            self.last_shared_selection = shared.selected_lap;
        }
        if self.settings.kind == ReportKind::Acceleration && self.channels.is_empty() {
            self.add_channel(shared.analysis_channel("MoTeC::Corr Speed"), shared);
        }
        self.settings
            .channel_statistics
            .resize(self.channels.len(), None);

        if let Some(id) = shared.pending_toggle_channel.take() {
            if let Some(index) = self.channels.iter().position(|c| c.channel_id == id) {
                self.channels.remove(index);
                self.settings.channel_statistics.remove(index);
            } else {
                self.add_channel(id, shared);
            }
        }
        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
            && let Some(id) = shared.dragging_channel.take()
        {
            self.add_channel(id, shared);
        }
        if self.settings.kind != ReportKind::Time {
            ui.horizontal_wrapped(|ui| {
                ui.label(if self.settings.kind == ReportKind::Acceleration {
                    "Speed channel (first slot):"
                } else {
                    "Add channel:"
                });
                let mut selected = None;
                channel_picker(
                    ui,
                    ("report_add", self.id),
                    &mut selected,
                    &mut self.filter,
                    shared,
                );
                if let Some(id) = selected {
                    self.add_channel(id, shared);
                }
                let mut remove = None;
                for (idx, pc) in self.channels.iter_mut().enumerate() {
                    refresh_plotted_channel(pc, shared);
                    let name = resolve_plotted_channel_display_meta(pc, shared).0;
                    let (button, clear) = super::utils::segmented_channel_button(
                        ui,
                        &name,
                        None,
                        "Remove from report",
                    );
                    button.context_menu(|ui| {
                        ui.menu_button("Report statistics", |ui| {
                            let mut inherit = self.settings.channel_statistics[idx].is_none();
                            if ui.checkbox(&mut inherit, "Use common statistics").changed() {
                                self.settings.channel_statistics[idx] = if inherit {
                                    None
                                } else {
                                    Some(self.settings.statistics.clone())
                                };
                            }
                            if let Some(stats) = &mut self.settings.channel_statistics[idx] {
                                for stat in ReportStatistic::ALL {
                                    let mut on = stats.contains(&stat);
                                    if ui.checkbox(&mut on, stat.label()).changed() {
                                        stats.retain(|s| *s != stat);
                                        if on {
                                            stats.push(stat);
                                        }
                                    }
                                }
                            }
                        });
                        if show_plotted_channel_display_menu(ui, pc, shared) {
                            remove = Some(idx);
                        }
                    });
                    if clear {
                        remove = Some(idx);
                    }
                }
                if let Some(idx) = remove {
                    self.channels.remove(idx);
                    self.settings.channel_statistics.remove(idx);
                }
            });
        }
        self.controls(ui, shared);
        if self.settings.drag_milestones {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Run details / time slip…").clicked() { shared.show_recording_editor = true; }
                if let Some(run) = shared.selected_lap.and_then(|i| shared.current_analysis()?.ranges.get(i))
                    && let Some(reaction) = run.reaction_time {
                    ui.label(format!("Reaction: {reaction:.3} s (excluded from elapsed times)"));
                }
                ui.weak("Time-slip elapsed values take precedence; other milestones use corrected distance.");
            });
        }
        if self.settings.kind == ReportKind::Acceleration {
            self.acceleration_ui(ui, shared);
            return;
        }
        if ui.ui_contains_pointer()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::G))
        {
            if ui.input(|i| i.modifiers.shift) {
                self.settings.split_horizontal = !self.settings.split_horizontal;
            } else {
                self.settings.view = match self.settings.view {
                    ReportView::Table => ReportView::Chart,
                    ReportView::Chart => ReportView::Split,
                    ReportView::Split => ReportView::Table,
                };
            }
        }
        ui.separator();
        if self.channels.is_empty() && self.settings.kind != ReportKind::Time {
            ui.label("Add or drop channels to build this report");
            return;
        }
        let mut registry: Vec<_> = self
            .channels
            .iter()
            .map(|pc| build_plotted_channel_info(pc, shared))
            .collect();
        if self.settings.kind == ReportKind::Time {
            registry = vec![PlottedChannelInfo {
                channel_id: ChannelId::Unresolved(usize::MAX),
                name: "Elapsed time".into(),
                unit: "s".into(),
                freq: 0.0,
                dec_places: 3,
                color: egui::Color32::WHITE,
                data: std::sync::Arc::from([]),
                display_scale: 1.,
                display_offset: 0.,
                interpolation: i3rs_core::Interpolation::Linear,
                enum_labels: Default::default(),
            }];
        }
        let grouping_key = format!(
            "{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}",
            (
                self.settings.scope,
                self.settings.trusted_only,
                &self.settings.section_id,
                self.settings.reverse,
                self.settings.sections_as_headings,
                self.settings.range_kind,
                self.settings.drag_milestones,
                self.settings.drag_cumulative
            ),
            (
                shared.session_id,
                shared.correction_revision,
                shared.recording_key()
            ),
            (shared.selected_lap, shared.comparisons.main_lap),
            shared.analysis_range(),
            shared.data_duration,
            shared.laps,
            shared.current_analysis().map(|a| (&a.ranges, &a.drag_runs)),
            shared.sectors,
            (
                shared
                    .track_data_if_ready()
                    .map(|t| std::sync::Arc::as_ptr(&t) as usize),
                shared
                    .correction_distance
                    .as_ref()
                    .map(|(d, _)| d.as_ptr() as usize)
            ),
        );
        let mut groups = if grouping_key != self.grouping_key {
            let (groups, diagnostic) = report_groups(&self.settings, shared);
            self.diagnostic = diagnostic;
            self.grouping_key = grouping_key;
            groups
        } else {
            self.groups
                .iter()
                .filter(|g| g.source.is_none())
                .cloned()
                .collect()
        };
        if let Some(message) = &self.diagnostic {
            ui.colored_label(egui::Color32::YELLOW, message);
        }
        if self.settings.kind != ReportKind::Time
            && registry.iter().any(|c| c.freq == 0.0 || c.data.is_empty())
        {
            ui.weak("Some channels are unavailable or loading; their results are shown as —");
        }
        self.prepare_populations(shared, &registry, &mut groups);
        if shared
            .comparisons
            .sessions
            .iter()
            .any(|s| !s.requests.is_empty())
            || self.population_contexts.iter().any(|s| s.is_loading())
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        if self.settings.kind != ReportKind::Time
            && self
                .external_sources
                .iter()
                .flatten()
                .any(|i| i.freq == 0.0 || i.data.is_empty())
        {
            ui.weak("Some outing channels are unavailable or loading; their results are shown as —. Details appear in Status / Messages.");
        }
        for message in &self.population_diagnostics {
            ui.colored_label(egui::Color32::YELLOW, message);
        }
        self.rebuild(&registry, groups);
        if self
            .selected_group
            .as_ref()
            .is_some_and(|id| !self.groups.iter().any(|g| &g.id == id))
        {
            self.selected_group = None;
        }
        if self.selected_group.is_none() {
            self.selected_group = self
                .groups
                .iter()
                .find(|g| g.source.is_none() && g.lap_index == shared.selected_lap)
                .map(|g| g.id.clone());
        }
        if self.settings.reference != ReportReference::Absolute
            && self.reference_groups
                [usize::from(self.settings.reference == ReportReference::Overlay0)]
            .is_empty()
        {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Comparison reference is unavailable; deltas are shown as —",
            );
        }
        if self.settings.kind != ReportKind::Time {
            shared
                .plotted_channel_registry
                .extend(registry.iter().cloned());
        }
        if self.visible_statistics().is_empty() {
            ui.label("Select at least one statistic");
            return;
        }
        ui.horizontal_wrapped(|ui| crate::output_ui::result_buttons(ui, &self.csv()));
        ui.weak(if self.settings.kind==ReportKind::Time {"Duration of selected intervals [s]; click a cell or chart to select its range."}else{"Sample-weighted statistics; click a cell, range or chart bar to select it. Blue = minimum, red = maximum when enabled."});
        let mut selection = None;
        if self.settings.view == ReportView::Split && self.settings.split_horizontal {
            ui.columns(2, |columns| {
                egui::ScrollArea::both()
                    .id_salt(("report_table_scroll", self.id))
                    .auto_shrink([false, false])
                    .show(&mut columns[0], |ui| {
                        selection = self.table(ui, shared, &registry);
                    });
                egui::ScrollArea::both()
                    .id_salt(("report_chart_scroll", self.id))
                    .auto_shrink([false, false])
                    .show(&mut columns[1], |ui| {
                        selection = self.charts(ui, shared, &registry).or(selection);
                    });
            });
        } else {
            if self.settings.view != ReportView::Chart {
                let height = if self.settings.view == ReportView::Split {
                    ui.available_height() * 0.45
                } else {
                    ui.available_height()
                };
                egui::ScrollArea::both()
                    .id_salt(("report_table_scroll", self.id))
                    .max_height(height.max(0.))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        selection = self.table(ui, shared, &registry);
                    });
            }
            if self.settings.view == ReportView::Split {
                ui.separator();
            }
            if self.settings.view != ReportView::Table {
                egui::ScrollArea::both()
                    .id_salt(("report_chart_scroll", self.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        selection = self.charts(ui, shared, &registry).or(selection);
                    });
            }
        }
        if let Some(group) = selection {
            self.selected_group = Some(self.groups[group].id.clone());
            if self.settings.scope != ReportScope::AnalysisWindow
                && self.groups[group].source.is_none()
            {
                shared.select_lap(self.groups[group].lap_index);
            }
        }
    }
    fn table(
        &self,
        ui: &mut egui::Ui,
        shared: &SharedState,
        registry: &[PlottedChannelInfo],
    ) -> Option<usize> {
        let mut selection = None;
        egui::Grid::new(("report_grid", self.id))
            .spacing([12., 4.])
            .striped(true)
            .show(ui, |ui| {
                if self.settings.channels_in_columns {
                    ui.strong("Range");
                    for info in registry {
                        for stat in &self.visible_statistics() {
                            ui.colored_label(
                                info.color,
                                format!(
                                    "{} [{}] · {}",
                                    info.name,
                                    info.unit,
                                    self.stat_label(*stat)
                                ),
                            );
                        }
                    }
                    ui.end_row();
                    for (g, group) in self.groups.iter().enumerate() {
                        if group_label(ui, group, shared, self.selected_group.as_deref()).clicked()
                        {
                            selection = Some(g);
                        }
                        for (c, info) in registry.iter().enumerate() {
                            for stat in &self.visible_statistics() {
                                if self.cell(ui, c, g, *stat, info) {
                                    selection = Some(g);
                                }
                            }
                        }
                        ui.end_row();
                    }
                } else {
                    ui.strong("Channel");
                    ui.strong("Range");
                    for stat in &self.visible_statistics() {
                        ui.strong(self.stat_label(*stat));
                    }
                    ui.end_row();
                    for (c, info) in registry.iter().enumerate() {
                        for (g, group) in self.groups.iter().enumerate() {
                            ui.colored_label(info.color, format!("{} [{}]", info.name, info.unit));
                            if group_label(ui, group, shared, self.selected_group.as_deref())
                                .clicked()
                            {
                                selection = Some(g);
                            }
                            for stat in &self.visible_statistics() {
                                if self.cell(ui, c, g, *stat, info) {
                                    selection = Some(g);
                                }
                            }
                            ui.end_row();
                        }
                    }
                }
            });
        selection
    }
    fn cell(
        &self,
        ui: &mut egui::Ui,
        channel: usize,
        group: usize,
        stat: ReportStatistic,
        info: &PlottedChannelInfo,
    ) -> bool {
        let summary = &self.results[channel][group];
        let value = self.value(channel, group, stat);
        let color = self
            .highlight(channel, stat, value)
            .unwrap_or(ui.visuals().text_color());
        let text = value
            .map(|v| format!("{v:.prec$}", prec = info.dec_places.clamp(0, 12) as usize))
            .unwrap_or_else(|| "—".into());
        ui.selectable_label(
            self.selected_group.as_ref() == Some(&self.groups[group].id),
            egui::RichText::new(text).monospace().color(color),
        )
        .on_hover_text(format!(
            "{} finite / {} samples ({} native); {} missing",
            summary.finite_count,
            summary.sample_count,
            summary.native_count,
            summary.sample_count - summary.finite_count
        ))
        .clicked()
    }
    fn highlight(
        &self,
        channel: usize,
        stat: ReportStatistic,
        value: Option<f64>,
    ) -> Option<egui::Color32> {
        if !self.settings.highlight_min && !self.settings.highlight_max {
            return None;
        }
        let value = value?;
        let (min, max) = (0..self.groups.len())
            .filter_map(|g| self.value(channel, g, stat))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                (a.min(v), b.max(v))
            });
        if min == max {
            return None;
        }
        if self.settings.highlight_min && value == min {
            Some(egui::Color32::LIGHT_BLUE)
        } else if self.settings.highlight_max && value == max {
            Some(egui::Color32::LIGHT_RED)
        } else {
            None
        }
    }
    fn chart_statistics(&self, channel: usize) -> Vec<ReportStatistic> {
        let candidates = if self.settings.kind == ReportKind::Time {
            vec![ReportStatistic::Average]
        } else if self.settings.chart_statistics.is_empty() {
            vec![self.settings.chart_statistic]
        } else {
            self.settings.chart_statistics.clone()
        };
        candidates
            .into_iter()
            .filter(|stat| {
                self.settings.kind == ReportKind::Time
                    || self
                        .settings
                        .channel_statistics
                        .get(channel)
                        .and_then(Option::as_ref)
                        .unwrap_or(&self.settings.statistics)
                        .contains(stat)
            })
            .collect()
    }
    fn charts(
        &self,
        ui: &mut egui::Ui,
        shared: &SharedState,
        registry: &[PlottedChannelInfo],
    ) -> Option<usize> {
        use egui_plot::{Bar, BarChart, Line, Plot};
        let mut selection = None;
        for (c, info) in registry.iter().enumerate() {
            let statistics = self.chart_statistics(c);
            ui.colored_label(
                info.color,
                format!(
                    "{} [{}] · {}",
                    info.name,
                    info.unit,
                    statistics
                        .iter()
                        .map(|s| self.stat_label(*s))
                        .collect::<Vec<_>>()
                        .join(" / ")
                ),
            );
            Plot::new(("report_chart", self.id, c))
                .height(190.)
                .include_y(0.)
                .x_axis_label("Range (click a bar to select)")
                .y_axis_label(&info.unit)
                .x_axis_formatter(|mark, _| {
                    let i = mark.value.round();
                    if (mark.value - i).abs() < 1e-6 && i >= 0. {
                        self.groups
                            .get(i as usize)
                            .map_or_else(String::new, |g| g.label.clone())
                    } else {
                        String::new()
                    }
                })
                .show(ui, |plot| {
                    for (s, &stat) in statistics.iter().enumerate() {
                        let width = 0.65 / statistics.len().max(1) as f64;
                        let offset = (s as f64 + 0.5) * width - 0.325;
                        let series_color = if statistics.len() == 1 {
                            info.color
                        } else {
                            match stat {
                                ReportStatistic::Min => egui::Color32::LIGHT_BLUE,
                                ReportStatistic::Max => egui::Color32::LIGHT_RED,
                                _ => egui::Color32::GOLD,
                            }
                        };
                        let bars: Vec<_> = self
                            .groups
                            .iter()
                            .enumerate()
                            .filter_map(|(g, group)| {
                                let value = self.value(c, g, stat)?;
                                let selected = self.selected_group.as_ref().map_or(
                                    group.source.is_none()
                                        && group.lap_index == shared.selected_lap,
                                    |id| id == &group.id,
                                );
                                let color =
                                    self.highlight(c, stat, Some(value)).unwrap_or(series_color);
                                Some(
                                    Bar::new(g as f64 + offset, value)
                                        .width(width)
                                        .name(&group.label)
                                        .fill(color)
                                        .stroke(egui::Stroke::new(
                                            if selected { 2. } else { 0. },
                                            egui::Color32::WHITE,
                                        )),
                                )
                            })
                            .collect();
                        plot.bar_chart(BarChart::new(self.stat_label(stat), bars));
                        let mut run = Vec::new();
                        for g in 0..self.groups.len() {
                            if let Some(value) = self.value(c, g, stat) {
                                run.push([g as f64, value]);
                            } else if !run.is_empty() {
                                plot.line(
                                    Line::new("Trend", std::mem::take(&mut run))
                                        .color(series_color),
                                );
                            }
                        }
                        if !run.is_empty() {
                            plot.line(Line::new("Trend", run).color(series_color));
                        }
                    }
                    if plot.response().clicked()
                        && let Some(p) = plot.pointer_coordinate()
                    {
                        let index = p.x.round();
                        if index >= 0.
                            && (index as usize) < self.groups.len()
                            && (p.x - index).abs() <= 0.45
                            && statistics
                                .iter()
                                .any(|stat| self.value(c, index as usize, *stat).is_some())
                        {
                            selection = Some(index as usize);
                        }
                    }
                });
        }
        selection
    }
    fn csv(&self) -> String {
        let mut csv =
            String::from("Channel,Unit,Range,Statistic,Value,Samples,Finite,Native,Reference\n");
        for (c, info) in self.sources.iter().enumerate() {
            for (g, group) in self.groups.iter().enumerate() {
                let summary = &self.results[c][g];
                for stat in &self.visible_statistics() {
                    let cells = [
                        info.name.clone(),
                        info.unit.clone(),
                        group.label.clone(),
                        self.stat_label(*stat).into(),
                        self.value(c, g, *stat)
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        summary.sample_count.to_string(),
                        summary.finite_count.to_string(),
                        summary.native_count.to_string(),
                        format!("{:?}", self.settings.reference),
                    ];
                    csv.push_str(
                        &cells
                            .iter()
                            .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    csv.push('\n');
                }
            }
        }
        csv
    }
}

fn group_label(
    ui: &mut egui::Ui,
    group: &ReportGroup,
    shared: &SharedState,
    selected: Option<&str>,
) -> egui::Response {
    ui.selectable_label(
        selected.map_or(
            group.source.is_none() && group.lap_index == shared.selected_lap,
            |id| id == group.id,
        ),
        &group.label,
    )
}

fn report_groups(
    settings: &ReportSettings,
    shared: &SharedState,
) -> (Vec<ReportGroup>, Option<String>) {
    let mut groups = Vec::new();
    let outing = || ReportGroup {
        source: None,
        section: None,
        id: "outing".into(),
        label: "All".into(),
        lap_index: None,
        windows: vec![(0., shared.data_duration.unwrap_or(0.))],
    };
    let range = |i: usize| {
        shared.laps.get(i).map(|lap| ReportGroup {
            source: None,
            section: None,
            id: shared
                .current_analysis()
                .and_then(|a| a.ranges.get(i))
                .map_or_else(|| format!("range:{i}"), |r| r.id.clone()),
            label: lap.name.clone(),
            lap_index: Some(i),
            windows: vec![(lap.start_time, lap.end_time)],
        })
    };
    match settings.scope {
        ReportScope::Outing | ReportScope::SelectedOutings => groups.push(outing()),
        ReportScope::OutingAndRanges | ReportScope::Ranges | ReportScope::AllLoaded => {
            if matches!(
                settings.scope,
                ReportScope::OutingAndRanges | ReportScope::AllLoaded
            ) {
                groups.push(outing());
            }
            groups.extend((0..shared.laps.len()).filter_map(range));
        }
        ReportScope::SelectedRange | ReportScope::SelectedRanges => {
            let selected = if settings.scope == ReportScope::SelectedRanges {
                shared.comparisons.main_lap.or(shared.selected_lap)
            } else {
                shared.selected_lap
            };
            if let Some(group) = selected.and_then(range) {
                groups.push(group);
            } else {
                return (groups, Some("Select a range to build this report".into()));
            }
        }
        ReportScope::AnalysisWindow => groups.push(ReportGroup {
            source: None,
            section: None,
            id: "window".into(),
            label: "Analysis window".into(),
            lap_index: shared.selected_lap,
            windows: vec![shared.analysis_range()],
        }),
    }
    if let Some(kind) = settings.range_kind {
        groups.retain(|g| {
            g.lap_index
                .and_then(|i| shared.current_analysis()?.ranges.get(i))
                .is_some_and(|r| r.kind == kind)
        });
    }
    if settings.drag_milestones {
        shared.request_track_data_build();
        let mut expanded = Vec::new();
        for group in groups {
            let Some(run) = group
                .lap_index
                .and_then(|i| shared.current_analysis()?.ranges.get(i))
            else {
                continue;
            };
            if run.kind != i3rs_core::ranges::RangeKind::Run {
                continue;
            }
            let times = i3rs_core::drag::milestone_times(
                run,
                shared
                    .current_analysis()
                    .and_then(|a| a.drag_runs.get(&run.id)),
                shared
                    .correction_distance
                    .as_ref()
                    .map(|(d, hz)| (d.as_ref(), *hz)),
            );
            let mut previous = Some(0.);
            for (ft, elapsed) in i3rs_core::drag::MILESTONES_FT.into_iter().zip(times) {
                let start = if settings.drag_cumulative {
                    Some(0.)
                } else {
                    previous
                };
                let windows = start
                    .zip(elapsed)
                    .map(|(a, b)| vec![(run.start + a, run.start + b)])
                    .unwrap_or_default();
                let mut row = group.clone();
                row.id = format!("{}:{ft}ft", group.id);
                row.section = Some(format!("drag:{ft}"));
                row.label = format!("{} · {ft} ft", group.label);
                row.windows = windows;
                expanded.push(row);
                previous = elapsed;
            }
        }
        groups = expanded;
        if groups.is_empty() {
            return (
                groups,
                Some("Select a Run range; create or edit runs in Run details / time slip".into()),
            );
        }
    }
    if settings.sections_as_headings {
        let mut expanded = Vec::new();
        let mut diagnostic = None;
        for section in &shared.sectors {
            let mut sub = settings.clone();
            sub.sections_as_headings = false;
            sub.section_id = Some(section.semantics.id.clone());
            let (mut rows, message) = report_groups(&sub, shared);
            for row in &mut rows {
                row.section = Some(section.semantics.id.clone());
                row.id = format!("{}:{}", row.id, section.semantics.id);
                row.label = format!("{} · {}", row.label, section.name);
            }
            expanded.extend(rows);
            if message.is_some() {
                diagnostic = message;
            }
        }
        if shared.sectors.is_empty() {
            diagnostic = Some("No sections are defined for this recording".into());
        }
        return (expanded, diagnostic);
    }
    let mut diagnostic = None;
    if let Some(id) = &settings.section_id {
        let section = shared.sectors.iter().find(|s| &s.semantics.id == id);
        let track = shared.track_data_if_ready();
        shared.request_track_data_build();
        let mut mask = Vec::new();
        if let Some((section, track)) = section.zip(track) {
            for lap in &shared.laps {
                if let Some((a, b)) = i3rs_core::sector_interval(section, lap, &track) {
                    if b < a {
                        mask.extend([(a, lap.end_time), (lap.start_time, b)]);
                    } else {
                        mask.push((a, b));
                    }
                }
            }
        } else {
            diagnostic = Some("Selected section is unavailable or track data is loading".into());
        }
        for group in &mut groups {
            group.windows = intersect_report_windows(&group.windows, &mask);
        }
    }
    if settings.trusted_only {
        let trusted: Vec<_> = shared
            .current_analysis()
            .into_iter()
            .flat_map(|a| &a.ranges)
            .filter(|r| r.trust == i3rs_core::ranges::Trust::Trusted)
            .map(|r| (r.start, r.end))
            .collect();
        for group in &mut groups {
            group.windows = intersect_report_windows(&group.windows, &trusted);
        }
    }
    if groups.iter().all(|g| g.windows.is_empty()) && diagnostic.is_none() {
        diagnostic = Some(if settings.drag_milestones {
            "No eligible milestones: check the run's time slip, corrected distance and trust selection"
        } else {
            "No intervals satisfy the report's range, section and trust selection"
        }.into());
    }
    if settings.reverse {
        groups.reverse();
    }
    (groups, diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use i3rs_core::{
        Lap,
        ranges::{RecordingAnalysis, Trust},
    };
    use std::sync::Arc;

    fn state() -> SharedState {
        let mut shared = SharedState::new();
        shared.data_duration = Some(4.);
        shared.laps = vec![
            Lap {
                number: 0,
                name: "Out lap".into(),
                start_time: 0.,
                end_time: 1.25,
            },
            Lap {
                number: 1,
                name: "Timed range".into(),
                start_time: 1.25,
                end_time: 4.,
            },
        ];
        let key = shared.recording_key().to_string();
        let mut analysis = RecordingAnalysis::from_laps(key.clone(), 4., &shared.laps);
        analysis.ranges[0].trust = Trust::Untrusted;
        analysis.ranges[1].trust = Trust::Trusted;
        shared.recording_analyses.insert(key, analysis);
        shared
    }
    fn info() -> PlottedChannelInfo {
        PlottedChannelInfo {
            channel_id: ChannelId::Math(0),
            name: "Speed, \"test\"".into(),
            unit: "km/h".into(),
            freq: 1.0,
            dec_places: 1,
            color: egui::Color32::GREEN,
            data: Arc::from(vec![0., 10., 20., 30.]),
            display_scale: 1.,
            display_offset: 0.,
            interpolation: i3rs_core::Interpolation::Linear,
            enum_labels: Default::default(),
        }
    }
    #[test]
    fn drag_deltas_compare_matching_distance_milestones_and_csv_matches_values() {
        let mut shared = state();
        let key = shared.recording_key().to_string();
        let a = shared.recording_analyses.get_mut(&key).unwrap();
        a.ranges[1].kind = i3rs_core::ranges::RangeKind::Run;
        a.drag_runs.insert(
            a.ranges[1].id.clone(),
            i3rs_core::drag::RunDetails {
                elapsed: [Some(0.5), Some(1.25), None, None, None],
            },
        );
        shared.selected_lap = Some(1);
        let mut panel = ReportPanel::new(1, "Drag");
        panel.settings = ReportSettings {
            kind: ReportKind::Time,
            scope: ReportScope::SelectedRange,
            drag_milestones: true,
            reference: ReportReference::Main,
            ..Default::default()
        };
        let groups = report_groups(&panel.settings, &shared).0;
        let mut reference = groups.clone();
        reference[0].windows = vec![(0., 0.25)];
        reference[1].windows = vec![(0.25, 0.75)];
        panel.reference_groups[0] = reference;
        let mut time = info();
        time.name = "Elapsed time".into();
        time.unit = "s".into();
        time.dec_places = 3;
        panel.rebuild(&[time], groups);
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), Some(0.25));
        assert_eq!(panel.value(0, 1, ReportStatistic::Average), Some(0.25));
        let csv = panel.csv();
        assert!(csv.contains("330 ft") && csv.contains("0.25") && csv.contains("Main"));
    }

    #[test]
    fn drag_reports_use_run_splits_official_elapsed_times_and_stage_reports_exclude_transport() {
        let mut shared = state();
        let key = shared.recording_key().to_string();
        let a = shared.recording_analyses.get_mut(&key).unwrap();
        a.ranges[0].kind = i3rs_core::ranges::RangeKind::Transport;
        a.ranges[1].kind = i3rs_core::ranges::RangeKind::Run;
        a.ranges[1].reaction_time = Some(0.4);
        let id = a.ranges[1].id.clone();
        a.drag_runs.insert(
            id,
            i3rs_core::drag::RunDetails {
                elapsed: [Some(0.3), Some(0.8), Some(1.3), Some(2.), Some(2.5)],
            },
        );
        shared.selected_lap = Some(1);
        let mut settings = ReportSettings {
            scope: ReportScope::SelectedRange,
            drag_milestones: true,
            range_kind: Some(i3rs_core::ranges::RangeKind::Run),
            ..Default::default()
        };
        let (groups, error) = report_groups(&settings, &shared);
        assert!(error.is_none());
        assert_eq!(groups.len(), 5);
        assert_eq!(groups[0].windows, [(1.25, 1.55)]);
        assert_eq!(groups[1].windows, [(1.55, 2.05)]);
        assert_eq!(groups[4].windows, [(3.25, 3.75)]);
        assert_eq!(groups[1].section.as_deref(), Some("drag:330"));
        settings.drag_cumulative = true;
        assert_eq!(
            report_groups(&settings, &shared).0[4].windows,
            [(1.25, 3.75)]
        );
        settings.trusted_only = true;
        shared.recording_analyses.get_mut(&key).unwrap().ranges[1].trust = Trust::Untrusted;
        assert!(
            report_groups(&settings, &shared)
                .0
                .iter()
                .all(|g| g.windows.is_empty())
        );
        let a = shared.recording_analyses.get_mut(&key).unwrap();
        a.ranges[1].kind = i3rs_core::ranges::RangeKind::Stage;
        settings = ReportSettings {
            scope: ReportScope::Ranges,
            range_kind: Some(i3rs_core::ranges::RangeKind::Stage),
            ..Default::default()
        };
        let groups = report_groups(&settings, &shared).0;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lap_index, Some(1));
    }

    #[test]
    fn scopes_and_trust_use_recording_ranges_and_preserve_off_grid_boundaries() {
        let mut shared = state();
        let mut settings = ReportSettings::default();
        let (groups, error) = report_groups(&settings, &shared);
        assert!(error.is_none());
        assert_eq!(groups.len(), 3);
        settings.trusted_only = true;
        let (groups, _) = report_groups(&settings, &shared);
        assert_eq!(groups[0].windows, [(1.25, 4.)]);
        assert!(groups[1].windows.is_empty());
        assert_eq!(groups[2].windows, [(1.25, 4.)]);
        settings.scope = ReportScope::SelectedRange;
        assert!(report_groups(&settings, &shared).1.is_some());
        shared.select_lap(Some(1));
        let (groups, _) = report_groups(&settings, &shared);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lap_index, Some(1));
        settings.scope = ReportScope::AnalysisWindow;
        shared.zoom_range = Some((1.51, 2.75));
        assert_eq!(
            report_groups(&settings, &shared).0[0].windows,
            [(1.51, 2.75)]
        );
        settings.scope = ReportScope::Ranges;
        settings.reverse = true;
        assert_eq!(report_groups(&settings, &shared).0[0].lap_index, Some(1));
    }
    #[test]
    fn section_selection_splits_beacon_wrap_and_reports_missing_sections() {
        let mut shared = state();
        shared.laps = vec![Lap {
            number: 1,
            name: "Lap".into(),
            start_time: 0.,
            end_time: 4.,
        }];
        shared.store_track_data(Some(i3rs_core::TrackData::from_normalized_parts(
            vec![0., 0.001, 0.002, 0.003, 0.004],
            vec![0.; 5],
            vec![0., 1., 2., 3., 4.],
            1.0,
        )));
        shared.sectors.push(i3rs_core::Sector {
            name: "Across beacon".into(),
            semantics: i3rs_core::track::SectionSemantics {
                id: "wrap".into(),
                ..Default::default()
            },
            start_index: 3,
            end_index: 1,
            distance: None,
        });
        let mut settings = ReportSettings {
            scope: ReportScope::Outing,
            section_id: Some("wrap".into()),
            ..Default::default()
        };
        let (groups, diagnostic) = report_groups(&settings, &shared);
        assert!(diagnostic.is_none());
        assert_eq!(groups[0].windows, [(0., 1.), (3., 4.)]);
        settings.section_id = Some("absent".into());
        let (groups, diagnostic) = report_groups(&settings, &shared);
        assert!(groups[0].windows.is_empty());
        assert!(diagnostic.is_some());
        assert_eq!(settings.section_id.as_deref(), Some("absent"));
    }
    #[test]
    fn cache_tracks_data_transforms_range_boundaries_interpolation_and_missing_values() {
        let shared = state();
        let settings = ReportSettings {
            scope: ReportScope::AnalysisWindow,
            ..Default::default()
        };
        let mut panel = ReportPanel::new(1, "Report");
        panel.settings = settings;
        let mut source = info();
        let mut groups = report_groups(&panel.settings, &shared).0;
        groups[0].windows = vec![(0.25, 2.25)];
        panel.rebuild(&[source.clone()], groups.clone());
        assert_eq!(panel.results[0][0].average, Some(15.));
        source.display_scale = -2.;
        source.display_offset = 5.;
        panel.rebuild(&[source.clone()], groups.clone());
        assert_eq!(panel.results[0][0].min, Some(-35.));
        panel.settings.boundary = ReportBoundary::Interpolate;
        panel.rebuild(&[source.clone()], groups.clone());
        assert_eq!(panel.results[0][0].start, Some(0.));
        source.interpolation = i3rs_core::Interpolation::Previous;
        panel.rebuild(&[source.clone()], groups.clone());
        assert_eq!(panel.results[0][0].start, Some(5.));
        groups[0].windows = vec![(1., 2.)];
        source.data = Arc::from(vec![f64::NAN; 4]);
        panel.rebuild(&[source], groups);
        assert_eq!(panel.results[0][0].min, None);
    }
    #[test]
    fn table_chart_and_csv_share_selected_statistics_without_nan_or_invented_zero() {
        let shared = state();
        let mut panel = ReportPanel::new(1, "Report");
        panel.settings.statistics = vec![ReportStatistic::Range, ReportStatistic::Change];
        panel.rebuild(&[info()], report_groups(&panel.settings, &shared).0);
        let csv = panel.csv();
        assert!(csv.contains("\"Speed, \"\"test\"\"\",\"km/h\",\"All\",\"Range\",\"30\""));
        assert!(csv.contains("\"Timed range\",\"Change\",\"10\""));
        assert!(!csv.contains("\"Avg\""));
        assert_eq!(panel.results[0][0].value(ReportStatistic::Range), Some(30.));
        let mut source = info();
        source.data = Arc::from(vec![f64::NAN; 4]);
        panel.rebuild(&[source], report_groups(&panel.settings, &shared).0);
        let csv = panel.csv();
        assert!(csv.contains("\"All\",\"Range\",\"\",\"4\",\"0\",\"4\""));
        assert!(!csv.contains("NaN"));
    }
    #[test]
    fn workspace_migrates_legacy_reports_and_round_trips_all_settings_and_channel_ownership() {
        let legacy: crate::workspace::ReportPanelConfig =
            serde_json::from_str(r#"{"id":7,"title":"Old report"}"#).unwrap();
        assert_eq!(legacy.settings, ReportSettings::default());
        let mut shared = state();
        let mut def =
            shared.create_math_channel_def("Report only".into(), "1".into(), "rpm".into(), 1);
        def.freq = 1.0;
        def.data = Some(Arc::from(vec![1., 2., 3., 4.]));
        shared.math_channels.push(def);
        let mut panel = ReportPanel::new(7, "Configured report");
        panel.add_channel(ChannelId::Math(0), &shared);
        panel.settings = ReportSettings {
            scope: ReportScope::AnalysisWindow,
            statistics: ReportStatistic::ALL.into(),
            boundary: ReportBoundary::Interpolate,
            trusted_only: true,
            section_id: Some("missing".into()),
            view: ReportView::Split,
            channels_in_columns: true,
            reverse: true,
            highlight_min: true,
            highlight_max: true,
            chart_statistic: ReportStatistic::Change,
            chart_statistics: vec![ReportStatistic::Min, ReportStatistic::Max],
            reference: ReportReference::Overlay0,
            split_horizontal: true,
            channel_statistics: vec![Some(vec![ReportStatistic::Range])],
            acceleration: i3rs_core::acceleration::AccelerationSettings {
                hysteresis_mps: 1.2,
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = panel.settings.clone();
        let dock = egui_dock::DockState::new(vec![crate::panels::PanelTab::Report(panel)]);
        let saved = crate::workspace::save_workspace(&[("Reports".into(), &dock)], 0, &shared);
        let decoded = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        let loaded = crate::workspace::load_workspace(&decoded, &mut shared);
        let (_, crate::panels::PanelTab::Report(report)) =
            loaded[0].1.iter_all_tabs().next().unwrap()
        else {
            panic!()
        };
        assert_eq!(report.settings, expected);
        assert_eq!(report.channels.len(), 1);
        assert!(shared.display_channel_registry.is_empty());
    }
    #[test]
    fn profile_charts_use_each_channels_selected_statistics() {
        use ReportStatistic::{Average, Max, Min};
        let mut panel = ReportPanel::new(1, "Stages");
        panel.settings.statistics = vec![Min, Max, Average];
        panel.settings.chart_statistics = vec![Min, Max, Average];
        panel.settings.channel_statistics = vec![Some(vec![Min]), None, Some(vec![Max])];
        assert_eq!(panel.chart_statistics(0), vec![Min]);
        assert_eq!(panel.chart_statistics(1), vec![Min, Max, Average]);
        assert_eq!(panel.chart_statistics(2), vec![Max]);
        panel.settings.kind = ReportKind::Time;
        assert_eq!(panel.chart_statistics(0), vec![Average]);
    }

    #[test]
    fn every_view_and_orientation_renders_without_mutating_shared_selection_or_results() {
        let mut shared = state();
        shared.select_lap(Some(1));
        let mut def =
            shared.create_math_channel_def("Report only".into(), "1".into(), "rpm".into(), 1);
        def.freq = 1.0;
        def.data = Some(Arc::from(vec![0., 10., 20., 30.]));
        shared.math_channels.push(def);
        let mut panel = ReportPanel::new(1, "Report");
        panel.add_channel(ChannelId::Math(0), &shared);
        panel.settings.statistics = ReportStatistic::ALL.into();
        panel.settings.chart_statistics = vec![
            ReportStatistic::Min,
            ReportStatistic::Max,
            ReportStatistic::Average,
        ];
        let ctx = egui::Context::default();
        let mut expected = None;
        for view in [ReportView::Table, ReportView::Chart, ReportView::Split] {
            panel.settings.view = view;
            for columns in [false, true] {
                panel.settings.channels_in_columns = columns;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1600., 1000.),
                        )),
                        ..Default::default()
                    },
                    |ui| panel.ui(ui, &mut shared),
                );
                output.textures_delta.clear();
                let csv = panel.csv();
                if let Some(expected) = &expected {
                    assert_eq!(expected, &csv);
                } else {
                    expected = Some(csv);
                }
                assert_eq!(shared.selected_lap, Some(1));
            }
        }
    }
    #[test]
    fn external_results_deltas_slot_statistics_and_time_share_cache() {
        let shared = state();
        let mut panel = ReportPanel::new(10, "Comparison");
        panel.settings.statistics = ReportStatistic::ALL.into();
        let main = info();
        let mut other = info();
        other.data = Arc::from(vec![5., 15., 25., 35.]);
        other.display_scale = 2.;
        other.display_offset = 10.;
        panel.external_sources = vec![vec![other.clone()]];
        let mut groups = report_groups(&panel.settings, &shared).0;
        let mut external = groups[0].clone();
        external.id = "external".into();
        external.source = Some(0);
        groups.push(external.clone());
        panel.reference_groups = [vec![groups[0].clone()], vec![external]];
        panel.rebuild(std::slice::from_ref(&main), groups.clone());
        assert_eq!(panel.value(0, 3, ReportStatistic::Average), Some(50.));
        panel.settings.reference = ReportReference::Main;
        panel.rebuild(std::slice::from_ref(&main), groups.clone());
        assert_eq!(panel.value(0, 3, ReportStatistic::Average), Some(35.));
        panel.settings.reference = ReportReference::Overlay0;
        panel.rebuild(std::slice::from_ref(&main), groups.clone());
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), Some(-35.));
        panel.settings.channel_statistics = vec![Some(vec![ReportStatistic::Change])];
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), None);
        assert_eq!(panel.value(0, 0, ReportStatistic::Change), Some(-30.));
        panel.settings.channel_statistics.clear();
        panel.settings.kind = ReportKind::Time;
        panel.settings.reference = ReportReference::Absolute;
        panel.rebuild(&[main], groups);
        assert_eq!(panel.value(0, 1, ReportStatistic::Average), Some(1.25));
        assert_eq!(panel.value(0, 2, ReportStatistic::Average), Some(2.75));
    }
    #[test]
    fn loaded_outings_resolve_their_own_channels_and_missing_slots_stay_blank() {
        use super::super::graph::{LapOverlay, OverlaySession, OverlaySource};
        let ld = Arc::new(
            i3rs_core::LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        );
        let mut shared = state();
        shared.ld_file = Some(ld.clone());
        let mut definition =
            shared.create_math_channel_def("Independent".into(), "2".into(), "km/h".into(), 1);
        definition.freq = 1.0;
        definition.data = Some(Arc::from(vec![999.; 4]));
        shared.math_channels.push(definition);
        shared.comparisons.main_lap = Some(1);
        shared.selected_lap = Some(1);
        shared.comparisons.sessions.push(OverlaySession {
            settings_key: String::new(),
            requests: Default::default(),
            failures: Default::default(),
            path: "other.ld".into(),
            file_name: "Other outing".into(),
            ld_file: ld,
            laps: shared.laps.clone(),
            distance_axis_cache: None,
            channel_cache: Default::default(),
        });
        shared.comparisons.overlays.push(LapOverlay {
            alignment_valid: true,
            gps_align: false,
            visible: true,
            color: None,
            source: OverlaySource::External(0),
            lap_idx: 1,
            manual_offset: 0.,
            stretch_to_reference: false,
        });
        let mut panel = ReportPanel::new(11, "Outings");
        panel.add_channel(ChannelId::Math(0), &shared);
        panel.settings.scope = ReportScope::AllLoaded;
        let registry = vec![build_plotted_channel_info(&panel.channels[0], &shared)];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut groups = report_groups(&panel.settings, &shared).0;
            panel.prepare_populations(&mut shared, &registry, &mut groups);
            panel.rebuild(&registry, groups);
            if panel.external_sources[0][0].freq > 0.0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "external worker failed"
            );
            std::thread::yield_now();
        }
        assert_eq!(panel.groups.len(), 6);
        assert_eq!(panel.results[0][0].average, Some(999.));
        assert_eq!(panel.results[0][3].average, Some(2.));
        panel.settings.reference = ReportReference::Overlay0;
        panel.rebuild(&registry, panel.groups.clone());
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), Some(997.));
        panel.settings.scope = ReportScope::SelectedOutings;
        let mut groups = report_groups(&panel.settings, &shared).0;
        panel.prepare_populations(&mut shared, &registry, &mut groups);
        assert_eq!(groups.len(), 2);
        panel.settings.scope = ReportScope::SelectedRanges;
        let mut groups = report_groups(&panel.settings, &shared).0;
        panel.prepare_populations(&mut shared, &registry, &mut groups);
        assert_eq!(groups.len(), 2);
        panel.external_sources[0][0].freq = 0.0;
        panel.external_sources[0][0].data = Arc::from([]);
        panel.rebuild(&registry, groups);
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), None);
    }
    #[test]
    fn acceleration_views_use_the_same_si_events_and_persist_settings() {
        let mut shared = state();
        shared.select_lap(Some(1));
        let mut definition =
            shared.create_math_channel_def("Speed".into(), "0".into(), "km/h".into(), 1);
        definition.freq = 1.0;
        definition.data = Some(Arc::from(vec![0., 36., 72., 108., 72., 36., 0.]));
        shared.math_channels.push(definition);
        let mut panel = ReportPanel::new(12, "Acceleration");
        panel.add_channel(ChannelId::Math(0), &shared);
        panel.settings.kind = ReportKind::Acceleration;
        panel.settings.acceleration.points_mps = vec![5., 15., 25.];
        let ctx = egui::Context::default();
        let mut expected = None;
        for view in [ReportView::Table, ReportView::Chart, ReportView::Split] {
            panel.settings.view = view;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600., 1000.),
                    )),
                    ..Default::default()
                },
                |ui| panel.ui(ui, &mut shared),
            );
            output.textures_delta.clear();
            let csv = panel.acceleration_csv();
            if let Some(expected) = &expected {
                assert_eq!(expected, &csv);
            } else {
                expected = Some(csv);
            }
            assert_eq!(panel.acceleration_result.events.len(), 5);
            assert_eq!(panel.acceleration_result.events[0].distance_metres, 10.);
        }
        let settings: ReportSettings =
            serde_json::from_str(&serde_json::to_string(&panel.settings).unwrap()).unwrap();
        assert_eq!(settings, panel.settings);
        assert_eq!(shared.selected_lap, Some(1));
    }
    #[test]
    fn time_report_uses_durations_without_publishing_a_speed_as_elapsed_time() {
        let mut shared = state();
        let mut panel = ReportPanel::new(13, "Time");
        panel.settings.kind = ReportKind::Time;
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| panel.ui(ui, &mut shared));
        output.textures_delta.clear();
        assert!(panel.channels.is_empty());
        assert!(shared.plotted_channel_registry.is_empty());
        assert_eq!(panel.value(0, 1, ReportStatistic::Average), Some(1.25));
        assert!(panel.csv().contains("Duration"));
        panel.settings.scope = ReportScope::SelectedRanges;
        shared.selected_lap = Some(0);
        shared.comparisons.main_lap = Some(1);
        assert_eq!(
            report_groups(&panel.settings, &shared).0[0].lap_index,
            Some(1)
        );
    }
    #[test]
    fn section_deltas_match_stable_section_identity_and_time_trust_masks() {
        let shared = state();
        let mut panel = ReportPanel::new(14, "Sections");
        panel.settings.kind = ReportKind::Time;
        panel.settings.sections_as_headings = true;
        panel.settings.reference = ReportReference::Main;
        let mut groups = report_groups(&ReportSettings::default(), &shared).0;
        groups[0].section = Some("first:shared-suffix".into());
        groups[0].windows = vec![(0., 1.)];
        groups[1].section = Some("second:shared-suffix".into());
        groups[1].windows = vec![(0., 2.)];
        let mut refs = groups.clone();
        refs[0].windows = vec![(0., 0.5)];
        refs[1].windows = vec![(0., 1.25)];
        panel.reference_groups[0] = refs;
        panel.rebuild(&[info()], groups);
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), Some(0.5));
        assert_eq!(panel.value(0, 1, ReportStatistic::Average), Some(0.75));
        let settings = ReportSettings {
            kind: ReportKind::Time,
            trusted_only: true,
            ..Default::default()
        };
        let rows = report_groups(&settings, &shared).0;
        panel.settings = settings;
        panel.rebuild(&[info()], rows);
        assert_eq!(panel.value(0, 0, ReportStatistic::Average), Some(2.75));
        assert_eq!(panel.value(0, 1, ReportStatistic::Average), None);
    }
}

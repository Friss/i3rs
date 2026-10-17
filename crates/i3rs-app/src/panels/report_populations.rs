use super::super::graph::{OverlaySource, resolve_recording_channel};
use super::*;

pub(super) struct SourceContext {
    key: String,
    state: SharedState,
    track: Option<std::sync::mpsc::Receiver<Option<i3rs_core::TrackData>>>,
}
impl SourceContext {
    pub(super) fn is_loading(&self) -> bool {
        self.track.is_some()
    }
}
impl ReportPanel {
    pub(super) fn summarize(
        &self,
        c: usize,
        g: &ReportGroup,
        registry: &[PlottedChannelInfo],
    ) -> ReportSummary {
        if self.settings.kind == ReportKind::Time {
            let duration: f64 = i3rs_core::report::report_windows(&g.windows)
                .iter()
                .map(|(a, b)| b - a)
                .sum();
            return if g.windows.is_empty() {
                ReportSummary::default()
            } else {
                ReportSummary {
                    sample_count: 1,
                    finite_count: 1,
                    native_count: 0,
                    min: Some(duration),
                    max: Some(duration),
                    average: Some(duration),
                    stddev: Some(0.),
                    start: Some(duration),
                    end: Some(duration),
                }
            };
        }
        let Some(info) = g.source.map_or_else(
            || registry.get(c),
            |s| self.external_sources.get(s).and_then(|r| r.get(c)),
        ) else {
            return ReportSummary::default();
        };
        report_statistics(
            &ReportSeries {
                data: &info.data,
                freq: info.freq,
                interpolation: info.interpolation,
                scale: info.display_scale,
                offset: info.display_offset,
            },
            &g.windows,
            self.settings.boundary,
        )
    }
    pub(super) fn value(&self, c: usize, g: usize, stat: ReportStatistic) -> Option<f64> {
        let enabled = self
            .settings
            .channel_statistics
            .get(c)
            .and_then(|v| v.as_ref())
            .unwrap_or(&self.settings.statistics);
        if self.settings.kind != ReportKind::Time && !enabled.contains(&stat) {
            return None;
        }
        let value = self.results.get(c)?.get(g)?.value(stat)?;
        let result = match self.settings.reference {
            ReportReference::Absolute => Some(value),
            ReportReference::Main => Some(value - self.reference_value(0, c, g, stat)?),
            ReportReference::Overlay0 => Some(value - self.reference_value(1, c, g, stat)?),
        };
        result.filter(|v| v.is_finite())
    }
    fn reference_value(&self, r: usize, c: usize, g: usize, stat: ReportStatistic) -> Option<f64> {
        // Section headings compare the corresponding section, never a whole-lap value.
        if self.settings.sections_as_headings {
            let section = self.groups[g].section.as_ref()?;
            let source = self.reference_groups[r]
                .iter()
                .find(|row| row.section.as_ref() == Some(section))?;
            return self.summarize(c, source, &self.sources).value(stat);
        }
        self.reference_results[r].get(c)?.value(stat)
    }
    pub(super) fn prepare_populations(
        &mut self,
        shared: &mut SharedState,
        registry: &[PlottedChannelInfo],
        groups: &mut Vec<ReportGroup>,
    ) {
        self.population_diagnostics.clear();
        let mut selected = self.settings.clone();
        selected.scope = ReportScope::SelectedRange;
        let previous = shared.selected_lap;
        shared.selected_lap = shared.comparisons.main_lap.or(previous);
        self.reference_groups[0] = cached_groups(&selected, shared, &mut self.population_groups);
        shared.selected_lap = previous;
        self.reference_groups[1].clear();
        let definition = shared.track_data_if_ready().and_then(|t| {
            i3rs_core::export_track_definition(&t, &shared.sectors, "Report sections".into())
        });
        let mut comparisons = std::mem::take(&mut shared.comparisons);
        self.external_sources.clear();
        self.population_contexts
            .truncate(comparisons.sessions.len());
        for (si, session) in comparisons.sessions.iter_mut().enumerate() {
            let key = format!(
                "{}:{:?}:{:?}",
                session.path.display(),
                shared.math_evaluation_key(0),
                (&shared.recording_analyses, &definition)
            );
            if self
                .population_contexts
                .get(si)
                .is_none_or(|c| c.key != key)
            {
                let mut state =
                    crate::comparison::recording_context(shared, session.ld_file.clone());
                state.laps = state
                    .current_analysis()
                    .map_or_else(|| session.laps.clone(), |a| a.laps());
                state.pending_track_definition = definition.clone();
                let entry = SourceContext {
                    key,
                    state,
                    track: None,
                };
                if si < self.population_contexts.len() {
                    self.population_contexts[si] = entry;
                } else {
                    self.population_contexts.push(entry);
                }
            }
            let context = &mut self.population_contexts[si];
            if (self.settings.section_id.is_some() || self.settings.sections_as_headings)
                && context.track.is_none()
                && !context.state.corrections_settled()
            {
                let ld = session.ld_file.clone();
                let corrections = context.state.current_corrections();
                let laps = context.state.competition_laps();
                let (tx, rx) = std::sync::mpsc::channel();
                context.track = Some(rx);
                let work = move || {
                    let track =
                        i3rs_core::corrections::build_corrected_recording(&ld, &corrections, &laps)
                            .ok()
                            .and_then(|r| r.track);
                    let _ = tx.send(track);
                };
                #[cfg(not(target_arch = "wasm32"))]
                std::thread::spawn(work);
                #[cfg(target_arch = "wasm32")]
                work();
            }
            if let Some(rx) = &context.track {
                match rx.try_recv() {
                    Ok(track) => {
                        context.state.store_track_data(track);
                        context.track = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        context.state.store_track_data(None);
                        context.track = None;
                    }
                    _ => {}
                }
            }
            let mut infos = Vec::new();
            for (c, main) in registry.iter().enumerate() {
                let mut info = main.clone();
                if self.settings.kind == ReportKind::Time {
                    infos.push(info);
                    continue;
                }
                if let Some(slot) = self
                    .channels
                    .get(c)
                    .and_then(|pc| crate::workspace::save_analysis_channel(pc, shared))
                {
                    // Resolve in the source unit; retain this report's display transform exactly once.
                    let target = slot.source_unit.as_deref().unwrap_or(&main.unit);
                    if let Some((data, freq)) =
                        resolve_recording_channel(session, shared, &slot.name, target)
                    {
                        info.data = data;
                        info.freq = freq;
                        info.interpolation =
                            crate::comparison::reference_interpolation(&context.state, &slot.name);
                    } else {
                        info.data = std::sync::Arc::from([]);
                        info.freq = 0;
                    }
                } else {
                    info.data = std::sync::Arc::from([]);
                    info.freq = 0;
                }
                infos.push(info);
            }
            self.external_sources.push(infos);
            let include = match self.settings.scope {
                ReportScope::AllLoaded => true,
                ReportScope::SelectedOutings | ReportScope::SelectedRanges => comparisons
                    .overlays
                    .iter()
                    .any(|o| o.visible && matches!(o.source,OverlaySource::External(i) if i==si)),
                _ => false,
            };
            if include {
                if (self.settings.sections_as_headings && context.state.sectors.is_empty())
                    || self.settings.section_id.as_ref().is_some_and(|id| {
                        !context.state.sectors.iter().any(|s| &s.semantics.id == id)
                    })
                {
                    self.population_diagnostics.push(format!(
                        "{}: selected sections are unavailable or loading",
                        session.file_name
                    ));
                }
                if self.settings.scope == ReportScope::SelectedRanges {
                    for overlay in comparisons.overlays.iter().filter(|o| {
                        o.visible && matches!(o.source,OverlaySource::External(i) if i==si)
                    }) {
                        context.state.selected_lap = Some(overlay.lap_idx);
                        let (mut rows, _) = (
                            cached_groups(&selected, &context.state, &mut self.population_groups),
                            None::<String>,
                        );
                        tag(&mut rows, si, &session.file_name);
                        groups.extend(rows);
                    }
                } else {
                    let (mut rows, _) = (
                        cached_groups(&self.settings, &context.state, &mut self.population_groups),
                        None::<String>,
                    );
                    tag(&mut rows, si, &session.file_name);
                    groups.extend(rows);
                }
            }
        }
        if let Some(overlay) = comparisons.overlays.first() {
            match overlay.source {
                OverlaySource::MainSession => {
                    let old = shared.selected_lap;
                    shared.selected_lap = Some(overlay.lap_idx);
                    self.reference_groups[1] =
                        cached_groups(&selected, shared, &mut self.population_groups);
                    shared.selected_lap = old;
                }
                OverlaySource::External(si) => {
                    if let Some(context) = self.population_contexts.get_mut(si) {
                        context.state.selected_lap = Some(overlay.lap_idx);
                        let (mut rows, _) = (
                            cached_groups(&selected, &context.state, &mut self.population_groups),
                            None::<String>,
                        );
                        tag(&mut rows, si, &comparisons.sessions[si].file_name);
                        self.reference_groups[1] = rows;
                    }
                }
            }
        }
        if self.settings.scope == ReportScope::SelectedRanges {
            for overlay in comparisons
                .overlays
                .iter()
                .filter(|o| o.visible && matches!(o.source, OverlaySource::MainSession))
            {
                let old = shared.selected_lap;
                shared.selected_lap = Some(overlay.lap_idx);
                let (mut rows, _) = report_groups(&selected, shared);
                shared.selected_lap = old;
                for row in &mut rows {
                    row.label = format!("Overlay · {}", row.label);
                    row.id = format!("overlay:{}", row.id);
                }
                groups.extend(rows);
            }
        }
        shared.comparisons = comparisons;
    }
}
fn tag(rows: &mut [ReportGroup], source: usize, name: &str) {
    for row in rows {
        row.source = Some(source);
        row.id = format!("source:{source}:{}", row.id);
        row.label = format!("{name} · {}", row.label);
    }
}

fn cached_groups(
    settings: &ReportSettings,
    shared: &SharedState,
    cache: &mut std::collections::HashMap<String, Vec<ReportGroup>>,
) -> Vec<ReportGroup> {
    let key = format!(
        "{:?}:{:?}:{:?}:{:?}:{:?}:{:?}",
        (
            settings.scope,
            settings.trusted_only,
            &settings.section_id,
            settings.sections_as_headings,
            settings.reverse
        ),
        (
            shared.recording_key(),
            shared.session_id,
            shared.selected_lap,
            shared.analysis_range(),
            shared.data_duration
        ),
        shared.laps,
        shared.current_analysis().map(|a| &a.ranges),
        shared.sectors,
        shared
            .track_data_if_ready()
            .map(|t| std::sync::Arc::as_ptr(&t) as usize)
    );
    if let Some(rows) = cache.get(&key) {
        return rows.clone();
    }
    let rows = report_groups(settings, shared).0;
    if cache.len() > 32 {
        cache.clear();
    }
    cache.insert(key, rows.clone());
    rows
}

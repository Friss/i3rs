//! Shared comparison ownership and recording-specific, asynchronous resolution.
use crate::panels::graph::{LapOverlay, OverlaySession};
use crate::state::{MathChannelDef, SharedState};
use i3rs_core::{ChannelData, ChannelKey, ChannelResolution, EvaluationOptions};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    #[default]
    Linked,
    Overview,
    Independent,
    FollowCursor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphView {
    #[serde(skip)]
    pub last_mode: Option<ViewMode>,
    #[serde(skip)]
    pub linked_window: Option<(f64, f64)>,
    pub show_time_variance: bool,
    pub saved_range: Option<(f64, f64)>,
    pub show_annotations: bool,
    pub show_events: bool,
    pub event_channel: String,
    pub error_channel: String,
    pub mode: ViewMode,
    pub local_cursor: bool,
    pub cursor: Option<f64>,
    pub local_range: Option<(f64, f64)>,
    pub detail_seconds: f64,
}
impl Default for GraphView {
    fn default() -> Self {
        Self {
            last_mode: None,
            linked_window: None,
            show_time_variance: false,
            saved_range: None,
            show_annotations: true,
            show_events: true,
            event_channel: String::new(),
            error_channel: String::new(),
            mode: ViewMode::Linked,
            local_cursor: false,
            cursor: None,
            local_range: None,
            detail_seconds: 10.,
        }
    }
}
impl GraphView {
    pub fn range(
        &self,
        linked: Option<(f64, f64)>,
        cursor: Option<f64>,
        duration: f64,
    ) -> Option<(f64, f64)> {
        match self.mode {
            ViewMode::Linked => linked,
            ViewMode::Overview => Some((0., duration)),
            ViewMode::Independent => self.local_range.or(linked),
            ViewMode::FollowCursor => {
                let width = self.detail_seconds.clamp(0.1, duration.max(0.1));
                let start =
                    (cursor.unwrap_or(0.) - width / 2.).clamp(0., (duration - width).max(0.));
                Some((start, (start + width).min(duration)))
            }
        }
    }
}

#[derive(Default)]
pub struct Comparisons {
    pub pending_swap: Option<usize>,
    pub unresolved_overlays: Vec<crate::workspace::GraphOverlayConfig>,
    pub main_lap: Option<usize>,
    pub overlays: Vec<LapOverlay>,
    pub sessions: Vec<OverlaySession>,
}

pub type ResolvedSeries = Result<(Arc<[f64]>, u16), String>;

/// Copy definitions and settings, never cached results from another recording.
pub fn recording_context(shared: &SharedState, ld: Arc<i3rs_core::LdFile>) -> SharedState {
    let mut context = SharedState::new();
    context.data_duration = Some(ld.duration_secs());
    context.ld_file = Some(ld);
    context.channel_settings = shared.channel_settings.clone();
    context.channel_aliases = shared.channel_aliases.clone();
    context.recording_analyses = shared.recording_analyses.clone();
    context.math_channels = shared
        .math_channels
        .iter()
        .map(|mc| {
            let mut def = MathChannelDef::new(
                mc.id,
                mc.name.clone(),
                mc.expression.clone(),
                mc.unit.clone(),
                mc.dec_places,
            );
            def.options = mc.options.clone();
            def
        })
        .collect();
    context
}

pub fn evaluate_channel(
    context: &SharedState,
    reference: &str,
) -> Result<(Vec<f64>, u16, String), String> {
    fn resolve(
        context: &SharedState,
        reference: &str,
        visiting: &mut HashSet<u64>,
        cache: &mut HashMap<String, (Vec<f64>, u16, String)>,
    ) -> Result<(Vec<f64>, u16, String), String> {
        if let Some(result) = cache.get(reference) {
            return Ok(result.clone());
        }
        let result = match context.resolve_analysis_reference(reference) {
            ChannelResolution::Unresolved { reason, .. } => return Err(reason),
            ChannelResolution::Resolved { key, .. } => match key {
                ChannelKey::Physical(idx) => {
                    let ld = context.ld_file.as_ref().ok_or("No recording")?;
                    let ch = &ld.channels[idx];
                    let data = ld
                        .read_channel_data(ch)
                        .ok_or_else(|| format!("Cannot decode {}", ch.name))?;
                    let unit = context
                        .channel_settings
                        .mapping(&ch.name, context.recording_key())
                        .and_then(|m| m.unit.clone())
                        .unwrap_or_else(|| ch.unit.clone());
                    (data, ch.freq, unit)
                }
                ChannelKey::Constant(idx) => {
                    let c = &context.channel_settings.constants[idx];
                    (
                        vec![c.value; context.data_duration.unwrap_or(0.).ceil() as usize],
                        1,
                        c.unit.clone(),
                    )
                }
                ChannelKey::Generated(name) => {
                    let ld = context.ld_file.as_ref().ok_or("No recording")?;
                    let laps = context
                        .current_analysis()
                        .map(|a| a.laps())
                        .unwrap_or_else(|| i3rs_core::detect_laps(ld));
                    let corrected = i3rs_core::corrections::build_corrected_recording(
                        ld,
                        &context.current_corrections(),
                        &laps,
                    )?;
                    let source = match name.as_str() {
                        "Corr Speed" => corrected.speed,
                        "Corr GPS Speed" => corrected.gps_speed,
                        _ => corrected.distance,
                    }
                    .ok_or_else(|| format!("{name}: no usable source"))?;
                    (
                        source.0,
                        source.1,
                        if name.contains("Speed") { "m/s" } else { "m" }.into(),
                    )
                }
                ChannelKey::Derived(id) => {
                    if !visiting.insert(id) {
                        return Err(format!("Cyclic dependency: {reference}"));
                    }
                    let mc = &context.math_channels[context
                        .math_channel_index_by_id(id)
                        .ok_or("Missing definition")?];
                    let ast =
                        i3rs_core::parse_expression(&mc.expression).map_err(|e| e.to_string())?;
                    let mut inputs = HashMap::new();
                    let mut options = EvaluationOptions {
                        output_freq: mc.options.output_freq,
                        interpolation: mc.options.interpolation,
                        duration: context.data_duration,
                        ..Default::default()
                    };
                    for name in i3rs_core::referenced_channels(&ast) {
                        let (samples, freq, unit) = resolve(context, &name, visiting, cache)?;
                        options.input_units.insert(name.clone(), unit);
                        let policy = match context.resolve_analysis_reference(&name) {
                            ChannelResolution::Resolved {
                                key: ChannelKey::Physical(idx),
                                ..
                            } => {
                                let ch = &context.ld_file.as_ref().unwrap().channels[idx];
                                context
                                    .channel_settings
                                    .mapping(&ch.name, context.recording_key())
                                    .map(|m| m.interpolation)
                                    .unwrap_or(
                                        if super::panels::utils::uses_discrete_values(
                                            &ch.name,
                                            &ch.enum_labels,
                                        ) {
                                            i3rs_core::Interpolation::Previous
                                        } else {
                                            i3rs_core::Interpolation::Linear
                                        },
                                    )
                            }
                            ChannelResolution::Resolved {
                                key: ChannelKey::Derived(id),
                                ..
                            } => {
                                let def = &context.math_channels
                                    [context.math_channel_index_by_id(id).unwrap()];
                                if !def.options.enum_labels.is_empty()
                                    && def.options.interpolation == i3rs_core::Interpolation::Linear
                                {
                                    i3rs_core::Interpolation::Previous
                                } else {
                                    def.options.interpolation
                                }
                            }
                            _ => i3rs_core::Interpolation::Linear,
                        };
                        options.input_interpolation.insert(name.clone(), policy);
                        inputs.insert(name, ChannelData { samples, freq });
                    }
                    let (data, freq) = i3rs_core::evaluate_expression_with_options(
                        &mc.expression,
                        &inputs,
                        &HashMap::new(),
                        &options,
                    )
                    .map_err(|e| e.to_string())?;
                    visiting.remove(&id);
                    (data, freq, mc.unit.clone())
                }
            },
        };
        cache.insert(reference.into(), result.clone());
        Ok(result)
    }
    resolve(context, reference, &mut HashSet::new(), &mut HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_overlay_evaluates_its_own_inputs_and_aliases() {
        let ld = Arc::new(
            i3rs_core::LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        );
        let channel = ld.find_channel_by_name("Engine Speed").unwrap();
        let expected = ld.read_channel_data(channel).unwrap();
        let mut main = SharedState::new();
        main.channel_settings.aliases.insert(
            "RPM alias".into(),
            vec!["Missing RPM".into(), "Engine Speed".into()],
        );
        let mut def = MathChannelDef::new(
            7,
            "Double RPM".into(),
            "'RPM alias' * 2".into(),
            "rpm".into(),
            0,
        );
        def.data = Some(Arc::from(vec![999.; 10]));
        main.math_channels.push(def);
        let context = recording_context(&main, ld.clone());
        let (actual, freq, _) = evaluate_channel(&context, "User::Double RPM").unwrap();
        assert_eq!(freq, channel.freq);
        assert_eq!(actual.len(), expected.len());
        assert_eq!(actual[10], expected[10] * 2.);
        let (again, _, _) = evaluate_channel(&context, "User::Double RPM").unwrap();
        assert_eq!(actual, again);
        main.math_channels[0].options.recording = Some("another-recording".into());
        assert!(evaluate_channel(&recording_context(&main, ld), "User::Double RPM").is_err());
    }
    #[test]
    fn view_modes_preserve_their_range_contract() {
        let mut view = GraphView::default();
        assert_eq!(
            view.range(Some((20., 30.)), Some(50.), 100.),
            Some((20., 30.))
        );
        view.mode = ViewMode::Overview;
        assert_eq!(
            view.range(Some((20., 30.)), Some(50.), 100.),
            Some((0., 100.))
        );
        view.mode = ViewMode::FollowCursor;
        assert_eq!(view.range(None, Some(99.), 100.), Some((90., 100.)));
        view.mode = ViewMode::Independent;
        view.local_range = Some((12., 17.));
        assert_eq!(view.range(Some((20., 30.)), None, 100.), Some((12., 17.)));
        let restored: GraphView =
            serde_json::from_str(&serde_json::to_string(&view).unwrap()).unwrap();
        assert_eq!(restored.local_range, view.local_range);
        assert_eq!(restored.mode, view.mode);
    }
}

/// A comparison population stays on its source recording's native sample grid.
pub struct OverlayPopulation {
    pub recording: String,
    pub label: String,
    pub color: eframe::egui::Color32,
    pub range: (f64, f64),
    pub channels: Vec<crate::state::PlottedChannelInfo>,
    pub filter: crate::panels::distribution::PreparedFilter,
}
pub fn overlay_populations(
    shared: &mut SharedState,
    slots: &[&crate::state::PlottedChannel],
    filter: &crate::panels::distribution::ComponentFilter,
) -> Vec<OverlayPopulation> {
    let range = shared.analysis_range();
    overlay_populations_in_range(shared, slots, filter, range)
}
pub fn overlay_populations_in_range(
    shared: &mut SharedState,
    slots: &[&crate::state::PlottedChannel],
    filter: &crate::panels::distribution::ComponentFilter,
    main_range: (f64, f64),
) -> Vec<OverlayPopulation> {
    use crate::panels::{
        graph::{OverlaySource, resolve_recording_channel},
        utils::*,
    };
    let mut comparisons = std::mem::take(&mut shared.comparisons);
    let mut output = Vec::new();
    shared.request_track_data_build();

    let main_lap = comparisons
        .main_lap
        .and_then(|i| shared.laps.get(i))
        .cloned();
    let Some(main_lap) = main_lap else {
        shared.comparisons = comparisons;
        return output;
    };
    for overlay in &mut comparisons.overlays {
        if overlay.gps_align {
            match crate::panels::graph::gps_alignment_offset(
                shared,
                &mut comparisons.sessions,
                overlay,
                comparisons.main_lap,
            ) {
                Ok(Some(offset)) => {
                    overlay.manual_offset = offset;
                    overlay.alignment_valid = true;
                }
                Ok(None) => overlay.alignment_valid = false,
                Err(error) => {
                    overlay.alignment_valid = false;
                    shared.message(error);
                }
            }
        }
    }
    for (index, overlay) in comparisons
        .overlays
        .iter()
        .enumerate()
        .filter(|(_, o)| o.visible && (!o.gps_align || o.alignment_valid))
    {
        if matches!(overlay.source,OverlaySource::External(i) if i>=comparisons.sessions.len()) {
            continue;
        }
        let context = match overlay.source {
            OverlaySource::MainSession => None,
            OverlaySource::External(i) => comparisons
                .sessions
                .get(i)
                .map(|s| recording_context(shared, s.ld_file.clone())),
        };
        let source = context.as_ref().unwrap_or(shared);
        let lap = match overlay.source {
            OverlaySource::MainSession => shared.laps.get(overlay.lap_idx).cloned(),
            OverlaySource::External(i) => {
                let session = &mut comparisons.sessions[i];
                if let Some(a) = source.current_analysis() {
                    session.laps = a.laps();
                }
                session.laps.get(overlay.lap_idx).cloned()
            }
        };
        let Some(lap) = lap else {
            continue;
        };
        let mut channels = Vec::new();
        let mut ready = true;
        for pc in slots {
            let mut info = build_plotted_channel_info(pc, shared);
            if let OverlaySource::External(i) = overlay.source {
                let reference = pc
                    .requested_reference
                    .as_ref()
                    .map(|r| r.0.clone())
                    .unwrap_or_else(|| match pc.channel_id {
                        crate::state::ChannelId::Math(i) => format!(
                            "{}::{}",
                            shared.math_channels[i].options.source, shared.math_channels[i].name
                        ),
                        _ => resolve_channel_meta(pc.channel_id, shared).0,
                    });
                let unit = resolve_channel_meta(pc.channel_id, shared).1;
                if let Some((data, hz)) = resolve_recording_channel(
                    &mut comparisons.sessions[i],
                    shared,
                    &reference,
                    &unit,
                ) {
                    info.data = data;
                    info.freq = hz;
                    info.interpolation = reference_interpolation(source, &reference);
                } else {
                    ready = false;
                }
            }
            channels.push(info);
        }
        let gate_reference = filter.gate.trim();
        let mut gate = None;
        let mut constant = None;
        if !gate_reference.is_empty() {
            if let ChannelResolution::Resolved { key, .. } =
                source.resolve_analysis_reference(gate_reference)
            {
                let unit = match key {
                    ChannelKey::Constant(i) => {
                        constant = Some(source.channel_settings.constants[i].value);
                        source.channel_settings.constants[i].unit.clone()
                    }
                    ChannelKey::Physical(i) => {
                        let ch = &source.ld_file.as_ref().unwrap().channels[i];
                        source
                            .channel_settings
                            .mapping(&ch.name, source.recording_key())
                            .and_then(|m| m.unit.clone())
                            .unwrap_or_else(|| ch.unit.clone())
                    }
                    ChannelKey::Derived(id) => source.math_channels
                        [source.math_channel_index_by_id(id).unwrap()]
                    .unit
                    .clone(),
                    ChannelKey::Generated(name) => {
                        if name.contains("Speed") { "m/s" } else { "m" }.into()
                    }
                };
                if constant.is_none() {
                    let result = match overlay.source {
                        OverlaySource::External(i) => resolve_recording_channel(
                            &mut comparisons.sessions[i],
                            shared,
                            gate_reference,
                            &unit,
                        ),
                        OverlaySource::MainSession => source
                            .resolve_slot(gate_reference, false)
                            .and_then(|id| create_plotted_channel(id, source, 0))
                            .map(|pc| {
                                let i = build_plotted_channel_info(&pc, source);
                                (i.data, i.freq)
                            }),
                    };
                    if let Some((data, freq)) = result {
                        gate = Some(crate::state::PlottedChannelInfo {
                            channel_id: crate::state::ChannelId::Physical(usize::MAX),
                            name: gate_reference.into(),
                            unit,
                            freq,
                            dec_places: 3,
                            color: eframe::egui::Color32::GRAY,
                            data,
                            display_scale: 1.,
                            display_offset: 0.,
                            interpolation: reference_interpolation(source, gate_reference),
                            enum_labels: Default::default(),
                        });
                    } else {
                        ready = false;
                    }
                }
            } else {
                ready = false;
            }
        }
        let trusted_ranges = filter.trusted_only.then(|| {
            source
                .current_analysis()
                .into_iter()
                .flat_map(|a| &a.ranges)
                .filter(|r| r.trust == i3rs_core::ranges::Trust::Trusted)
                .map(|r| (r.start, r.end))
                .collect()
        });
        // A source distance axis gives the same correspondence used by graph overlays.
        let source_axis = match overlay.source {
            OverlaySource::MainSession => shared
                .distance_axis_cache
                .as_ref()
                .map(|a| (a.data.clone(), a.freq)),
            OverlaySource::External(i) => resolve_recording_channel(
                &mut comparisons.sessions[i],
                shared,
                "MoTeC::Corr Dist",
                "m",
            ),
        };
        let has_distance = shared.distance_axis_cache.is_some() && source_axis.is_some();
        if let OverlaySource::External(i) = overlay.source
            && comparisons.sessions[i]
                .requests
                .contains_key("MoTeC::Corr Dist:m")
            && shared.distance_axis_cache.is_some()
        {
            continue;
        }
        let range =
            if let (Some(main), Some((data, hz))) = (&shared.distance_axis_cache, source_axis) {
                mapped_distance_range(
                    (&main.data, main.freq),
                    (&data, hz),
                    &main_lap,
                    &lap,
                    main_range,
                    overlay.manual_offset,
                    overlay.stretch_to_reference,
                )
            } else {
                mapped_time_range(
                    &main_lap,
                    &lap,
                    main_range,
                    overlay.manual_offset,
                    overlay.stretch_to_reference,
                )
            };
        if !ready {
            continue;
        }
        let Some(range) = range else {
            continue;
        };
        let label = match overlay.source {
            OverlaySource::MainSession => lap.name.clone(),
            OverlaySource::External(i) => {
                format!("{} · {}", comparisons.sessions[i].file_name, lap.name)
            }
        };
        let label = format!(
            "{} [{}]",
            label,
            if has_distance {
                "distance"
            } else {
                "elapsed time"
            }
        );
        output.push(OverlayPopulation {
            recording: source.recording_key().into(),
            label,
            color: overlay.color.unwrap_or(
                crate::state::CHANNEL_COLORS[(index + 1) % crate::state::CHANNEL_COLORS.len()],
            ),
            range,
            channels,
            filter: crate::panels::distribution::PreparedFilter {
                constant,
                trusted_ranges,
                gate,
                available: true,
                key: String::new(),
            },
        });
    }
    shared.comparisons = comparisons;
    output
}
fn mapped_time_range(
    main: &i3rs_core::Lap,
    source: &i3rs_core::Lap,
    range: (f64, f64),
    offset: f64,
    stretch: bool,
) -> Option<(f64, f64)> {
    let scale = if stretch {
        main.duration() / source.duration()
    } else {
        1.
    };
    let a =
        (source.start_time + (range.0 - main.start_time - offset) / scale).max(source.start_time);
    let b = (source.start_time + (range.1 - main.start_time - offset) / scale).min(source.end_time);
    (a.is_finite() && b.is_finite() && b > a).then_some((a, b))
}
fn mapped_distance_range(
    main: (&[f64], u16),
    source: (&[f64], u16),
    ml: &i3rs_core::Lap,
    sl: &i3rs_core::Lap,
    range: (f64, f64),
    offset: f64,
    stretch: bool,
) -> Option<(f64, f64)> {
    let sample = |axis: (&[f64], u16), t| i3rs_core::sample_at_time(axis.0, axis.1, t);
    let m0 = sample(main, ml.start_time)?;
    let s0 = sample(source, sl.start_time)?;
    let scale = if stretch {
        (sample(main, ml.end_time)? - m0) / (sample(source, sl.end_time)? - s0)
    } else {
        1.
    };
    let shift = sample(
        source,
        (sl.start_time + offset).clamp(0., source.0.len() as f64 / source.1.max(1) as f64),
    )? - s0;
    let inverse = |distance: f64| {
        let w =
            i3rs_core::sample_window_for_time(source.1, source.0.len(), sl.start_time, sl.end_time);
        let values = &source.0[w.start_sample..w.end_sample];
        if values.is_empty()
            || values.iter().any(|v| !v.is_finite())
            || values.windows(2).any(|p| p[1] < p[0])
        {
            return None;
        }
        let i = values.partition_point(|v| *v < distance);
        if i == 0 {
            return Some(sl.start_time);
        }
        if i >= values.len() {
            return Some(sl.end_time);
        }
        let fraction = (distance - values[i - 1]) / (values[i] - values[i - 1]).max(f64::EPSILON);
        Some((w.start_sample + i - 1) as f64 / source.1 as f64 + fraction / source.1 as f64)
    };
    let a = inverse((sample(main, range.0)? - m0 - shift) / scale + s0)?;
    let b = inverse((sample(main, range.1)? - m0 - shift) / scale + s0)?;
    (b > a).then_some((a, b))
}

/// Rebase portable role selections before loading the new Main recording.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn swap_external_main(
    config: &mut crate::workspace::ComparisonConfig,
    current_path: &str,
    index: usize,
) -> Result<String, String> {
    let selected = config
        .overlays
        .get(index)
        .ok_or("Overlay selection no longer exists")?
        .clone();
    let target = selected
        .file_path
        .clone()
        .ok_or("Selected overlay is in the Main recording")?;
    let old_main = config.main_lap.ok_or("No Main range selected")?;
    for overlay in &mut config.overlays {
        overlay.manual_offset -= selected.manual_offset;
        if overlay.file_path.as_deref() == Some(&target) {
            overlay.file_path = None;
        } else if overlay.file_path.is_none() {
            overlay.file_path = Some(current_path.into());
        }
    }
    config.overlays[index] = crate::workspace::GraphOverlayConfig {
        gps_align: selected.gps_align,
        visible: true,
        color: selected.color,
        file_path: Some(current_path.into()),
        lap_index: old_main,
        manual_offset: -selected.manual_offset,
        stretch_to_reference: selected.stretch_to_reference,
    };
    config.main_lap = Some(selected.lap_index);
    Ok(target)
}

fn reference_interpolation(source: &SharedState, reference: &str) -> i3rs_core::Interpolation {
    use i3rs_core::Interpolation;
    match source.resolve_analysis_reference(reference) {
        ChannelResolution::Resolved {
            key: ChannelKey::Physical(i),
            ..
        } => {
            let ch = &source.ld_file.as_ref().unwrap().channels[i];
            source
                .channel_settings
                .mapping(&ch.name, source.recording_key())
                .map(|m| m.interpolation)
                .unwrap_or_else(|| {
                    if crate::panels::utils::uses_discrete_values(&ch.name, &ch.enum_labels) {
                        Interpolation::Previous
                    } else {
                        Interpolation::Linear
                    }
                })
        }
        ChannelResolution::Resolved {
            key: ChannelKey::Derived(id),
            ..
        } => {
            let m = &source.math_channels[source.math_channel_index_by_id(id).unwrap()];
            if !m.options.enum_labels.is_empty() && m.options.interpolation == Interpolation::Linear
            {
                Interpolation::Previous
            } else {
                m.options.interpolation
            }
        }
        _ => Interpolation::Linear,
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;
    use crate::workspace::{ComparisonConfig, GraphOverlayConfig};
    fn lap(a: f64, b: f64) -> i3rs_core::Lap {
        i3rs_core::Lap {
            number: 1,
            name: "Range".into(),
            start_time: a,
            end_time: b,
        }
    }
    fn overlay(path: Option<&str>, lap: usize, offset: f64) -> GraphOverlayConfig {
        GraphOverlayConfig {
            gps_align: false,
            visible: true,
            color: Some([10, 20, 30]),
            file_path: path.map(str::to_owned),
            lap_index: lap,
            manual_offset: offset,
            stretch_to_reference: false,
        }
    }
    #[test]
    fn external_role_swap_rebases_paths_offsets_and_survives_json() {
        let mut config = ComparisonConfig {
            main_lap: Some(1),
            overlays: vec![
                overlay(Some("other.ld"), 3, 0.25),
                overlay(None, 2, 0.5),
                overlay(Some("other.ld"), 4, -0.5),
            ],
        };
        assert_eq!(
            swap_external_main(&mut config, "main.ld", 0).unwrap(),
            "other.ld"
        );
        let config: ComparisonConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(config.main_lap, Some(3));
        assert_eq!(config.overlays[0].file_path.as_deref(), Some("main.ld"));
        assert_eq!(config.overlays[0].lap_index, 1);
        assert_eq!(config.overlays[0].manual_offset, -0.25);
        assert_eq!(config.overlays[1].file_path.as_deref(), Some("main.ld"));
        assert_eq!(config.overlays[1].manual_offset, 0.25);
        assert!(config.overlays[2].file_path.is_none());
        assert_eq!(config.overlays[2].manual_offset, -0.75);
    }
    #[test]
    fn range_mapping_preserves_distance_offset_and_stretch_with_no_extrapolation() {
        let main = lap(0., 10.);
        let source = lap(0., 20.);
        let md: Vec<_> = (0..=20).map(|i| i as f64 * 10.).collect();
        let sd: Vec<_> = (0..=30).map(|i| i as f64 * 5.).collect();
        assert_eq!(
            mapped_distance_range((&md, 1), (&sd, 1), &main, &source, (2., 8.), 0., false),
            Some((4., 16.))
        );
        assert_eq!(
            mapped_distance_range((&md, 1), (&sd, 1), &main, &source, (2., 8.), 2., false),
            Some((2., 14.))
        );
        assert_eq!(
            mapped_time_range(&main, &source, (2., 8.), 0., true),
            Some((4., 16.))
        );
        assert_eq!(
            mapped_time_range(&main, &source, (2., 8.), 100., false),
            None
        );
        let missing = [0., f64::NAN, 10.];
        assert!(
            mapped_distance_range(
                (&md, 1),
                (&missing, 1),
                &main,
                &lap(0., 2.),
                (0., 1.),
                0.,
                false
            )
            .is_none()
        );
    }
}

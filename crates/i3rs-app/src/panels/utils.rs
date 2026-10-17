//! Shared utilities used by multiple panel types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use eframe::egui;

use crate::state::{
    CHANNEL_COLORS, ChannelId, ChannelPreference, PlottedChannel, PlottedChannelInfo, SharedState,
    YAxis, channel_preference_key,
};

pub type ChannelDisplayMeta = (String, String, f64, i16, Arc<HashMap<i64, String>>);

static EMPTY_ENUM_LABELS: LazyLock<Arc<HashMap<i64, String>>> =
    LazyLock::new(|| Arc::new(HashMap::new()));
static EMPTY_SAMPLES: LazyLock<Arc<[f64]>> = LazyLock::new(|| Arc::from(Vec::<f64>::new()));

/// Discrete (enum/state) channels show labels and are never interpolated.
pub fn uses_discrete_values(name: &str, enum_labels: &HashMap<i64, String>) -> bool {
    !enum_labels.is_empty() || i3rs_core::is_state_channel(name)
}

/// Format a value using file-parsed enum labels, falling back to hardcoded labels.
pub fn format_enum_value(
    name: &str,
    enum_labels: &HashMap<i64, String>,
    value: f64,
) -> Option<String> {
    if !value.is_finite() {
        return Some("---".into());
    }
    if !enum_labels.is_empty() {
        let v = value.round() as i64;
        if let Some(label) = enum_labels.get(&v) {
            return Some(label.clone());
        }
    }
    i3rs_core::format_state_value(name, value)
}

#[derive(Clone, Copy)]
pub struct DisplayUnitPreset {
    pub label: &'static str,
    pub scale: f64,
    pub offset: f64,
    pub unit: &'static str,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DisplayTransformFingerprint {
    pub scale_bits: u64,
    pub offset_bits: u64,
    pub unit: Option<String>,
}

/// Resolve channel metadata (name, unit, freq, dec_places, enum_labels) from a ChannelId.
pub fn resolve_channel_display_meta(id: ChannelId, shared: &SharedState) -> ChannelDisplayMeta {
    match id {
        ChannelId::Analysis(index) => {
            let reference = shared
                .unresolved_references
                .borrow()
                .get(index)
                .map(|r| r.0.clone())
                .unwrap_or_default();
            let (name, unit) = match shared.resolve_analysis_reference(&reference) {
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Constant(i),
                    ..
                } => {
                    let c = &shared.channel_settings.constants[i];
                    (c.name.clone(), c.unit.clone())
                }
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Generated(name),
                    ..
                } => {
                    let unit = if name.contains("Speed") { "m/s" } else { "m" };
                    (name, unit.into())
                }
                _ => (format!("{reference} (unavailable)"), String::new()),
            };
            (
                name,
                unit,
                shared.analysis_series(index).map_or(0.0, |(_, hz)| hz),
                3,
                Arc::clone(&EMPTY_ENUM_LABELS),
            )
        }
        ChannelId::Unresolved(index) => {
            let refs = shared.unresolved_references.borrow();
            let name = refs
                .get(index)
                .map_or("Unavailable channel", |r| r.0.as_str());
            (
                format!("{name} (unavailable)"),
                String::new(),
                0.0,
                0,
                Arc::clone(&EMPTY_ENUM_LABELS),
            )
        }
        ChannelId::Physical(idx) => {
            if let Some(ld) = &shared.ld_file
                && let Some(ch) = ld.channels.get(idx)
            {
                let mapping = shared
                    .channel_settings
                    .mapping(&ch.name, shared.recording_key());
                let labels = mapping.filter(|m| !m.enum_labels.is_empty()).map_or_else(
                    || Arc::clone(&ch.enum_labels),
                    |m| Arc::new(m.enum_labels.iter().map(|(v, l)| (*v, l.clone())).collect()),
                );
                (
                    mapping
                        .filter(|m| !m.name.is_empty())
                        .map_or_else(|| ch.name.clone(), |m| m.name.clone()),
                    mapping
                        .and_then(|m| m.unit.clone())
                        .unwrap_or_else(|| ch.unit.clone()),
                    ch.freq,
                    mapping
                        .and_then(|m| m.precision)
                        .unwrap_or(ch.display_decimals as i16),
                    labels,
                )
            } else {
                (
                    "???".into(),
                    String::new(),
                    0.0,
                    0,
                    Arc::clone(&EMPTY_ENUM_LABELS),
                )
            }
        }
        ChannelId::Math(idx) => {
            if let Some(mc) = shared.math_channels.get(idx) {
                (
                    mc.name.clone(),
                    mc.unit.clone(),
                    mc.freq,
                    mc.dec_places,
                    Arc::new(
                        mc.options
                            .enum_labels
                            .iter()
                            .map(|(v, l)| (*v, l.clone()))
                            .collect(),
                    ),
                )
            } else {
                (
                    "???".into(),
                    String::new(),
                    0.0,
                    0,
                    Arc::clone(&EMPTY_ENUM_LABELS),
                )
            }
        }
    }
}

pub fn resolve_plotted_channel_display_meta(
    channel: &PlottedChannel,
    shared: &SharedState,
) -> ChannelDisplayMeta {
    let (name, mut unit, freq, dec_places, enum_labels) =
        resolve_channel_display_meta(channel.channel_id, shared);
    if let Some(display_unit) = &channel.display_unit {
        unit = display_unit.clone();
    }
    (name, unit, freq, dec_places, enum_labels)
}

/// Resolve channel metadata (name, unit, freq, dec_places) from a ChannelId.
pub fn resolve_channel_meta(id: ChannelId, shared: &SharedState) -> (String, String, f64, i16) {
    let (name, unit, freq, dec_places, _) = resolve_channel_display_meta(id, shared);
    (name, unit, freq, dec_places)
}

fn normalized_unit(unit: &str) -> String {
    unit.to_ascii_lowercase().replace(' ', "")
}

pub fn display_presets_for_unit(unit: &str) -> Vec<DisplayUnitPreset> {
    match normalized_unit(unit).as_str() {
        "rad" => vec![DisplayUnitPreset {
            label: "Show as degrees",
            scale: 180.0 / std::f64::consts::PI,
            offset: 0.0,
            unit: "deg",
        }],
        "deg" | "°" => vec![DisplayUnitPreset {
            label: "Show as radians",
            scale: std::f64::consts::PI / 180.0,
            offset: 0.0,
            unit: "rad",
        }],
        "km/h" | "kph" => vec![DisplayUnitPreset {
            label: "Show as mph",
            scale: 0.621_371_192,
            offset: 0.0,
            unit: "mph",
        }],
        "mph" => vec![DisplayUnitPreset {
            label: "Show as km/h",
            scale: 1.609_344,
            offset: 0.0,
            unit: "km/h",
        }],
        "kpa" => vec![DisplayUnitPreset {
            label: "Show as psi",
            scale: 0.145_037_738,
            offset: 0.0,
            unit: "psi",
        }],
        "psi" => vec![DisplayUnitPreset {
            label: "Show as kPa",
            scale: 6.894_757_293,
            offset: 0.0,
            unit: "kPa",
        }],
        "°c" | "degc" | "c" => vec![DisplayUnitPreset {
            label: "Show as °F",
            scale: 1.8,
            offset: 32.0,
            unit: "°F",
        }],
        "°f" | "degf" | "f" => vec![DisplayUnitPreset {
            label: "Show as °C",
            scale: 5.0 / 9.0,
            offset: -32.0 * 5.0 / 9.0,
            unit: "°C",
        }],
        _ => Vec::new(),
    }
}

pub fn transform_channel_value(channel: &PlottedChannel, value: f64) -> f64 {
    value * channel.display_scale + channel.display_offset
}

pub fn display_transform_fingerprint(channel: &PlottedChannel) -> DisplayTransformFingerprint {
    DisplayTransformFingerprint {
        scale_bits: channel.display_scale.to_bits(),
        offset_bits: channel.display_offset.to_bits(),
        unit: channel.display_unit.clone(),
    }
}

/// Interpolate a channel value at a given time using linear interpolation.
pub fn interp_at_time(data: &[f64], freq: f64, time: f64) -> Option<f64> {
    i3rs_core::analysis::sample_at_time(data, freq, time)
}

/// Shared searchable picker for analysis channel roles, including math channels.
pub fn channel_picker(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut Option<ChannelId>,
    filter: &mut String,
    shared: &SharedState,
) -> bool {
    let previous = *selected;
    let name = selected
        .map(|id| resolve_channel_meta(id, shared).0)
        .unwrap_or_else(|| "None".into());
    egui::ComboBox::from_id_salt(id)
        .selected_text(name)
        .width(170.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.text_edit_singleline(filter);
            let query = filter.to_lowercase();
            if ui.selectable_value(selected, None, "None").clicked() {
                ui.close();
            }
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    if let Some(ld) = &shared.ld_file {
                        for ch in &ld.channels {
                            if ch.name.to_lowercase().contains(&query)
                                && ui
                                    .selectable_value(
                                        selected,
                                        Some(ChannelId::Physical(ch.index)),
                                        &ch.name,
                                    )
                                    .clicked()
                            {
                                ui.close();
                            }
                        }
                    }
                    for candidate in shared.channel_catalog().iter().filter(|c| {
                        matches!(
                            c.key,
                            i3rs_core::ChannelKey::Constant(_)
                                | i3rs_core::ChannelKey::Generated(_)
                        )
                    }) {
                        if candidate.name.to_lowercase().contains(&query) {
                            let reference = format!("{}::{}", candidate.source, candidate.name);
                            let id = shared.analysis_channel(&reference);
                            if ui
                                .selectable_value(selected, Some(id), &candidate.name)
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    }
                    for (idx, ch) in shared.math_channels.iter().enumerate() {
                        if ch.name.to_lowercase().contains(&query)
                            && ui
                                .selectable_value(
                                    selected,
                                    Some(ChannelId::Math(idx)),
                                    format!("{} (math)", ch.name),
                                )
                                .clicked()
                        {
                            ui.close();
                        }
                    }
                });
        });
    *selected != previous
}

/// Sample positions in [start, end). Never include a position before a beacon.
pub fn sample_range(len: usize, freq: f64, range: (f64, f64)) -> std::ops::Range<usize> {
    let window = i3rs_core::analysis::sample_window_for_time(freq, len, range.0, range.1);
    window.start_sample..window.end_sample
}

/// Load raw channel data from a ChannelId, returning the immutable shared sample buffer.
pub fn load_channel_data(channel_id: ChannelId, shared: &SharedState) -> Option<Arc<[f64]>> {
    if !matches!(channel_id, ChannelId::Unresolved(_)) && !shared.channel_enabled(channel_id) {
        return None;
    }
    match channel_id {
        ChannelId::Analysis(i) => Some(
            shared
                .analysis_series(i)
                .map_or_else(|| Arc::clone(&EMPTY_SAMPLES), |(data, _)| data),
        ),
        ChannelId::Unresolved(_) => Some(Arc::clone(&EMPTY_SAMPLES)),
        ChannelId::Physical(idx) => {
            if let Some(decoded) = shared.decoded_physical_channel_if_ready(idx) {
                Some(Arc::clone(&decoded.data))
            } else {
                shared.request_physical_channel_decode(idx);
                Some(Arc::clone(&EMPTY_SAMPLES))
            }
        }
        ChannelId::Math(idx) => {
            let mc = shared.math_channels.get(idx)?;
            Some(
                mc.data
                    .as_ref()
                    .map_or_else(|| Arc::clone(&EMPTY_SAMPLES), Arc::clone),
            )
        }
    }
}

/// Load channel data and wrap it in a PlottedChannel with stats and color.
pub fn create_plotted_channel(
    channel_id: ChannelId,
    shared: &SharedState,
    color_idx: usize,
) -> Option<PlottedChannel> {
    let data = load_channel_data(channel_id, shared)?;
    let (cached_min, cached_max, cached_avg) = match channel_id {
        ChannelId::Analysis(_) => basic_stats(&data),
        ChannelId::Unresolved(_) => (0., 0., 0.),
        ChannelId::Physical(idx) => {
            if let Some(decoded) = shared.decoded_physical_channel_if_ready(idx) {
                (decoded.stats.min, decoded.stats.max, decoded.stats.avg)
            } else {
                (0.0, 0.0, 0.0)
            }
        }
        ChannelId::Math(idx) => {
            let mc = shared.math_channels.get(idx)?;
            (mc.cached_min, mc.cached_max, mc.cached_avg)
        }
    };
    let mut plotted = PlottedChannel {
        channel_id,
        color: CHANNEL_COLORS[color_idx % CHANNEL_COLORS.len()],
        data,
        tile_group: color_idx,
        y_axis: YAxis::Left,
        display_scale: 1.0,
        display_offset: 0.0,
        display_unit: None,
        math_id: match channel_id {
            ChannelId::Math(idx) => shared.math_channels.get(idx).map(|mc| mc.id),
            _ => None,
        },
        requested_reference: if let ChannelId::Analysis(i) = channel_id {
            shared.unresolved_references.borrow().get(i).cloned()
        } else {
            None
        },
        portable_source_unit: None,
        recording_display_base: None,
        math_display_signature: None,
        scale_mode: crate::state::ScaleMode::Auto,
        manual_min: cached_min,
        manual_max: cached_max,
        cached_min,
        cached_max,
        cached_avg,
    };
    apply_math_display_defaults(&mut plotted, shared);
    apply_channel_preferences(&mut plotted, shared);
    Some(plotted)
}

/// Definition display defaults are separate from raw result units and panel overrides.
pub fn apply_math_display_defaults(channel: &mut PlottedChannel, shared: &SharedState) {
    let ChannelId::Math(idx) = channel.channel_id else {
        return;
    };
    let Some(mc) = shared.math_channels.get(idx) else {
        return;
    };
    let signature = (mc.unit.clone(), mc.options.display_unit.clone());
    if channel.math_display_signature.as_ref() == Some(&signature) {
        return;
    }
    let projection = |from: &str, to: &Option<String>| {
        to.as_ref()
            .and_then(|unit| i3rs_core::channels::unit_transform(from, unit).ok())
            .map_or((1.0, 0.0, None), |(scale, offset)| {
                (scale, offset, to.clone())
            })
    };
    let before = channel
        .math_display_signature
        .as_ref()
        .map(|(from, to)| projection(from, to));
    let follows_default = before.map_or(
        channel.display_scale == 1.0
            && channel.display_offset == 0.0
            && channel.display_unit.is_none(),
        |(scale, offset, unit)| {
            (channel.display_scale - scale).abs() < 1e-9
                && (channel.display_offset - offset).abs() < 1e-9
                && channel.display_unit == unit
        },
    );
    if follows_default {
        let (scale, offset, unit) = projection(&signature.0, &signature.1);
        set_plotted_channel_display_transform(channel, scale, offset, unit);
    } else if channel
        .math_display_signature
        .as_ref()
        .is_some_and(|(from, _)| from != &signature.0)
        && let Some(unit) = &channel.display_unit
    {
        if let Ok((scale, offset)) = i3rs_core::channels::unit_transform(&signature.0, unit) {
            set_plotted_channel_display_transform(channel, scale, offset, Some(unit.clone()));
        } else {
            let (scale, offset, unit) = projection(&signature.0, &signature.1);
            set_plotted_channel_display_transform(channel, scale, offset, unit);
            shared.message(format!(
                "{}: display units reset after result unit change",
                mc.name
            ));
        }
    }
    if channel
        .math_display_signature
        .as_ref()
        .is_some_and(|(unit, _)| unit != &signature.0)
    {
        channel.portable_source_unit = Some(signature.0.clone());
    }
    channel.math_display_signature = Some(signature);
}

pub fn reconcile_source_unit(channel: &mut PlottedChannel, shared: &SharedState) -> bool {
    if matches!(channel.channel_id, ChannelId::Unresolved(_)) {
        return true;
    }
    let unit = resolve_channel_meta(channel.channel_id, shared).1;
    if let Some(previous) = &channel.portable_source_unit
        && !unit.is_empty()
        && !previous.is_empty()
        && unit != *previous
    {
        match i3rs_core::channels::unit_transform(&unit, previous) {
            Ok((scale, offset)) => {
                if channel.display_unit.is_none() {
                    channel.display_unit = Some(previous.clone());
                }
                channel.display_offset += offset * channel.display_scale;
                channel.display_scale *= scale;
            }
            Err(error) => {
                shared.message(error);
                channel.data = Arc::clone(&EMPTY_SAMPLES);
                return false;
            }
        }
    }
    channel.portable_source_unit = Some(unit);
    true
}

pub fn refresh_plotted_channel(channel: &mut PlottedChannel, shared: &SharedState) -> bool {
    if let Some((reference, false)) = &channel.requested_reference {
        let id = shared
            .resolve_slot(reference, false)
            .unwrap_or_else(|| shared.unresolved_channel(reference, false));
        if id != channel.channel_id {
            if channel.portable_source_unit.is_none()
                && !matches!(channel.channel_id, ChannelId::Unresolved(_))
            {
                channel.portable_source_unit =
                    Some(resolve_channel_meta(channel.channel_id, shared).1);
            }
            channel.channel_id = id;
            channel.math_id = None;
            channel.data = Arc::clone(&EMPTY_SAMPLES);
            channel.recording_display_base = None;
        }
    }
    if let ChannelId::Unresolved(index) = channel.channel_id {
        let reference = shared.unresolved_references.borrow().get(index).cloned();
        if let Some((name, math)) = reference
            && let Some(id) = shared.resolve_slot(&name, math)
        {
            channel.channel_id = id;
            channel.math_id = if let ChannelId::Math(i) = id {
                shared.math_channels.get(i).map(|m| m.id)
            } else {
                None
            };
        } else {
            return false;
        }
    }
    if let Some(id) = channel.math_id {
        let Some(idx) = shared.math_channel_index_by_id(id) else {
            if let Some((reference, kind)) = &channel.requested_reference {
                channel.channel_id = shared.unresolved_channel(reference, *kind);
                channel.math_id = None;
            } else {
                channel.channel_id = ChannelId::Math(usize::MAX);
            }
            let changed = !channel.data.is_empty();
            channel.data = Arc::clone(&EMPTY_SAMPLES);
            return changed;
        };
        channel.channel_id = ChannelId::Math(idx);
    }
    if !shared.channel_enabled(channel.channel_id) {
        let changed = !channel.data.is_empty();
        channel.data = Arc::clone(&EMPTY_SAMPLES);
        return changed;
    }
    apply_math_display_defaults(channel, shared);
    apply_recording_display_transform(channel, shared);
    if !reconcile_source_unit(channel, shared) {
        return false;
    }
    match channel.channel_id {
        ChannelId::Analysis(i) => {
            let data = shared
                .analysis_series(i)
                .map_or_else(|| Arc::clone(&EMPTY_SAMPLES), |(data, _)| data);
            let changed = !Arc::ptr_eq(&data, &channel.data);
            if changed {
                let (min, max, avg) = basic_stats(&data);
                channel.cached_min = min;
                channel.cached_max = max;
                channel.cached_avg = avg;
                channel.data = data;
            }
            changed
        }
        ChannelId::Unresolved(_) => false,
        ChannelId::Physical(idx) => {
            if let Some(decoded) = shared.decoded_physical_channel_if_ready(idx) {
                let changed = channel.data.as_ptr() != decoded.data.as_ptr()
                    || channel.data.len() != decoded.data.len()
                    || channel.cached_min.to_bits() != decoded.stats.min.to_bits()
                    || channel.cached_max.to_bits() != decoded.stats.max.to_bits()
                    || channel.cached_avg.to_bits() != decoded.stats.avg.to_bits();
                if changed {
                    channel.data = Arc::clone(&decoded.data);
                    channel.cached_min = decoded.stats.min;
                    channel.cached_max = decoded.stats.max;
                    channel.cached_avg = decoded.stats.avg;
                }
                changed
            } else {
                shared.request_physical_channel_decode(idx);
                false
            }
        }
        ChannelId::Math(idx) => {
            let Some(math_channel) = shared.math_channels.get(idx) else {
                return false;
            };
            let Some(data) = &math_channel.data else {
                let changed = !channel.data.is_empty();
                channel.data = Arc::clone(&EMPTY_SAMPLES);
                return changed;
            };
            let changed = channel.data.as_ptr() != data.as_ptr()
                || channel.data.len() != data.len()
                || channel.cached_min.to_bits() != math_channel.cached_min.to_bits()
                || channel.cached_max.to_bits() != math_channel.cached_max.to_bits()
                || channel.cached_avg.to_bits() != math_channel.cached_avg.to_bits();
            if changed {
                channel.data = Arc::clone(data);
                channel.cached_min = math_channel.cached_min;
                channel.cached_max = math_channel.cached_max;
                channel.cached_avg = math_channel.cached_avg;
            }
            changed
        }
    }
}

/// Build readout metadata for a plotted channel.
pub fn build_plotted_channel_info(
    channel: &PlottedChannel,
    shared: &SharedState,
) -> PlottedChannelInfo {
    let (name, unit, freq, dec_places, enum_labels) =
        resolve_plotted_channel_display_meta(channel, shared);
    PlottedChannelInfo {
        channel_id: channel.channel_id,
        name,
        unit,
        freq,
        dec_places,
        color: channel.color,
        data: channel.data.clone(),
        display_scale: channel.display_scale,
        display_offset: channel.display_offset,
        interpolation: match channel.channel_id {
            ChannelId::Analysis(i) => {
                let reference = shared
                    .unresolved_references
                    .borrow()
                    .get(i)
                    .map(|r| r.0.clone())
                    .unwrap_or_default();
                if matches!(
                    shared.resolve_analysis_reference(&reference),
                    i3rs_core::ChannelResolution::Resolved {
                        key: i3rs_core::ChannelKey::Generated(_),
                        ..
                    }
                ) {
                    i3rs_core::Interpolation::Linear
                } else {
                    i3rs_core::Interpolation::Previous
                }
            }
            ChannelId::Unresolved(_) => Default::default(),
            ChannelId::Physical(idx) => shared
                .ld_file
                .as_ref()
                .and_then(|ld| ld.channels.get(idx))
                .and_then(|ch| {
                    shared
                        .channel_settings
                        .mapping(&ch.name, shared.recording_key())
                })
                .map_or(Default::default(), |m| m.interpolation),
            ChannelId::Math(idx) => shared
                .math_channels
                .get(idx)
                .map_or(Default::default(), |mc| mc.options.interpolation),
        },
        enum_labels,
    }
}

fn apply_recording_display_transform(channel: &mut PlottedChannel, shared: &SharedState) {
    let mapping = if let ChannelId::Physical(idx) = channel.channel_id {
        shared
            .ld_file
            .as_ref()
            .and_then(|ld| ld.channels.get(idx))
            .and_then(|ch| {
                shared
                    .channel_settings
                    .mapping(&ch.name, shared.recording_key())
            })
    } else {
        None
    };
    if let Some(mapping) = mapping
        .filter(|m| m.display_scale != 1.0 || m.display_offset != 0.0 || m.display_unit.is_some())
    {
        if channel.recording_display_base.is_none() {
            channel.recording_display_base = Some((
                channel.display_scale,
                channel.display_offset,
                channel.display_unit.clone(),
            ));
        }
        set_plotted_channel_display_transform(
            channel,
            mapping.display_scale,
            mapping.display_offset,
            mapping.display_unit.clone(),
        );
    } else if let Some((scale, offset, unit)) = channel.recording_display_base.take() {
        set_plotted_channel_display_transform(channel, scale, offset, unit);
    }
}

pub fn apply_channel_preferences(channel: &mut PlottedChannel, shared: &SharedState) {
    let (name, _, _, _, _) = resolve_channel_display_meta(channel.channel_id, shared);
    let key = channel_preference_key(&name);
    if let Some(pref) = shared.channel_preferences.get(&key) {
        apply_channel_preference(channel, pref);
    }
    apply_recording_display_transform(channel, shared);
}

pub fn apply_channel_preference(channel: &mut PlottedChannel, pref: &ChannelPreference) {
    if let Some(color) = pref.color {
        channel.color = egui::Color32::from_rgb(color[0], color[1], color[2]);
    }
    channel.display_scale = pref.display_scale;
    channel.display_offset = pref.display_offset;
    channel.display_unit = pref.display_unit.clone();
    match pref.scale_manual {
        Some((min, max)) => {
            channel.scale_mode = crate::state::ScaleMode::Manual;
            channel.manual_min = min;
            channel.manual_max = max;
        }
        None => {
            channel.scale_mode = crate::state::ScaleMode::Auto;
        }
    }
}

/// Change display units while keeping manual limits tied to the same raw values.
pub fn set_plotted_channel_display_transform(
    channel: &mut PlottedChannel,
    scale: f64,
    offset: f64,
    unit: Option<String>,
) {
    if channel.scale_mode == crate::state::ScaleMode::Manual {
        let (mut min, mut max) = if channel.display_scale.abs() > f64::EPSILON {
            let transform = |value: f64| {
                let raw = (value - channel.display_offset) / channel.display_scale;
                raw * scale + offset
            };
            (transform(channel.manual_min), transform(channel.manual_max))
        } else {
            (
                channel.cached_min * scale + offset,
                channel.cached_max * scale + offset,
            )
        };
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        channel.manual_min = min;
        channel.manual_max = max;
    }

    channel.display_scale = scale;
    channel.display_offset = offset;
    channel.display_unit = unit;
}

fn merged_display_preference(
    existing: Option<&ChannelPreference>,
    channel: &PlottedChannel,
) -> ChannelPreference {
    ChannelPreference {
        color: existing.and_then(|pref| pref.color),
        display_scale: channel.display_scale,
        display_offset: channel.display_offset,
        display_unit: channel.display_unit.clone(),
        scale_manual: match channel.scale_mode {
            crate::state::ScaleMode::Manual => Some((channel.manual_min, channel.manual_max)),
            crate::state::ScaleMode::Auto => None,
        },
    }
}

pub fn show_plotted_channel_display_menu(
    ui: &mut egui::Ui,
    channel: &mut PlottedChannel,
    shared: &mut SharedState,
) -> bool {
    let (name, raw_unit, _, _, _) = resolve_channel_display_meta(channel.channel_id, shared);
    let mut remove_channel = false;

    if ui.button("Remove").clicked() {
        remove_channel = true;
        ui.close();
    }

    ui.separator();
    ui.label("Color:");
    for (i, &color) in CHANNEL_COLORS.iter().enumerate() {
        let label = format!("Color {}", i + 1);
        let resp = ui.selectable_label(channel.color == color, &label);
        let rect = resp.rect;
        let swatch = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 14.0, rect.center().y - 5.0),
            egui::vec2(10.0, 10.0),
        );
        ui.painter().rect_filled(swatch, 2.0, color);
        if resp.clicked() {
            channel.color = color;
            ui.close();
        }
    }

    let presets = display_presets_for_unit(&raw_unit);
    if !presets.is_empty() || channel.display_unit.is_some() {
        ui.separator();
        ui.label("Display units:");
        let raw_selected = channel.display_scale == 1.0
            && channel.display_offset == 0.0
            && channel.display_unit.is_none();
        if ui
            .selectable_label(raw_selected, format!("Raw ({})", raw_unit))
            .clicked()
        {
            set_plotted_channel_display_transform(channel, 1.0, 0.0, None);
            ui.close();
        }
        for preset in presets {
            let is_selected = (channel.display_scale - preset.scale).abs() < 1e-9
                && (channel.display_offset - preset.offset).abs() < 1e-9
                && channel.display_unit.as_deref() == Some(preset.unit);
            if ui.selectable_label(is_selected, preset.label).clicked() {
                set_plotted_channel_display_transform(
                    channel,
                    preset.scale,
                    preset.offset,
                    Some(preset.unit.to_string()),
                );
                ui.close();
            }
        }
    }

    if let ChannelId::Physical(idx) = channel.channel_id
        && !uses_discrete_values(
            &name,
            &resolve_channel_display_meta(channel.channel_id, shared).4,
        )
        && let Some(time) = shared.cursor_time
        && let Some(raw) = i3rs_core::channels::sample_with_policy(
            &channel.data,
            resolve_channel_meta(channel.channel_id, shared).2,
            time,
            i3rs_core::channels::Interpolation::Linear,
        )
        && ui.button("Zero at cursor for this recording").clicked()
    {
        let original = shared.ld_file.as_ref().unwrap().channels[idx].name.clone();
        let recording = shared.recording_key().to_string();
        let mut mapping = shared
            .channel_settings
            .mapping(&original, &recording)
            .cloned()
            .unwrap_or_default();
        mapping.original = original;
        mapping.recording = Some(recording);
        mapping.display_scale = channel.display_scale;
        mapping.display_offset = -raw * channel.display_scale;
        mapping.display_unit = channel.display_unit.clone();
        if let Some(existing) = shared
            .channel_settings
            .mappings
            .iter_mut()
            .find(|m| m.original == mapping.original && m.recording == mapping.recording)
        {
            *existing = mapping;
        } else {
            shared.channel_settings.mappings.push(mapping);
        }
        shared.invalidate_math_results();
        apply_recording_display_transform(channel, shared);
        ui.close();
    }

    let pref_key = channel_preference_key(&name);
    let has_global_pref = shared.channel_preferences.contains_key(&pref_key);
    ui.separator();
    if ui.button("Save current style as global default").clicked() {
        shared.channel_preferences.insert(
            pref_key.clone(),
            ChannelPreference {
                color: Some([channel.color.r(), channel.color.g(), channel.color.b()]),
                ..merged_display_preference(shared.channel_preferences.get(&pref_key), channel)
            },
        );
        shared.channel_preferences_dirty = true;
        ui.close();
    }
    if ui
        .add_enabled(has_global_pref, egui::Button::new("Apply global default"))
        .clicked()
    {
        apply_channel_preferences(channel, shared);
        ui.close();
    }
    if ui
        .add_enabled(has_global_pref, egui::Button::new("Clear global default"))
        .clicked()
    {
        shared.channel_preferences.remove(&pref_key);
        shared.channel_preferences_dirty = true;
        ui.close();
    }

    remove_channel
}

pub fn segmented_channel_button(
    ui: &mut egui::Ui,
    label: &str,
    fill: Option<egui::Color32>,
    clear_tooltip: &str,
) -> (egui::Response, bool) {
    let height = ui.spacing().interact_size.y;
    let text_width = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    let main_width = (text_width + 2. * ui.spacing().button_padding.x)
        .min(240.)
        .min((ui.max_rect().width() - height).max(40.));
    let output = ui.allocate_ui_with_layout(
        egui::vec2(main_width + height, height),
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(false),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            let mut main_button = egui::Button::new(label).truncate();
            let mut clear_button = egui::Button::new(egui::RichText::new("X").small());
            if let Some(fill) = fill {
                main_button = main_button.fill(fill);
                clear_button = clear_button.fill(fill);
            }
            let main_response = ui
                .add_sized([main_width, height], main_button)
                .on_hover_text(label);
            let clear_response = ui
                .add_sized([height, height], clear_button)
                .on_hover_text(clear_tooltip);
            (main_response, clear_response)
        },
    );

    let (main_response, clear_response) = output.inner;
    let divider_stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter().line_segment(
        [
            egui::pos2(clear_response.rect.left(), clear_response.rect.top() + 2.0),
            egui::pos2(
                clear_response.rect.left(),
                clear_response.rect.bottom() - 2.0,
            ),
        ],
        divider_stroke,
    );

    (main_response, clear_response.clicked())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ChannelId;

    #[test]
    fn merged_display_preference_preserves_existing_color() {
        let existing = ChannelPreference {
            color: Some([1, 2, 3]),
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            scale_manual: None,
        };
        let channel = PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![1.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 2.0,
            display_offset: 5.0,
            display_unit: Some("psi".into()),
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
        };

        let merged = merged_display_preference(Some(&existing), &channel);

        assert_eq!(merged.color, Some([1, 2, 3]));
        assert_eq!(merged.display_scale, 2.0);
        assert_eq!(merged.display_offset, 5.0);
        assert_eq!(merged.display_unit.as_deref(), Some("psi"));
    }

    #[test]
    fn applying_preference_sets_and_clears_manual_limits() {
        let mut channel = PlottedChannel {
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
            manual_min: 0.0,
            manual_max: 1.0,
            cached_min: 0.0,
            cached_max: 1.0,
            cached_avg: 0.5,
        };
        let mut preference = ChannelPreference {
            scale_manual: Some((10.0, 20.0)),
            ..ChannelPreference::default()
        };

        apply_channel_preference(&mut channel, &preference);
        assert_eq!(channel.scale_mode, crate::state::ScaleMode::Manual);
        assert_eq!((channel.manual_min, channel.manual_max), (10.0, 20.0));

        preference.scale_manual = None;
        apply_channel_preference(&mut channel, &preference);
        assert_eq!(channel.scale_mode, crate::state::ScaleMode::Auto);
    }

    #[test]
    fn unit_conversion_preserves_manual_limits_as_raw_values() {
        let mut channel = PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data: Arc::from(vec![0.0, 1000.0]),
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 0.145_037_738,
            display_offset: 0.0,
            display_unit: Some("psi".into()),
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: crate::state::ScaleMode::Manual,
            manual_min: 0.0,
            manual_max: 145.037_738,
            cached_min: 0.0,
            cached_max: 1000.0,
            cached_avg: 500.0,
        };

        set_plotted_channel_display_transform(&mut channel, 1.0, 0.0, None);

        assert!((channel.manual_min - 0.0).abs() < 1e-9);
        assert!((channel.manual_max - 1000.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod portable_unit_tests {
    use super::*;
    #[test]
    fn math_result_unit_change_does_not_apply_the_conversion_twice() {
        let mut shared = SharedState::new();
        let mut m = shared.create_math_channel_def("Temperature".into(), "0".into(), "C".into(), 1);
        m.options.display_unit = Some("F".into());
        m.data = Some(Arc::from(vec![0.]));
        m.freq = 1.0;
        shared.math_channels.push(m);
        let mut pc = create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap();
        refresh_plotted_channel(&mut pc, &shared);
        assert!((pc.display_scale - 1.8).abs() < 1e-9);
        shared.math_channels[0].unit = "F".into();
        shared.math_channels[0].data = Some(Arc::from(vec![32.]));
        refresh_plotted_channel(&mut pc, &shared);
        assert_eq!((pc.display_scale, pc.display_offset), (1., 0.));
        assert_eq!(pc.data[0], 32.);
    }
}

fn basic_stats(data: &[f64]) -> (f64, f64, f64) {
    let (mut low, mut high, mut sum, mut count) = (f64::INFINITY, f64::NEG_INFINITY, 0., 0usize);
    for &value in data.iter().filter(|v| v.is_finite()) {
        low = low.min(value);
        high = high.max(value);
        sum += value;
        count += 1;
    }
    if count == 0 {
        (0., 0., 0.)
    } else {
        (low, high, sum / count as f64)
    }
}

#[cfg(test)]
mod generated_slot_tests {
    use super::*;
    #[test]
    fn generated_channels_and_scoped_constants_rebind_and_reuse_native_buffers() {
        let mut shared = SharedState::new();
        shared.data_duration = Some(3.);
        shared.correction_speed = Some((Arc::from(vec![1., 2., 3., 4., 5., 6.]), 2.0));
        let id = shared.resolve_slot("MoTeC::Corr Speed", false).unwrap();
        let mut pc = create_plotted_channel(id, &shared, 0).unwrap();
        assert!(matches!(id, ChannelId::Analysis(_)));
        assert_eq!(resolve_channel_meta(id, &shared).2, 2.0);
        assert_eq!(pc.data.len(), 6);
        assert_eq!((pc.cached_min, pc.cached_max), (1., 6.));
        assert!(!refresh_plotted_channel(&mut pc, &shared));
        shared
            .channel_settings
            .constants
            .push(i3rs_core::channels::SetupConstant {
                name: "Mass".into(),
                value: 1000.,
                unit: "kg".into(),
                recording: shared.recording_key().into(),
                comment: String::new(),
            });
        let id = shared.resolve_slot("Mass", false).unwrap();
        let mut c = create_plotted_channel(id, &shared, 0).unwrap();
        let before = c.data.clone();
        assert_eq!(&*c.data, &[1000.; 3]);
        assert!(!refresh_plotted_channel(&mut c, &shared));
        assert!(Arc::ptr_eq(&before, &c.data));
        shared.channel_settings.constants[0].value = 1100.;
        assert!(refresh_plotted_channel(&mut c, &shared));
        assert_eq!(&*c.data, &[1100.; 3]);
        let dock = egui_dock::DockState::new(vec![crate::panels::PanelTab::Graph({
            let mut g = crate::panels::graph::GraphPanel::new(1, "Generated");
            g.plotted_channels.push(pc);
            g.plotted_channels.push(c);
            g
        })]);
        let workspace = crate::workspace::save_workspace(&[("Sheet".into(), &dock)], 0, &shared);
        let loaded = crate::workspace::load_workspace(&workspace, &mut shared);
        let (_, crate::panels::PanelTab::Graph(graph)) =
            loaded[0].1.iter_all_tabs().next().unwrap()
        else {
            panic!()
        };
        assert_eq!(graph.plotted_channels.len(), 2);
        assert!(
            graph
                .plotted_channels
                .iter()
                .all(|p| matches!(p.channel_id, ChannelId::Analysis(_)))
        );
    }
}

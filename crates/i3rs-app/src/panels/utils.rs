//! Shared utilities used by multiple panel types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use eframe::egui;

use crate::state::{
    CHANNEL_COLORS, ChannelId, ChannelPreference, PlottedChannel, PlottedChannelInfo, SharedState,
    YAxis, channel_preference_key,
};

pub type ChannelDisplayMeta = (String, String, u16, i16, Arc<HashMap<i64, String>>);

static EMPTY_ENUM_LABELS: LazyLock<Arc<HashMap<i64, String>>> =
    LazyLock::new(|| Arc::new(HashMap::new()));
static EMPTY_SAMPLES: LazyLock<Arc<[f64]>> = LazyLock::new(|| Arc::from(Vec::<f64>::new()));

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
        ChannelId::Physical(idx) => {
            if let Some(ld) = &shared.ld_file
                && let Some(ch) = ld.channels.get(idx)
            {
                (
                    ch.name.clone(),
                    ch.unit.clone(),
                    ch.freq,
                    ch.dec_places,
                    Arc::clone(&ch.enum_labels),
                )
            } else {
                (
                    "???".into(),
                    String::new(),
                    0,
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
                    Arc::clone(&EMPTY_ENUM_LABELS),
                )
            } else {
                (
                    "???".into(),
                    String::new(),
                    0,
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
pub fn resolve_channel_meta(id: ChannelId, shared: &SharedState) -> (String, String, u16, i16) {
    let (name, unit, freq, dec_places, _) = resolve_channel_display_meta(id, shared);
    (name, unit, freq, dec_places)
}

fn normalized_unit(unit: &str) -> String {
    unit.to_ascii_lowercase().replace(' ', "")
}

pub fn display_presets_for_unit(unit: &str) -> Vec<DisplayUnitPreset> {
    match normalized_unit(unit).as_str() {
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
pub fn interp_at_time(data: &[f64], freq: u16, time: f64) -> Option<f64> {
    if freq == 0 || data.is_empty() {
        return None;
    }
    let sample_f = time * freq as f64;
    let idx = sample_f as usize;
    if idx >= data.len() {
        return None;
    }
    if idx + 1 < data.len() {
        let frac = sample_f - idx as f64;
        Some(data[idx] * (1.0 - frac) + data[idx + 1] * frac)
    } else {
        Some(data[idx])
    }
}

/// Extract visible data slice based on zoom range, filtering out non-finite values.
pub fn get_visible_slice(data: &[f64], freq: u16, shared: &SharedState) -> Vec<f64> {
    if let Some((t0, t1)) = shared.zoom_range {
        let start = (t0 * freq as f64).floor() as usize;
        let end = (t1 * freq as f64).ceil() as usize;
        let start = start.min(data.len());
        let end = end.min(data.len());
        data[start..end]
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect()
    } else {
        data.iter().copied().filter(|v| v.is_finite()).collect()
    }
}

/// Load raw channel data from a ChannelId, returning the immutable shared sample buffer.
pub fn load_channel_data(channel_id: ChannelId, shared: &SharedState) -> Option<Arc<[f64]>> {
    match channel_id {
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
            Some(Arc::clone(mc.data.as_ref()?))
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
        scale_mode: crate::state::ScaleMode::Auto,
        manual_min: cached_min,
        manual_max: cached_max,
        cached_min,
        cached_max,
        cached_avg,
    };
    apply_channel_preferences(&mut plotted, shared);
    Some(plotted)
}

pub fn refresh_plotted_channel(channel: &mut PlottedChannel, shared: &SharedState) -> bool {
    match channel.channel_id {
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
                return false;
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
        name,
        unit,
        freq,
        dec_places,
        color: channel.color,
        data: channel.data.clone(),
        display_scale: channel.display_scale,
        display_offset: channel.display_offset,
        enum_labels,
    }
}

pub fn apply_channel_preferences(channel: &mut PlottedChannel, shared: &SharedState) {
    let (name, _, _, _, _) = resolve_channel_display_meta(channel.channel_id, shared);
    let key = channel_preference_key(&name);
    let Some(pref) = shared.channel_preferences.get(&key) else {
        return;
    };

    apply_channel_preference(channel, pref);
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
    let output = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;

        let mut main_button = egui::Button::new(label);
        if let Some(fill) = fill {
            main_button = main_button.fill(fill);
        }
        let main_response = ui.add(main_button);

        let button_size = egui::vec2(main_response.rect.height(), main_response.rect.height());
        let mut clear_button = egui::Button::new(egui::RichText::new("X").small());
        if let Some(fill) = fill {
            clear_button = clear_button.fill(fill);
        }
        let clear_response = ui
            .add_sized([button_size.x, button_size.y], clear_button)
            .on_hover_text(clear_tooltip);

        (main_response, clear_response)
    });

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

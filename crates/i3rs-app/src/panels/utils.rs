//! Shared utilities used by multiple panel types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use eframe::egui;

use crate::state::{
    CHANNEL_COLORS, ChannelId, ChannelPreference, PlottedChannel, PlottedChannelInfo, SharedState,
    YAxis, channel_preference_key, compute_channel_stats,
};

pub type ChannelDisplayMeta = (String, String, u16, i16, Arc<HashMap<i64, String>>);

static EMPTY_ENUM_LABELS: LazyLock<Arc<HashMap<i64, String>>> =
    LazyLock::new(|| Arc::new(HashMap::new()));

#[derive(Clone, Copy)]
pub struct DisplayUnitPreset {
    pub label: &'static str,
    pub scale: f64,
    pub offset: f64,
    pub unit: &'static str,
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

pub fn display_transform_fingerprint(channel: &PlottedChannel) -> (u64, u64, Option<String>) {
    (
        channel.display_scale.to_bits(),
        channel.display_offset.to_bits(),
        channel.display_unit.clone(),
    )
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

/// Load raw channel data from a ChannelId, returning the data vector.
pub fn load_channel_data(channel_id: ChannelId, shared: &SharedState) -> Option<Vec<f64>> {
    match channel_id {
        ChannelId::Physical(idx) => {
            let ld = shared.ld_file.as_ref()?;
            ld.read_channel_data(ld.channels.get(idx)?)
        }
        ChannelId::Math(idx) => {
            let mc = shared.math_channels.get(idx)?;
            Some((**mc.data.as_ref()?).clone())
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
    let (min, max, avg, _) = compute_channel_stats(&data);
    let mut plotted = PlottedChannel {
        channel_id,
        color: CHANNEL_COLORS[color_idx % CHANNEL_COLORS.len()],
        data: Arc::new(data),
        tile_group: color_idx,
        y_axis: YAxis::Left,
        display_scale: 1.0,
        display_offset: 0.0,
        display_unit: None,
        cached_min: min,
        cached_max: max,
        cached_avg: avg,
    };
    apply_channel_preferences(&mut plotted, shared);
    Some(plotted)
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

    if let Some(color) = pref.color {
        channel.color = egui::Color32::from_rgb(color[0], color[1], color[2]);
    }
    channel.display_scale = pref.display_scale;
    channel.display_offset = pref.display_offset;
    channel.display_unit = pref.display_unit.clone();
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
            channel.display_scale = 1.0;
            channel.display_offset = 0.0;
            channel.display_unit = None;
            ui.close();
        }
        for preset in presets {
            let is_selected = (channel.display_scale - preset.scale).abs() < 1e-9
                && (channel.display_offset - preset.offset).abs() < 1e-9
                && channel.display_unit.as_deref() == Some(preset.unit);
            if ui.selectable_label(is_selected, preset.label).clicked() {
                channel.display_scale = preset.scale;
                channel.display_offset = preset.offset;
                channel.display_unit = Some(preset.unit.to_string());
                ui.close();
            }
        }
    }

    let pref_key = channel_preference_key(&name);
    let has_global_pref = shared.channel_preferences.contains_key(&pref_key);
    ui.separator();
    if ui
        .button("Save current display as global default")
        .clicked()
    {
        shared.channel_preferences.insert(
            pref_key.clone(),
            ChannelPreference {
                color: None,
                display_scale: channel.display_scale,
                display_offset: channel.display_offset,
                display_unit: channel.display_unit.clone(),
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

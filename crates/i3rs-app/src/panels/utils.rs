//! Shared utilities used by multiple panel types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::state::{
    CHANNEL_COLORS, ChannelId, PlottedChannel, PlottedChannelInfo, SharedState, YAxis,
    compute_channel_stats,
};

pub type ChannelDisplayMeta = (String, String, u16, i16, Arc<HashMap<i64, String>>);

static EMPTY_ENUM_LABELS: LazyLock<Arc<HashMap<i64, String>>> =
    LazyLock::new(|| Arc::new(HashMap::new()));

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
    Some(PlottedChannel {
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
    })
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

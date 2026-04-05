//! Shared utilities used by multiple panel types.

use std::sync::Arc;

use crate::state::{
    CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState, YAxis, compute_channel_stats,
};

/// Resolve channel metadata (name, unit, freq, dec_places) from a ChannelId.
pub fn resolve_channel_meta(id: ChannelId, shared: &SharedState) -> (String, String, u16, i16) {
    match id {
        ChannelId::Physical(idx) => {
            if let Some(ld) = &shared.ld_file
                && let Some(ch) = ld.channels.get(idx)
            {
                (ch.name.clone(), ch.unit.clone(), ch.freq, ch.dec_places)
            } else {
                ("???".into(), String::new(), 0, 0)
            }
        }
        ChannelId::Math(idx) => {
            if let Some(mc) = shared.math_channels.get(idx) {
                (mc.name.clone(), mc.unit.clone(), mc.freq, mc.dec_places)
            } else {
                ("???".into(), String::new(), 0, 0)
            }
        }
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
        y_axis: YAxis::Left,
        cached_min: min,
        cached_max: max,
        cached_avg: avg,
    })
}

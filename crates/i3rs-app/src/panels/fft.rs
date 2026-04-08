//! FFT panel: frequency spectrum analysis for vibration diagnosis.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use i3rs_core::{FftPlanner, compute_fft_with_planner};

use crate::state::{ChannelId, PlottedChannel, SharedState};

use super::utils::{build_plotted_channel_info, create_plotted_channel, resolve_channel_meta};

/// Return the sub-slice of data visible in the current zoom range (no copy).
fn visible_subslice<'a>(data: &'a [f64], freq: u16, shared: &SharedState) -> &'a [f64] {
    if let Some((t0, t1)) = shared.zoom_range {
        let start = (t0 * freq as f64).floor() as usize;
        let end = (t1 * freq as f64).ceil() as usize;
        &data[start.min(data.len())..end.min(data.len())]
    } else {
        data
    }
}

/// Cached FFT result.
struct FftCache {
    /// Fingerprint: (data pointer, data length, zoom range, channel_id).
    fingerprint: (usize, usize, Option<(u64, u64)>, ChannelId),
    frequencies: Vec<f64>,
    magnitudes: Vec<f64>,
}

pub struct FftPanel {
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    /// Log scale for Y axis.
    pub log_scale: bool,
    /// Cached FFT results per channel.
    caches: Vec<Option<FftCache>>,
    /// Reusable FFT planner (caches algorithm selection per data length).
    planner: FftPlanner<f64>,
}

impl FftPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            channels: Vec::new(),
            log_scale: false,
            caches: Vec::new(),
            planner: FftPlanner::new(),
        }
    }

    pub fn clear_channels(&mut self) {
        self.channels.clear();
        self.caches.clear();
    }

    fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self.channels.iter().any(|c| c.channel_id == channel_id) {
            return;
        }
        if let Some(pc) = create_plotted_channel(channel_id, shared, self.channels.len()) {
            self.channels.push(pc);
            self.caches.push(None);
        }
    }

    fn remove_channel(&mut self, channel_id: ChannelId) {
        if let Some(idx) = self
            .channels
            .iter()
            .position(|c| c.channel_id == channel_id)
        {
            self.channels.remove(idx);
            self.caches.remove(idx);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        // Handle drop from channel browser
        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
            && let Some(ch_id) = shared.dragging_channel.take()
        {
            self.add_channel(ch_id, shared);
        }

        // Handle pending toggle
        if let Some(ch_id) = shared.pending_toggle_channel.take() {
            if self.channels.iter().any(|c| c.channel_id == ch_id) {
                self.remove_channel(ch_id);
            } else {
                self.add_channel(ch_id, shared);
            }
        }

        if self.channels.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Drag a channel here for frequency analysis");
            });
            return;
        }

        // Register channels for readout
        for pc in &self.channels {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(
                    pc.channel_id,
                    pc.color,
                    pc.data.clone(),
                    shared,
                ));
        }

        // Toolbar
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.log_scale, "Log scale");

            let mut to_remove = None;
            for pc in &self.channels {
                let (name, _, _, _) = resolve_channel_meta(pc.channel_id, shared);
                let btn = egui::Button::new(&name).fill(pc.color.linear_multiply(0.3));
                let resp = ui.add(btn);
                if resp.secondary_clicked() {
                    to_remove = Some(pc.channel_id);
                }
                resp.on_hover_text("Right-click to remove");
            }
            if let Some(id) = to_remove {
                self.remove_channel(id);
            }
        });
        ui.separator();

        // Compute and plot FFT for each channel
        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let log_scale = self.log_scale;

        // Ensure caches vec matches channels vec
        while self.caches.len() < self.channels.len() {
            self.caches.push(None);
        }

        // Pre-compute FFT data outside the plot closure
        let mut fft_lines: Vec<(Vec<[f64; 2]>, egui::Color32, String)> = Vec::new();

        for (i, pc) in self.channels.iter().enumerate() {
            let (name, _, freq, _) = resolve_channel_meta(pc.channel_id, shared);
            if freq == 0 {
                continue;
            }

            let ptr = Arc::as_ptr(&pc.data) as usize;
            let len = pc.data.len();
            let fingerprint = (ptr, len, zoom_key, pc.channel_id);

            let cache: &mut Option<FftCache> = &mut self.caches[i];
            let needs_recompute = cache.as_ref().is_none_or(|c| c.fingerprint != fingerprint);

            if needs_recompute {
                let data_slice = visible_subslice(&pc.data, freq, shared);
                let result = compute_fft_with_planner(data_slice, freq as f64, &mut self.planner);
                *cache = Some(FftCache {
                    fingerprint,
                    frequencies: result.frequencies,
                    magnitudes: result.magnitudes,
                });
            }

            if let Some(c) = cache.as_ref() {
                let points: Vec<[f64; 2]> = c
                    .frequencies
                    .iter()
                    .zip(c.magnitudes.iter())
                    .skip(1) // skip DC component
                    .map(|(&f, &m): (&f64, &f64)| {
                        let y = if log_scale {
                            (m.max(1e-12)).log10() * 20.0
                        } else {
                            m
                        };
                        [f, y]
                    })
                    .collect();

                fft_lines.push((points, pc.color, name));
            }
        }

        let y_label = if log_scale {
            "Magnitude (dB)"
        } else {
            "Magnitude"
        };

        Plot::new(format!("fft_{}", self.id))
            .legend(Legend::default())
            .x_axis_label("Frequency (Hz)")
            .y_axis_label(y_label)
            .allow_boxed_zoom(true)
            .show(ui, |plot_ui| {
                for (points, color, name) in &fft_lines {
                    plot_ui.line(Line::new(name, PlotPoints::new(points.clone())).color(*color));
                }
            });
    }
}

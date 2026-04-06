//! Histogram panel: distribution of channel values with per-lap breakdown.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot};

use crate::state::{CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState};

use super::utils::{
    build_plotted_channel_info, create_plotted_channel, get_visible_slice, interp_at_time,
    resolve_channel_meta,
};

/// Cached histogram bin counts for a single channel.
struct HistogramBins {
    min: f64,
    bin_width: f64,
    counts: Vec<u64>,
}

struct HistogramCache {
    /// (data_ptr, data_len, zoom_key, bin_count, per_lap, num_laps)
    fingerprint: (Vec<usize>, Option<(u64, u64)>, usize, bool, usize),
    /// One entry per chart: (bins, color, label)
    charts: Vec<(HistogramBins, egui::Color32, String)>,
}

pub struct HistogramPanel {
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    pub bin_count: usize,
    /// Whether to show per-lap breakdown (stacked bars).
    pub per_lap: bool,
    cache: Option<HistogramCache>,
}

impl HistogramPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            channels: Vec::new(),
            bin_count: 50,
            per_lap: false,
            cache: None,
        }
    }

    pub fn clear_channels(&mut self) {
        self.channels.clear();
        self.cache = None;
    }

    fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self.channels.iter().any(|c| c.channel_id == channel_id) {
            return;
        }
        if let Some(pc) = create_plotted_channel(channel_id, shared, self.channels.len()) {
            self.channels.push(pc);
        }
    }

    fn remove_channel(&mut self, channel_id: ChannelId) {
        self.channels.retain(|c| c.channel_id != channel_id);
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

        // Handle pending toggle from browser
        if let Some(ch_id) = shared.pending_toggle_channel.take() {
            if self.channels.iter().any(|c| c.channel_id == ch_id) {
                self.remove_channel(ch_id);
            } else {
                self.add_channel(ch_id, shared);
            }
        }

        if self.channels.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Drag a channel here to see its distribution");
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
            ui.label("Bins:");
            ui.add(
                egui::DragValue::new(&mut self.bin_count)
                    .range(5..=500)
                    .speed(1),
            );
            ui.checkbox(&mut self.per_lap, "Per-lap breakdown");

            // Remove channel buttons
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

        // Plot histograms
        let plot = Plot::new(format!("histogram_{}", self.id))
            .legend(Legend::default())
            .allow_boxed_zoom(false)
            .x_axis_label("Value")
            .y_axis_label("Count");

        // Show cursor line if we have a cursor time
        let cursor_values: Vec<(String, f64, egui::Color32)> =
            if let Some(cursor_time) = shared.cursor_time {
                self.channels
                    .iter()
                    .filter_map(|pc| {
                        let (name, _, freq, _) = resolve_channel_meta(pc.channel_id, shared);
                        let val = interp_at_time(&pc.data, freq, cursor_time)?;
                        Some((name, val, pc.color))
                    })
                    .collect()
            } else {
                Vec::new()
            };

        // Cache histogram computation
        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let data_ptrs: Vec<usize> = self
            .channels
            .iter()
            .map(|pc| Arc::as_ptr(&pc.data) as usize)
            .collect();
        let fingerprint = (
            data_ptrs,
            zoom_key,
            self.bin_count,
            self.per_lap,
            shared.laps.len(),
        );

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let mut charts = Vec::new();
            for (ch_idx, pc) in self.channels.iter().enumerate() {
                let (name, _, freq, _) = resolve_channel_meta(pc.channel_id, shared);

                if !self.per_lap || shared.laps.is_empty() {
                    let data_slice = get_visible_slice(&pc.data, freq, shared);
                    let bins = compute_histogram_bins(&data_slice, self.bin_count);
                    charts.push((bins, pc.color, name));
                } else {
                    for (lap_idx, lap) in shared.laps.iter().enumerate() {
                        let start = (lap.start_time * freq as f64).floor() as usize;
                        let end = (lap.end_time * freq as f64).ceil() as usize;
                        let start = start.min(pc.data.len());
                        let end = end.min(pc.data.len());
                        if start >= end {
                            continue;
                        }
                        let lap_data: Vec<f64> = pc.data[start..end]
                            .iter()
                            .copied()
                            .filter(|v| v.is_finite())
                            .collect();

                        let lap_color = CHANNEL_COLORS[(ch_idx + lap_idx) % CHANNEL_COLORS.len()];
                        let label = format!("{} — {}", name, lap.name);
                        let bins = compute_histogram_bins(&lap_data, self.bin_count);
                        charts.push((bins, lap_color, label));
                    }
                }
            }
            self.cache = Some(HistogramCache {
                fingerprint,
                charts,
            });
        }

        let cached = self.cache.as_ref().unwrap();

        plot.show(ui, |plot_ui| {
            for (bins, color, name) in &cached.charts {
                let bars = bins_to_bar_chart(bins, *color, name);
                plot_ui.bar_chart(bars);
            }

            for (name, val, color) in &cursor_values {
                plot_ui.vline(
                    egui_plot::VLine::new(format!("{}: {:.2}", name, val), *val)
                        .color(*color)
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
            }
        });
    }
}

fn compute_histogram_bins(data: &[f64], bin_count: usize) -> HistogramBins {
    if data.is_empty() {
        return HistogramBins {
            min: 0.0,
            bin_width: 1.0,
            counts: vec![],
        };
    }

    let min = data.iter().copied().fold(f64::MAX, f64::min);
    let max = data.iter().copied().fold(f64::MIN, f64::max);

    if (max - min).abs() < f64::EPSILON {
        return HistogramBins {
            min,
            bin_width: 1.0,
            counts: vec![data.len() as u64],
        };
    }

    let bin_width = (max - min) / bin_count as f64;
    let mut counts = vec![0u64; bin_count];

    for &v in data {
        let idx = ((v - min) / bin_width) as usize;
        let idx = idx.min(bin_count - 1);
        counts[idx] += 1;
    }

    HistogramBins {
        min,
        bin_width,
        counts,
    }
}

fn bins_to_bar_chart(bins: &HistogramBins, color: egui::Color32, name: &str) -> BarChart {
    let bars: Vec<Bar> = bins
        .counts
        .iter()
        .enumerate()
        .map(|(i, &count)| {
            let center = bins.min + (i as f64 + 0.5) * bins.bin_width;
            Bar::new(center, count as f64).width(bins.bin_width * 0.9)
        })
        .collect();

    BarChart::new(name, bars).color(color)
}

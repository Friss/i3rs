//! Histogram panel: distribution of channel values with per-lap breakdown.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot};

use crate::state::{CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState};

use super::utils::{
    build_plotted_channel_info, create_plotted_channel, display_transform_fingerprint,
    get_visible_slice, interp_at_time, resolve_plotted_channel_display_meta,
    segmented_channel_button, show_plotted_channel_display_menu, transform_channel_value,
};

/// Cached histogram bin counts for a single channel.
struct HistogramBins {
    min: f64,
    bin_width: f64,
    counts: Vec<u64>,
}

struct HistogramCache {
    /// (data_ptr, display_transform, zoom_key, bin_count, per_lap, num_laps)
    fingerprint: (
        Vec<(usize, (u64, u64, Option<String>))>,
        Option<(u64, u64)>,
        usize,
        bool,
        usize,
    ),
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
    pub lock_x_range: bool,
    pub x_min: f64,
    pub x_max: f64,
    pub lock_y_range: bool,
    pub y_min: f64,
    pub y_headroom_pct: f64,
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
            lock_x_range: false,
            x_min: 0.0,
            x_max: 1.0,
            lock_y_range: false,
            y_min: 0.0,
            y_headroom_pct: 10.0,
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
                .push(build_plotted_channel_info(pc, shared));
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
            ui.separator();
            ui.checkbox(&mut self.lock_x_range, "Lock X");
            if self.lock_x_range {
                ui.add(
                    egui::DragValue::new(&mut self.x_min)
                        .speed(10.0)
                        .prefix("min "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.x_max)
                        .speed(10.0)
                        .prefix("max "),
                );
            }
            ui.checkbox(&mut self.lock_y_range, "Lock Y");
            if self.lock_y_range {
                ui.add(
                    egui::DragValue::new(&mut self.y_min)
                        .speed(1.0)
                        .prefix("min "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.y_headroom_pct)
                        .speed(1.0)
                        .range(0.0..=1000.0)
                        .suffix("%"),
                );
            }

            // Remove channel buttons
            let mut to_remove = None;
            for pc in &mut self.channels {
                let (name, _, _, _, _) = resolve_plotted_channel_display_meta(pc, shared);
                let (resp, clear_clicked) = segmented_channel_button(
                    ui,
                    &name,
                    Some(pc.color.linear_multiply(0.3)),
                    "Remove channel",
                );
                resp.context_menu(|ui| {
                    if show_plotted_channel_display_menu(ui, pc, shared) {
                        to_remove = Some(pc.channel_id);
                    }
                });
                if clear_clicked {
                    to_remove = Some(pc.channel_id);
                }
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
        let cursor_values: Vec<(String, f64, egui::Color32)> = if let Some(cursor_time) =
            shared.cursor_time
        {
            self.channels
                .iter()
                .filter_map(|pc| {
                    let (name, _, freq, _, _) = resolve_plotted_channel_display_meta(pc, shared);
                    let val =
                        transform_channel_value(pc, interp_at_time(&pc.data, freq, cursor_time)?);
                    Some((name, val, pc.color))
                })
                .collect()
        } else {
            Vec::new()
        };

        // Cache histogram computation
        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let channel_fingerprints: Vec<(usize, (u64, u64, Option<String>))> = self
            .channels
            .iter()
            .map(|pc| {
                (
                    Arc::as_ptr(&pc.data) as usize,
                    display_transform_fingerprint(pc),
                )
            })
            .collect();
        let fingerprint = (
            channel_fingerprints,
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
                let (name, unit, freq, _, _) = resolve_plotted_channel_display_meta(pc, shared);
                let series_label = histogram_series_label(&name, &unit);

                if !self.per_lap || shared.laps.is_empty() {
                    let data_slice: Vec<f64> = get_visible_slice(&pc.data, freq, shared)
                        .into_iter()
                        .map(|value| transform_channel_value(pc, value))
                        .collect();
                    let bins = compute_histogram_bins(&data_slice, self.bin_count);
                    charts.push((bins, pc.color, series_label.clone()));
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
                            .map(|value| transform_channel_value(pc, value))
                            .collect();

                        let lap_color = CHANNEL_COLORS[(ch_idx + lap_idx) % CHANNEL_COLORS.len()];
                        let label = format!("{} — {}", series_label, lap.name);
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

        let locked_x_range = if self.lock_x_range && self.x_max > self.x_min {
            Some((self.x_min, self.x_max))
        } else {
            None
        };
        let locked_y_range = if self.lock_y_range {
            let max_count = cached
                .charts
                .iter()
                .flat_map(|(bins, _, _)| bins.counts.iter().copied())
                .max()
                .unwrap_or(0) as f64;
            let headroom = max_count * (self.y_headroom_pct / 100.0);
            Some((self.y_min, (max_count + headroom).max(self.y_min + 1.0)))
        } else {
            None
        };

        plot.show(ui, |plot_ui| {
            if let Some((x_min, x_max)) = locked_x_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }
            if let Some((y_min, y_max)) = locked_y_range {
                plot_ui.set_plot_bounds_y(y_min..=y_max);
            }
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

fn histogram_series_label(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({unit})")
    }
}

fn bins_to_bar_chart(bins: &HistogramBins, color: egui::Color32, name: &str) -> BarChart {
    let bars: Vec<Bar> = bins
        .counts
        .iter()
        .enumerate()
        .map(|(i, &count)| {
            let center = bins.min + (i as f64 + 0.5) * bins.bin_width;
            Bar::new(center, count as f64)
                .width(bins.bin_width * 0.9)
                .fill(color)
                .stroke(egui::Stroke::NONE)
        })
        .collect();

    BarChart::new(name, bars).color(color)
}

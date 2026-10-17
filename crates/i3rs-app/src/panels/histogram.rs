//! Histogram panel: distribution of channel values with per-lap breakdown.

use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot};

use crate::state::{CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState};

use super::utils::{
    DisplayTransformFingerprint, build_plotted_channel_info, channel_picker,
    create_plotted_channel, display_transform_fingerprint, interp_at_time, refresh_plotted_channel,
    resolve_plotted_channel_display_meta, sample_range, segmented_channel_button,
    show_plotted_channel_display_menu, transform_channel_value,
};

use super::distribution::ColorBands;

/// Cached histogram bin counts for a single channel.
struct HistogramBins {
    freq: f64,
    min: f64,
    bin_width: f64,
    counts: Vec<u64>,
    base: Vec<u64>,
    denominator: u64,
}

#[derive(PartialEq, Eq)]
struct HistogramChannelFingerprint {
    data_ptr: usize,
    display_transform: DisplayTransformFingerprint,
    color: [u8; 4],
}

impl HistogramChannelFingerprint {
    fn of(pc: &PlottedChannel) -> Self {
        Self {
            data_ptr: pc.data.as_ptr() as usize,
            display_transform: display_transform_fingerprint(pc),
            color: pc.color.to_array(),
        }
    }
}

#[derive(PartialEq, Eq)]
struct HistogramFingerprint {
    overlays: String,
    filter: String,
    channels: Vec<HistogramChannelFingerprint>,
    zoom_key: Option<(u64, u64)>,
    bin_count: usize,
    per_lap: bool,
    lap_ranges: Vec<(u64, u64)>,
    bounds: Option<(u64, u64)>,
    color_channel: Option<HistogramChannelFingerprint>,
    bands: (u64, u64, usize, bool),
}

struct HistogramCache {
    bounds: Vec<(f64, f64)>,
    fingerprint: HistogramFingerprint,
    /// One entry per chart: (bins, color, label)
    charts: Vec<(HistogramBins, egui::Color32, String)>,
}

pub struct HistogramPanel {
    pub band_presentation: super::distribution::BandPresentation,
    pub seconds: bool,
    pub bin_settings: HistogramBinSettings,
    pub filter: super::distribution::ComponentFilter,
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    pub bin_count: usize,
    /// Whether to show per-lap breakdown (stacked bars).
    pub per_lap: bool,
    pub percentage: bool,
    pub color_channel: Option<PlottedChannel>,
    pub color_bands: ColorBands,
    color_filter: String,
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
            band_presentation: Default::default(),
            seconds: false,
            bin_settings: Default::default(),
            filter: Default::default(),
            id,
            title: title.into(),
            channels: Vec::new(),
            bin_count: 50,
            per_lap: false,
            percentage: false,
            color_channel: None,
            color_bands: ColorBands::default(),
            color_filter: String::new(),
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
        self.color_channel = None;
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

        for channel in &mut self.channels {
            refresh_plotted_channel(channel, shared);
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

        if let Some(pc) = &mut self.color_channel {
            refresh_plotted_channel(pc, shared);
        }

        // Toolbar
        ui.horizontal_wrapped(|ui| {
            ui.label("Bins:");
            ui.add(
                egui::DragValue::new(&mut self.bin_count)
                    .range(5..=500)
                    .speed(1),
            );
            if ui
                .checkbox(&mut self.per_lap, "Per-lap breakdown")
                .changed()
                && self.per_lap
            {
                self.color_channel = None;
            }
            ui.checkbox(&mut self.percentage, "Percent");
            if ui.checkbox(&mut self.seconds, "Seconds").changed() && self.seconds {
                self.percentage = false;
            }
            if self.percentage {
                self.seconds = false;
            }
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
                        .prefix("Headroom ")
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
        ui.horizontal_wrapped(|ui| {
            ui.label("Color by:");
            let mut selected = self.color_channel.as_ref().map(|pc| pc.channel_id);
            if channel_picker(
                ui,
                ("hist_color", self.id),
                &mut selected,
                &mut self.color_filter,
                shared,
            ) {
                self.color_channel = selected.and_then(|id| create_plotted_channel(id, shared, 0));
                if self.color_channel.is_some() {
                    self.per_lap = false;
                }
            }
            if self.color_channel.is_some() {
                self.color_bands.controls(ui);
            }
            ui.label("Range follows selected lap / zoom");
        });
        ui.separator();

        self.band_presentation.controls(ui, self.color_bands);
        self.filter.controls(ui);
        let filter = self.filter.prepare(ui, shared);

        ui.horizontal_wrapped(|ui| {
            let mut manual = self.bin_settings.width.is_some();
            if ui.checkbox(&mut manual, "Bin width").changed() {
                self.bin_settings.width = manual.then_some(100.);
            }
            if let Some(width) = &mut self.bin_settings.width {
                ui.add(egui::DragValue::new(width).range(0.000001..=1e9));
            }
            ui.checkbox(&mut self.bin_settings.centered, "Centred X bins");
            for (policy, name) in [
                (OverflowPolicy::Exclude, "Exclude overflow"),
                (OverflowPolicy::Clamp, "Clamp overflow"),
                (OverflowPolicy::Separate, "Overflow bins"),
            ] {
                ui.selectable_value(&mut self.bin_settings.overflow, policy, name);
            }
        });
        // Plot histograms
        let plot = Plot::new(format!("histogram_{}", self.id))
            .legend(Legend::default())
            .allow_boxed_zoom(false)
            .x_axis_label("Value")
            .y_axis_label(if self.percentage {
                "Percent of samples"
            } else if self.seconds {
                "Seconds"
            } else {
                "Count"
            });

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

        // Locked X is a real bin range, not just a camera setting.
        let locked_x_range =
            (self.lock_x_range && self.x_max > self.x_min).then_some((self.x_min, self.x_max));

        // Cache histogram computation
        let (t0, t1) = shared.analysis_range();
        let slots: Vec<_> = self
            .channels
            .iter()
            .chain(self.color_channel.iter())
            .collect();
        let overlays = crate::comparison::overlay_populations(shared, &slots, &self.filter);
        let zoom_key = Some((t0.to_bits(), t1.to_bits()));
        let fingerprint = HistogramFingerprint {
            overlays: format!(
                "{:?}",
                overlays
                    .iter()
                    .map(|o| (
                        &o.label,
                        o.range,
                        &o.filter.trusted_ranges,
                        o.filter.constant,
                        o.filter.gate.as_ref().map(|g| (
                            g.data.as_ptr() as usize,
                            g.freq,
                            g.interpolation
                        )),
                        o.channels
                            .iter()
                            .map(|c| (
                                c.data.as_ptr() as usize,
                                c.freq,
                                c.display_scale,
                                c.display_offset,
                                c.interpolation
                            ))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            ),
            filter: format!(
                "{}:{:?}:{:?}",
                filter.key,
                self.bin_settings,
                self.channels
                    .iter()
                    .chain(self.color_channel.iter())
                    .map(|c| {
                        let i = build_plotted_channel_info(c, shared);
                        (i.freq, i.interpolation)
                    })
                    .collect::<Vec<_>>()
            ),
            channels: self
                .channels
                .iter()
                .map(HistogramChannelFingerprint::of)
                .collect(),
            zoom_key,
            bin_count: self.bin_count,
            per_lap: self.per_lap,
            lap_ranges: shared
                .laps
                .iter()
                .map(|l| (l.start_time.to_bits(), l.end_time.to_bits()))
                .collect(),
            bounds: locked_x_range.map(|(lo, hi)| (lo.to_bits(), hi.to_bits())),
            color_channel: self
                .color_channel
                .as_ref()
                .map(HistogramChannelFingerprint::of),
            bands: self.color_bands.key(),
        };

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let mut charts = Vec::new();
            let mut common_bounds = Vec::new();
            let color_meta = self
                .color_channel
                .as_ref()
                .map(|c| resolve_plotted_channel_display_meta(c, shared));
            let color_policy = self
                .color_channel
                .as_ref()
                .map(|c| build_plotted_channel_info(c, shared).interpolation)
                .unwrap_or_default();
            let bands = self.color_channel.as_ref().map(|_| self.color_bands);
            for (ch_idx, pc) in self.channels.iter().enumerate() {
                let (name, unit, freq, _, _) = resolve_plotted_channel_display_meta(pc, shared);
                let series_label = histogram_series_label(&name, &unit);

                let ranges: Vec<_> = if self.per_lap && !shared.laps.is_empty() {
                    shared
                        .laps
                        .iter()
                        .map(|l| (l.name.clone(), (l.start_time, l.end_time)))
                        .collect()
                } else {
                    vec![(String::new(), shared.analysis_range())]
                };
                // Share bin edges across the displayed ranges, excluding samples
                // outside them.
                let bounds = locked_x_range.unwrap_or_else(|| {
                    let main = ranges
                        .iter()
                        .flat_map(|(_, range)| sample_range(pc.data.len(), freq, *range))
                        .filter(|i| filter.includes(*i as f64 / freq))
                        .map(|i| transform_channel_value(pc, pc.data[i]));
                    let comparison = overlays.iter().flat_map(|overlay| {
                        let info = &overlay.channels[ch_idx];
                        sample_range(info.data.len(), info.freq, overlay.range)
                            .filter(move |i| overlay.filter.includes(*i as f64 / info.freq))
                            .map(move |i| info.transform_value(info.data[i]))
                    });
                    finite_bounds(main.chain(comparison))
                });
                common_bounds.push(bounds);
                for (lap_idx, (lap_name, range)) in ranges.into_iter().enumerate() {
                    let samples: Vec<_> = sample_range(pc.data.len(), freq, range)
                        .filter(|i| filter.includes(*i as f64 / freq))
                        .map(|i| {
                            let value = transform_channel_value(pc, pc.data[i]);
                            let band_value = self
                                .color_channel
                                .as_ref()
                                .zip(color_meta.as_ref())
                                .and_then(|(c, meta)| {
                                    i3rs_core::channels::sample_with_policy(
                                        &c.data,
                                        meta.2,
                                        i as f64 / freq,
                                        color_policy,
                                    )
                                    .map(|v| transform_channel_value(c, v))
                                });
                            (value, band_value.unwrap_or(f64::NAN))
                        })
                        .collect();
                    for (band_idx, mut bins) in histogram_bins_configured(
                        &samples,
                        self.bin_count,
                        bounds,
                        bands,
                        self.bin_settings,
                    )
                    .into_iter()
                    .enumerate()
                    {
                        bins.freq = freq;
                        let count: u64 = bins.counts.iter().sum();
                        let (color, label) = if let Some(bands) = bands {
                            (
                                bands.color(band_idx),
                                format!(
                                    "{} · {} {}: {} samples",
                                    series_label,
                                    bands.label(band_idx),
                                    color_meta.as_ref().map_or("", |m| m.1.as_str()),
                                    count
                                ),
                            )
                        } else if self.per_lap {
                            (
                                CHANNEL_COLORS[(ch_idx + lap_idx) % CHANNEL_COLORS.len()],
                                format!("{} · {}: {} samples", series_label, lap_name, count),
                            )
                        } else {
                            (pc.color, format!("{}: {} samples", series_label, count))
                        };
                        charts.push((bins, color, label));
                    }
                }
            }
            self.cache = Some(HistogramCache {
                bounds: common_bounds,
                fingerprint,
                charts,
            });
        }

        let mut overlay_charts = Vec::new();
        for overlay in &overlays {
            for (i, info) in overlay
                .channels
                .iter()
                .take(self.channels.len())
                .enumerate()
            {
                let bounds = self.cache.as_ref().unwrap().bounds[i];
                let samples: Vec<_> = sample_range(info.data.len(), info.freq, overlay.range)
                    .filter(|i| overlay.filter.includes(*i as f64 / info.freq))
                    .map(|i| {
                        let color = overlay
                            .channels
                            .get(self.channels.len())
                            .and_then(|c| {
                                i3rs_core::channels::sample_with_policy(
                                    &c.data,
                                    c.freq,
                                    i as f64 / info.freq,
                                    c.interpolation,
                                )
                                .map(|v| c.transform_value(v))
                            })
                            .unwrap_or(f64::NAN);
                        (info.transform_value(info.data[i]), color)
                    })
                    .collect();
                for (band, mut bins) in histogram_bins_configured(
                    &samples,
                    self.bin_count,
                    bounds,
                    self.color_channel.as_ref().map(|_| self.color_bands),
                    self.bin_settings,
                )
                .into_iter()
                .enumerate()
                {
                    bins.freq = info.freq;
                    let count: u64 = bins.counts.iter().sum();
                    overlay_charts.push((
                        bins,
                        overlay.color,
                        format!(
                            "{} · {} · {} · {} samples",
                            overlay.label,
                            info.name,
                            if self.color_channel.is_some() {
                                self.color_bands.label(band)
                            } else {
                                String::new()
                            },
                            count
                        ),
                        band,
                    ));
                }
            }
        }
        if !shared.comparisons.overlays.is_empty() {
            ui.weak(format!("{} comparison populations ready", overlays.len()));
            if shared
                .comparisons
                .sessions
                .iter()
                .any(|s| !s.requests.is_empty())
            {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let cached = self.cache.as_ref().unwrap();

        let percentage = self.percentage;
        let seconds = self.seconds;
        let sample_count: u64 = cached.charts.iter().flat_map(|(b, _, _)| &b.counts).sum();
        ui.label(format!("{sample_count} samples"));
        let locked_y_range = if self.lock_y_range {
            let max_count = cached
                .charts
                .iter()
                .flat_map(|(bins, _, _)| {
                    bins.counts.iter().zip(&bins.base).map(move |(n, base)| {
                        histogram_height(n + base, bins.denominator, percentage)
                            / if seconds {
                                if bins.freq > 0. { bins.freq } else { 1. }
                            } else {
                                1.
                            }
                    })
                })
                .fold(0.0_f64, f64::max);
            let headroom = max_count * (self.y_headroom_pct / 100.0);
            Some((self.y_min, (max_count + headroom).max(self.y_min + 1.0)))
        } else {
            None
        };

        let mut csv =
            String::from("Series,Bin start,Bin end,Count,Denominator,Value,Base,Output\n");
        for (index, (bins, _, label)) in cached.charts.iter().enumerate() {
            if self.color_channel.is_none()
                || self
                    .band_presentation
                    .visible(index % self.color_bands.len())
            {
                append_histogram_csv(&mut csv, bins, label, percentage, seconds);
            }
        }
        for (bins, _, label, band) in &overlay_charts {
            if self.color_channel.is_none() || self.band_presentation.visible(*band) {
                append_histogram_csv(&mut csv, bins, label, percentage, seconds);
            }
        }
        ui.horizontal_wrapped(|ui| crate::output_ui::result_buttons(ui, &csv));

        let plot_response = plot.show(ui, |plot_ui| {
            if let Some((x_min, x_max)) = locked_x_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }
            if let Some((y_min, y_max)) = locked_y_range {
                plot_ui.set_plot_bounds_y(y_min..=y_max);
            }
            for (index, (bins, color, name)) in cached.charts.iter().enumerate() {
                if self.color_channel.is_some()
                    && !self
                        .band_presentation
                        .visible(index % self.color_bands.len())
                {
                    continue;
                }
                let color = if self.color_channel.is_some() {
                    self.band_presentation
                        .color(self.color_bands, index % self.color_bands.len())
                } else {
                    *color
                };
                let bars = bins_to_bar_chart(bins, color, name, self.percentage, self.seconds);
                plot_ui.bar_chart(bars);
            }

            for (bins, color, label, band) in &overlay_charts {
                if self.color_channel.is_some() && !self.band_presentation.visible(*band) {
                    continue;
                }
                let color = if self.color_channel.is_some() {
                    self.band_presentation
                        .color(self.color_bands, *band)
                        .gamma_multiply(0.5)
                } else {
                    *color
                };
                plot_ui.bar_chart(bins_to_bar_chart(
                    bins,
                    color,
                    label,
                    self.percentage,
                    self.seconds,
                ));
            }
            for (name, val, color) in &cursor_values {
                plot_ui.vline(
                    egui_plot::VLine::new(format!("{}: {:.2}", name, val), *val)
                        .color(*color)
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
            }
        });

        plot_response.response.context_menu(|ui| {
            ui.label("Channels");
            ui.separator();

            let mut to_remove = None;
            for pc in &mut self.channels {
                let (name, _, _, _, _) = resolve_plotted_channel_display_meta(pc, shared);
                ui.menu_button(name, |ui| {
                    if show_plotted_channel_display_menu(ui, pc, shared) {
                        to_remove = Some(pc.channel_id);
                    }
                });
            }

            if let Some(id) = to_remove {
                self.remove_channel(id);
                self.cache = None;
                ui.close();
            }
        });
    }
}

#[cfg(test)]
fn selected_bounds(
    data: &[f64],
    freq: f64,
    ranges: impl Iterator<Item = (f64, f64)>,
    scale: f64,
    offset: f64,
) -> (f64, f64) {
    finite_bounds(ranges.flat_map(|range| {
        sample_range(data.len(), freq, range).map(move |i| data[i] * scale + offset)
    }))
}

fn finite_bounds(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in values.filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        return (0.0, 1.0);
    }
    if hi <= lo {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    }
}

#[cfg(test)]
fn histogram_bins(
    samples: &[(f64, f64)],
    count: usize,
    bounds: (f64, f64),
    bands: Option<ColorBands>,
) -> Vec<HistogramBins> {
    histogram_bins_configured(
        samples,
        count,
        bounds,
        bands,
        HistogramBinSettings::default(),
    )
}
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum OverflowPolicy {
    #[default]
    Exclude,
    Clamp,
    Separate,
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HistogramBinSettings {
    pub width: Option<f64>,
    pub centered: bool,
    pub overflow: OverflowPolicy,
}
fn histogram_bins_configured(
    samples: &[(f64, f64)],
    count: usize,
    bounds: (f64, f64),
    bands: Option<ColorBands>,
    settings: HistogramBinSettings,
) -> Vec<HistogramBins> {
    let requested_width = settings.width.filter(|w| w.is_finite() && *w > 0.);
    let count = requested_width
        .map_or(count, |w| {
            ((bounds.1 - bounds.0) / w).ceil() as usize + usize::from(settings.centered)
        })
        .clamp(1, 500);
    let width = requested_width.unwrap_or(
        (bounds.1 - bounds.0)
            / if settings.centered {
                count.saturating_sub(1).max(1)
            } else {
                count
            } as f64,
    );
    let lower = bounds.0 - if settings.centered { width / 2. } else { 0. };
    let bounds = (lower, lower + width * count as f64);
    let extra = if settings.overflow == OverflowPolicy::Separate {
        2
    } else {
        0
    };
    let count = count.clamp(1, 500);
    let width = (bounds.1 - bounds.0) / count as f64;
    let mut result: Vec<_> = (0..bands.map_or(1, ColorBands::len))
        .map(|_| HistogramBins {
            freq: 1.0,
            min: bounds.0 - if extra > 0 { width } else { 0. },
            bin_width: width,
            counts: vec![0; count + extra],
            base: vec![0; count + extra],
            denominator: 0,
        })
        .collect();
    if !width.is_finite() || width <= 0.0 {
        return result;
    }
    let mut total = 0;
    for &(value, color) in samples {
        if !value.is_finite()
            || (settings.overflow == OverflowPolicy::Exclude
                && (value < bounds.0 || value > bounds.1))
        {
            continue;
        }
        let bin = if extra > 0 && value < bounds.0 {
            0
        } else if extra > 0 && value > bounds.1 {
            count + 1
        } else {
            (((value - bounds.0) / width).floor().max(0.) as usize).min(count - 1) + extra / 2
        };
        let band = bands.map_or(0, |b| b.index(color));
        result[band].counts[bin] += 1;
        total += 1;
    }
    let mut base = vec![0; count + extra];
    for bins in &mut result {
        bins.denominator = total;
        bins.base.clone_from(&base);
        for (n, b) in bins.counts.iter().zip(&mut base) {
            *b += n;
        }
    }
    result
}

fn histogram_height(count: u64, total: u64, percentage: bool) -> f64 {
    if percentage {
        100.0 * count as f64 / total.max(1) as f64
    } else {
        count as f64
    }
}

fn histogram_series_label(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({unit})")
    }
}

fn append_histogram_csv(
    csv: &mut String,
    bins: &HistogramBins,
    name: &str,
    percentage: bool,
    seconds: bool,
) {
    use std::fmt::Write;
    for (i, count) in bins.counts.iter().enumerate() {
        let divisor = if seconds {
            if bins.freq > 0. { bins.freq } else { 1. }
        } else {
            1.
        };
        let value = histogram_height(*count, bins.denominator, percentage) / divisor;
        let base = histogram_height(bins.base[i], bins.denominator, percentage) / divisor;
        writeln!(
            csv,
            "{},{},{},{},{},{},{},{}",
            i3rs_core::output::csv_cell(name),
            bins.min + i as f64 * bins.bin_width,
            bins.min + (i + 1) as f64 * bins.bin_width,
            count,
            bins.denominator,
            value,
            base,
            if seconds {
                "s"
            } else if percentage {
                "%"
            } else {
                "samples"
            }
        )
        .unwrap();
    }
}

fn bins_to_bar_chart(
    bins: &HistogramBins,
    color: egui::Color32,
    name: &str,
    percentage: bool,
    seconds: bool,
) -> BarChart {
    let bars: Vec<Bar> = bins
        .counts
        .iter()
        .enumerate()
        .map(|(i, &count)| {
            let center = bins.min + (i as f64 + 0.5) * bins.bin_width;
            Bar::new(
                center,
                histogram_height(count, bins.denominator, percentage)
                    / if seconds {
                        if bins.freq > 0. { bins.freq } else { 1. }
                    } else {
                        1.
                    },
            )
            .base_offset(
                histogram_height(bins.base[i], bins.denominator, percentage)
                    / if seconds {
                        if bins.freq > 0. { bins.freq } else { 1. }
                    } else {
                        1.
                    },
            )
            .width(bins.bin_width * 0.9)
            .fill(color)
            .stroke(egui::Stroke::NONE)
        })
        .collect();

    BarChart::new(name, bars).color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the local May 8 S1 reference log"]
    fn matches_live_i2_lap_histograms() {
        // i2 Standard 1.1.4.457, RPM Histo, observed 2026-10-14:
        // Lap 2 is 59 ms, with one sample in 4900–5000 rpm / 90–100% throttle.
        let ld = i3rs_core::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        ))
        .unwrap();
        let laps = i3rs_core::detect_laps(&ld);
        let lap = &laps[2];
        assert!((lap.duration() - 0.059).abs() < 1e-9);
        let rpm = ld.find_channel_by_name("Engine Speed").unwrap();
        let throttle = ld.find_channel_by_name("Throttle Position").unwrap();
        let rpm_data = ld.read_channel_data(rpm).unwrap();
        let throttle_data = ld.read_channel_data(throttle).unwrap();
        let samples: Vec<_> =
            sample_range(rpm_data.len(), rpm.freq, (lap.start_time, lap.end_time))
                .map(|i| {
                    (
                        rpm_data[i],
                        interp_at_time(&throttle_data, throttle.freq, i as f64 / rpm.freq).unwrap(),
                    )
                })
                .collect();
        let bins = histogram_bins(
            &samples,
            90,
            (0.0, 9000.0),
            Some(ColorBands {
                count: 10,
                centered: false,
                ..ColorBands::default()
            }),
        );
        assert_eq!(bins.iter().flat_map(|b| &b.counts).sum::<u64>(), 1);
        assert_eq!(bins[10].counts[49], 1);
        assert_eq!(histogram_height(1, bins[10].denominator, true), 100.0);

        // Lap 3: the ten band totals are transcribed from the live i2 legend.
        // Its top band includes the 566 native samples decoded as exactly 100%.
        let lap = &laps[3];
        assert!((lap.duration() - 134.114).abs() < 1e-9);
        let samples: Vec<_> =
            sample_range(rpm_data.len(), rpm.freq, (lap.start_time, lap.end_time))
                .map(|i| {
                    (
                        rpm_data[i],
                        interp_at_time(&throttle_data, throttle.freq, i as f64 / rpm.freq).unwrap(),
                    )
                })
                .collect();
        let bins = histogram_bins(
            &samples,
            90,
            (0.0, 9000.0),
            Some(ColorBands {
                count: 10,
                centered: false,
                ..ColorBands::default()
            }),
        );
        assert_eq!(samples.len(), 2682);
        let totals: Vec<u64> = bins.iter().map(|b| b.counts.iter().sum()).collect();
        assert_eq!(
            totals,
            [0, 1037, 204, 94, 59, 36, 29, 16, 15, 23, 1169, 0, 0]
        );
    }

    #[test]
    fn rpm_and_throttle_boundaries_use_100_rpm_bins_and_ten_noncentered_bands() {
        let bands = ColorBands {
            count: 10,
            centered: false,
            ..ColorBands::default()
        };
        let samples = [
            (0.0, -0.1),
            (99.9, 0.0),
            (100.0, 9.9),
            (199.9, 10.0),
            (200.0, 99.9),
            (8999.9, 100.0),
            (9000.0, 100.1),
            (-0.1, 50.0),
            (9000.1, 50.0),
            (500.0, f64::NAN),
        ];
        let bins = histogram_bins(&samples, 90, (0.0, 9000.0), Some(bands));
        assert_eq!(bins.len(), 13); // Underflow, 10 bands, overflow, missing.
        assert_eq!(bins[0].bin_width, 100.0);
        assert_eq!(bins[0].counts[0], 1);
        assert_eq!(bins[1].counts[0], 1);
        assert_eq!(bins[1].counts[1], 1);
        assert_eq!(bins[2].counts[1], 1);
        assert_eq!(bins[10].counts[2], 1);
        assert_eq!(bins[10].counts[89], 1);
        assert_eq!(bins[11].counts[89], 1);
        assert_eq!(bins[12].counts[5], 1);
        assert_eq!(bins[0].denominator, 8);
        let percentage: f64 = bins
            .iter()
            .flat_map(|b| {
                b.counts
                    .iter()
                    .map(|n| histogram_height(*n, b.denominator, true))
            })
            .sum();
        assert!((percentage - 100.0).abs() < 1e-9);
    }

    #[test]
    fn automatic_bounds_follow_selected_samples_and_share_edges_across_laps() {
        let data = [-1000.0, 1.0, 2.0, f64::NAN, 1000.0, 3.0, 4.0, 2000.0];
        let bounds = selected_bounds(&data, 1.0, [(1.0, 3.0)].into_iter(), 2.0, 10.0);
        assert_eq!(bounds, (12.0, 14.0));
        let bounds = selected_bounds(&data, 1.0, [(1.0, 4.0), (5.0, 7.0)].into_iter(), 2.0, 10.0);
        assert_eq!(bounds, (12.0, 18.0));
        let first = histogram_bins(&[(12.0, 0.0), (14.0, 0.0)], 2, bounds, None);
        let second = histogram_bins(&[(16.0, 0.0), (18.0, 0.0)], 2, bounds, None);
        assert_eq!(first[0].counts, [2, 0]);
        assert_eq!(second[0].counts, [0, 2]);
        assert_eq!(
            selected_bounds(&data, 1.0, [(3.0, 4.0)].into_iter(), 1.0, 0.0),
            (0.0, 1.0)
        );
    }

    #[test]
    fn shared_bin_edges_and_percentages_preserve_counts() {
        let samples = [
            (0.0, 0.0),
            (5.0, 50.0),
            (10.0, 100.0),
            (f64::NAN, 30.0),
            (-1.0, 0.0),
            (11.0, 0.0),
            (5.0, f64::NAN),
        ];
        let bins = histogram_bins(&samples, 2, (0.0, 10.0), Some(ColorBands::default()));
        assert_eq!(bins.iter().flat_map(|b| &b.counts).sum::<u64>(), 4);
        let total_percent: f64 = bins
            .iter()
            .flat_map(|b| {
                b.counts
                    .iter()
                    .map(|n| histogram_height(*n, b.denominator, true))
            })
            .sum();
        assert!((total_percent - 100.0).abs() < 1e-9);
        assert_eq!(bins.last().unwrap().counts, [0, 1]);
        let lap_a = histogram_bins(&[(1.0, 0.0)], 2, (0.0, 10.0), None);
        let lap_b = histogram_bins(&[(9.0, 0.0)], 2, (0.0, 10.0), None);
        assert_eq!(lap_a[0].counts, [1, 0]);
        assert_eq!(lap_b[0].counts, [0, 1]);
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn width_center_overflow_and_seconds_preserve_population() {
        let samples = [
            (-6., 0.),
            (-5., 0.),
            (0., 0.),
            (5., 0.),
            (10., 0.),
            (15., 0.),
            (16., 0.),
            (f64::NAN, 0.),
        ];
        let settings = HistogramBinSettings {
            width: Some(10.),
            centered: true,
            overflow: OverflowPolicy::Separate,
        };
        let mut b = histogram_bins_configured(&samples, 20, (0., 10.), None, settings).remove(0);
        assert_eq!(b.counts, [1, 2, 3, 1]);
        assert_eq!(b.denominator, 7);
        b.freq = 2.0;
        assert_eq!(b.counts.iter().sum::<u64>() as f64 / b.freq, 3.5);
        let percent: f64 = b
            .counts
            .iter()
            .map(|n| histogram_height(*n, b.denominator, true))
            .sum();
        assert!((percent - 100.).abs() < 1e-10);
        let b = histogram_bins_configured(
            &samples,
            20,
            (0., 10.),
            None,
            HistogramBinSettings {
                overflow: OverflowPolicy::Clamp,
                ..settings
            },
        )
        .remove(0);
        assert_eq!(b.counts, [3, 4]);
        let b = histogram_bins_configured(
            &samples,
            20,
            (0., 10.),
            None,
            HistogramBinSettings {
                overflow: OverflowPolicy::Exclude,
                ..settings
            },
        )
        .remove(0);
        assert_eq!(b.counts, [2, 3]);
        assert_eq!(b.denominator, 5);
    }
}

#[cfg(test)]
mod output_contract_tests {
    use super::*;
    #[test]
    fn exported_heights_and_stacking_match_chart_percentage_and_sub_hz_seconds() {
        let bins = HistogramBins {
            freq: 0.5,
            min: 10.,
            bin_width: 2.,
            counts: vec![2, 3],
            base: vec![1, 2],
            denominator: 10,
        };
        let mut csv = String::new();
        append_histogram_csv(&mut csv, &bins, "A, B", true, false);
        assert!(csv.contains("\"A, B\",10,12,2,10,20,10,%"));
        let mut csv = String::new();
        append_histogram_csv(&mut csv, &bins, "A", false, true);
        assert!(csv.contains("\"A\",10,12,2,10,4,2,s"));
    }
}

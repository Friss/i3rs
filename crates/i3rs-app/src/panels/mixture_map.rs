//! Mixture map panel: 2D heatmap (e.g., AFR vs RPM vs TPS).
//!
//! User drops three channels: X axis, Y axis, and value (color).
//! The panel bins the data into a 2D grid and colors each cell by the
//! average value channel reading in that bin.

use super::distribution::ColorBands;
use eframe::egui;

use crate::state::{ChannelId, PlottedChannel, SharedState};

use super::utils::{
    DisplayTransformFingerprint, build_plotted_channel_info, create_plotted_channel,
    display_transform_fingerprint, interp_at_time, refresh_plotted_channel,
    resolve_plotted_channel_display_meta, segmented_channel_button,
    show_plotted_channel_display_menu,
};

#[derive(PartialEq, Eq)]
struct HeatmapFingerprint {
    x_data_ptr: usize,
    y_data_ptr: usize,
    value_data_ptr: usize,
    zoom_key: Option<(u64, u64)>,
    bins: usize,
    delay: u64,
    minimum_y: Option<u64>,
    frequencies: [u16; 3],
    x_transform: DisplayTransformFingerprint,
    y_transform: DisplayTransformFingerprint,
    value_transform: DisplayTransformFingerprint,
}

struct HeatmapCache {
    fingerprint: HeatmapFingerprint,
    heatmap: Heatmap,
}

pub struct MixtureMapPanel {
    pub id: u64,
    pub title: String,
    /// X-axis channel.
    pub x_channel: Option<PlottedChannel>,
    /// Y-axis channel.
    pub y_channel: Option<PlottedChannel>,
    /// Value (color) channel.
    pub value_channel: Option<PlottedChannel>,
    /// Number of bins along each axis.
    pub bins: usize,
    pub scatter_mode: bool,
    /// Positive delay aligns Y(t + delay) with X(t) and load(t).
    pub sensor_delay: f64,
    pub minimum_y: Option<f64>,
    pub color_bands: ColorBands,
    cache: Option<Box<HeatmapCache>>,
}

impl MixtureMapPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            x_channel: None,
            y_channel: None,
            value_channel: None,
            bins: 20,
            scatter_mode: false,
            sensor_delay: 0.0,
            minimum_y: None,
            color_bands: ColorBands::default(),
            cache: None,
        }
    }

    pub fn clear_channels(&mut self) {
        self.x_channel = None;
        self.y_channel = None;
        self.value_channel = None;
        self.cache = None;
    }

    fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if let Some(pc) = create_plotted_channel(channel_id, shared, 0) {
            if self.x_channel.is_none() {
                self.x_channel = Some(pc);
            } else if self.y_channel.is_none() {
                self.y_channel = Some(pc);
            } else if self.value_channel.is_none()
                || self
                    .value_channel
                    .as_ref()
                    .is_some_and(|v| v.channel_id != channel_id)
            {
                self.value_channel = Some(pc);
            }
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
            self.add_channel(ch_id, shared);
        }

        if let Some(channel) = self.x_channel.as_mut() {
            refresh_plotted_channel(channel, shared);
        }
        if let Some(channel) = self.y_channel.as_mut() {
            refresh_plotted_channel(channel, shared);
        }
        if let Some(channel) = self.value_channel.as_mut() {
            refresh_plotted_channel(channel, shared);
        }

        // Toolbar
        ui.horizontal_wrapped(|ui| {
            ui.label("X:");
            let mut clear_x = false;
            if let Some(channel) = self.x_channel.as_mut() {
                let (x_name, _, _, _, _) = resolve_plotted_channel_display_meta(channel, shared);
                let (xr, clear_clicked) =
                    segmented_channel_button(ui, &x_name, None, "Clear X channel");
                xr.context_menu(|ui| {
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_x = true;
                    }
                });
                if clear_clicked {
                    clear_x = true;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Drop X"));
            }
            if clear_x {
                self.x_channel = None;
                self.cache = None;
            }

            ui.label("Y:");
            let mut clear_y = false;
            if let Some(channel) = self.y_channel.as_mut() {
                let (y_name, _, _, _, _) = resolve_plotted_channel_display_meta(channel, shared);
                let (yr, clear_clicked) =
                    segmented_channel_button(ui, &y_name, None, "Clear Y channel");
                yr.context_menu(|ui| {
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_y = true;
                    }
                });
                if clear_clicked {
                    clear_y = true;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Drop Y"));
            }
            if clear_y {
                self.y_channel = None;
                self.cache = None;
            }

            ui.label("Value:");
            let mut clear_value = false;
            if let Some(channel) = self.value_channel.as_mut() {
                let (value_name, _, _, _, _) =
                    resolve_plotted_channel_display_meta(channel, shared);
                let (vr, clear_clicked) =
                    segmented_channel_button(ui, &value_name, None, "Clear value channel");
                vr.context_menu(|ui| {
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_value = true;
                    }
                });
                if clear_clicked {
                    clear_value = true;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Drop Value"));
            }
            if clear_value {
                self.value_channel = None;
                self.cache = None;
            }

            ui.label("Bins:");
            ui.add(egui::DragValue::new(&mut self.bins).range(5..=100).speed(1));
        });
        ui.separator();

        // Register channels for readout
        for pc in self
            .x_channel
            .iter()
            .chain(self.y_channel.iter())
            .chain(self.value_channel.iter())
        {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(pc, shared));
        }

        let (Some(x_ch), Some(y_ch), Some(v_ch)) =
            (&self.x_channel, &self.y_channel, &self.value_channel)
        else {
            ui.centered_and_justified(|ui| {
                ui.label("Drop three channels: X axis, Y axis, then Value (color)");
            });
            return;
        };

        let (x_name, x_unit, x_freq, x_dec_places, _) =
            resolve_plotted_channel_display_meta(x_ch, shared);
        let (y_name, y_unit, y_freq, y_dec_places, _) =
            resolve_plotted_channel_display_meta(y_ch, shared);
        let (v_name, v_unit, v_freq, v_dec_places, _) =
            resolve_plotted_channel_display_meta(v_ch, shared);

        ui.horizontal_wrapped(|ui| {
            ui.label("Y sensor delay (s):");
            ui.add(
                egui::DragValue::new(&mut self.sensor_delay)
                    .range(0.0..=5.0)
                    .speed(0.01),
            );
            let mut filter = self.minimum_y.is_some();
            ui.checkbox(&mut filter, "Exclude Y below");
            if filter {
                ui.add(egui::DragValue::new(self.minimum_y.get_or_insert(0.5)).speed(0.01));
            } else {
                self.minimum_y = None;
            }
            if y_name.to_ascii_lowercase().contains("lambda") {
                ui.weak("Gear-change filtering is not applied");
            }
        });
        let target_freq = x_freq.min(y_freq).min(v_freq);
        if target_freq == 0 {
            ui.label("Channel frequency is 0");
            return;
        }

        let (t0, t1) = shared.analysis_range();

        let x_series = (
            &x_ch.data[..],
            x_freq,
            x_ch.display_scale,
            x_ch.display_offset,
        );
        let y_series = (
            &y_ch.data[..],
            y_freq,
            y_ch.display_scale,
            y_ch.display_offset,
        );
        let v_series = (
            &v_ch.data[..],
            v_freq,
            v_ch.display_scale,
            v_ch.display_offset,
        );

        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let fingerprint = HeatmapFingerprint {
            x_data_ptr: x_ch.data.as_ptr() as usize,
            y_data_ptr: y_ch.data.as_ptr() as usize,
            value_data_ptr: v_ch.data.as_ptr() as usize,
            zoom_key,
            bins: self.bins,
            delay: self.sensor_delay.to_bits(),
            minimum_y: self.minimum_y.map(f64::to_bits),
            frequencies: [x_freq, y_freq, v_freq],
            x_transform: display_transform_fingerprint(x_ch),
            y_transform: display_transform_fingerprint(y_ch),
            value_transform: display_transform_fingerprint(v_ch),
        };

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let y_axis = axis_config(&y_name, &y_unit, y_dec_places);
            let heatmap = compute_heatmap(
                x_series,
                y_series,
                v_series,
                target_freq,
                (t0, t1),
                self.bins,
                y_axis.fixed_range,
                self.sensor_delay,
                self.minimum_y,
            );
            self.cache = Some(Box::new(HeatmapCache {
                fingerprint,
                heatmap,
            }));
        }
        let heatmap = &self.cache.as_ref().unwrap().heatmap;

        let x_axis = axis_config(&x_name, &x_unit, x_dec_places);
        let y_axis = axis_config(&y_name, &y_unit, y_dec_places);

        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.scatter_mode, true, "Samples");
            ui.selectable_value(&mut self.scatter_mode, false, "Heatmap");
            ui.label(format!(
                "{} samples · range follows lap / zoom",
                heatmap.triples.len()
            ));
            if self.scatter_mode {
                self.color_bands.controls(ui);
            }
        });
        let available = ui.available_rect_before_wrap();
        let margin = 50.0;
        let plot_rect = egui::Rect::from_min_max(
            egui::pos2(available.min.x + margin, available.min.y + 24.0),
            egui::pos2(available.max.x - 145.0, available.max.y - margin),
        );

        if plot_rect.width() < 10.0 || plot_rect.height() < 10.0 {
            return;
        }

        let (response, painter) = ui.allocate_painter(
            egui::vec2(available.width(), available.height()),
            egui::Sense::click(),
        );

        let to_screen = |x: f64, y: f64| {
            egui::pos2(
                plot_rect.left()
                    + normalized_fraction(x, heatmap.x_min, heatmap.x_max) as f32
                        * plot_rect.width(),
                plot_rect.bottom()
                    - normalized_fraction(y, heatmap.y_min, heatmap.y_max) as f32
                        * plot_rect.height(),
            )
        };
        let mut band_counts = vec![0_usize; self.color_bands.len()];
        let cell_w = plot_rect.width() / self.bins as f32;
        let cell_h = plot_rect.height() / self.bins as f32;
        let legend_steps = legend_step_count(&v_unit, heatmap.value_range);

        if self.scatter_mode {
            for &(x, y, value, _) in &heatmap.triples {
                let band = self.color_bands.index(value);
                band_counts[band] += 1;
                if y < heatmap.y_min || y > heatmap.y_max {
                    continue;
                }
                painter.circle_filled(to_screen(x, y), 1.75, self.color_bands.color(band));
            }
        } else {
            for yi in 0..self.bins {
                for xi in 0..self.bins {
                    let cell = &heatmap.cells[yi * self.bins + xi];
                    if cell.count == 0 {
                        continue;
                    }
                    let avg_val = cell.sum / cell.count as f64;
                    let frac = if heatmap.value_range > 0.0 {
                        ((avg_val - heatmap.value_min) / heatmap.value_range).clamp(0.0, 1.0) as f32
                    } else {
                        0.5
                    };

                    let color = heat_color(step_fraction(frac, legend_steps))
                        .gamma_multiply(occupancy_alpha(cell.count, heatmap.max_count));
                    let cell_rect = egui::Rect::from_min_size(
                        egui::pos2(
                            plot_rect.min.x + xi as f32 * cell_w,
                            plot_rect.max.y - (yi + 1) as f32 * cell_h,
                        ),
                        egui::vec2(cell_w, cell_h),
                    );
                    painter.rect_filled(cell_rect, 0.0, color);
                }
            }
        }
        painter.rect_stroke(
            plot_rect,
            0.0,
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(80)),
            egui::StrokeKind::Outside,
        );

        for i in 0..x_axis.tick_count {
            let frac = axis_fraction(i, x_axis.tick_count);
            let val = heatmap.x_min + frac * (heatmap.x_max - heatmap.x_min);
            let x = plot_rect.min.x + frac as f32 * plot_rect.width();
            painter.text(
                egui::pos2(x, plot_rect.max.y + 4.0),
                egui::Align2::CENTER_TOP,
                format_axis_value(val, x_axis.dec_places),
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }

        for i in 0..y_axis.tick_count {
            let frac = axis_fraction(i, y_axis.tick_count);
            let val = heatmap.y_min + frac * (heatmap.y_max - heatmap.y_min);
            let y = plot_rect.max.y - frac as f32 * plot_rect.height();
            painter.text(
                egui::pos2(plot_rect.min.x - 4.0, y),
                egui::Align2::RIGHT_CENTER,
                format_axis_value(val, y_axis.dec_places),
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }

        let x_label = if x_unit.is_empty() {
            x_name
        } else {
            format!("{} ({})", x_name, x_unit)
        };
        painter.text(
            egui::pos2(plot_rect.center().x, plot_rect.max.y + 20.0),
            egui::Align2::CENTER_TOP,
            x_label,
            egui::FontId::proportional(12.0),
            egui::Color32::LIGHT_GRAY,
        );

        let y_label = if y_unit.is_empty() {
            y_name
        } else {
            format!("{} ({})", y_name, y_unit)
        };
        painter.text(
            egui::pos2(plot_rect.left(), plot_rect.top() - 6.0),
            egui::Align2::LEFT_BOTTOM,
            y_label,
            egui::FontId::proportional(11.0),
            egui::Color32::LIGHT_GRAY,
        );

        let legend_x = plot_rect.max.x + 5.0;
        let legend_h = plot_rect.height().min(165.0);
        let legend_top = plot_rect.center().y - legend_h / 2.0;
        let legend_w = 12.0;
        if self.scatter_mode {
            painter.text(
                egui::pos2(legend_x, legend_top - 20.0),
                egui::Align2::LEFT_BOTTOM,
                legend_title(&v_name, &v_unit),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
            for (i, count) in band_counts.iter().enumerate() {
                let y = legend_top + i as f32 * 17.0;
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(legend_x, y), egui::vec2(10.0, 10.0)),
                    0.0,
                    self.color_bands.color(i),
                );
                painter.text(
                    egui::pos2(legend_x + 14.0, y),
                    egui::Align2::LEFT_TOP,
                    format!("{} ({})", self.color_bands.label(i), count),
                    egui::FontId::proportional(10.0),
                    egui::Color32::LIGHT_GRAY,
                );
            }
        } else {
            for i in 0..legend_steps {
                let frac = 1.0 - i as f32 / (legend_steps.saturating_sub(1).max(1)) as f32;
                let color = heat_color(frac);
                let y = legend_top + i as f32 * legend_h / legend_steps as f32;
                let r = egui::Rect::from_min_size(
                    egui::pos2(legend_x, y),
                    egui::vec2(legend_w, legend_h / legend_steps as f32 + 1.0),
                );
                painter.rect_filled(r, 0.0, color);
                let value = heatmap.value_min + heatmap.value_range * frac as f64;
                painter.text(
                    egui::pos2(legend_x + legend_w + 4.0, y + 1.0),
                    egui::Align2::LEFT_TOP,
                    format_axis_value(value, v_dec_places),
                    egui::FontId::proportional(9.0),
                    egui::Color32::GRAY,
                );
            }
            painter.text(
                egui::pos2(legend_x + legend_w / 2.0, legend_top - 4.0),
                egui::Align2::CENTER_BOTTOM,
                legend_title(&v_name, &v_unit),
                egui::FontId::proportional(10.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && plot_rect.contains(pos)
        {
            if self.scatter_mode {
                let nearest = heatmap
                    .triples
                    .iter()
                    .filter_map(|&(x, y, _, time)| {
                        let d = (pos - to_screen(x, y)).length_sq();
                        (d < 100.0).then_some((d, time))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, time)) = nearest {
                    shared.seek(time);
                }
            } else {
                let xi = (((pos.x - plot_rect.left()) / plot_rect.width()) * self.bins as f32)
                    .floor()
                    .clamp(0.0, self.bins.saturating_sub(1) as f32)
                    as usize;
                let yi = (((plot_rect.bottom() - pos.y) / plot_rect.height()) * self.bins as f32)
                    .floor()
                    .clamp(0.0, self.bins.saturating_sub(1) as f32)
                    as usize;
                if let Some(time) = heatmap.cells[yi * self.bins + xi].representative_time {
                    shared.seek(time);
                }
            }
        }

        if let Some(cursor_time) = shared.cursor_time
            && let Some((x_val, y_val, _)) = mixture_sample(
                x_series,
                y_series,
                v_series,
                cursor_time,
                (t0, t1),
                self.sensor_delay,
                self.minimum_y,
            )
            && (heatmap.x_min..=heatmap.x_max).contains(&x_val)
            && (heatmap.y_min..=heatmap.y_max).contains(&y_val)
        {
            painter.circle_stroke(
                to_screen(x_val, y_val),
                6.0,
                egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
            );
        }
    }
}

struct HeatmapCell {
    sum: f64,
    count: u32,
    representative_time: Option<f64>,
    representative_dist2: f64,
}

struct Heatmap {
    cells: Vec<HeatmapCell>,
    triples: Vec<(f64, f64, f64, f64)>,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    value_min: f64,
    value_range: f64,
    max_count: u32,
}

struct AxisConfig {
    fixed_range: Option<(f64, f64)>,
    tick_count: usize,
    dec_places: i16,
}

pub(crate) fn normalized_sensor_delay(delay: f64) -> f64 {
    if delay.is_finite() {
        delay.clamp(0.0, 5.0)
    } else {
        0.0
    }
}

/// A channel's samples, frequency, display scale and display offset.
type Series<'a> = (&'a [f64], u16, f64, f64);

/// The graph and cursor use the same delayed, transformed, filtered sample.
fn mixture_sample(
    x: Series,
    y: Series,
    value: Series,
    time: f64,
    range: (f64, f64),
    sensor_delay: f64,
    minimum_y: Option<f64>,
) -> Option<(f64, f64, f64)> {
    let delay = normalized_sensor_delay(sensor_delay);
    if !time.is_finite()
        || !range.0.is_finite()
        || !range.1.is_finite()
        || time < range.0.max(0.0)
        || time >= range.1 - delay
    {
        return None;
    }
    let sample = |(data, freq, scale, offset): Series, t: f64| {
        if freq == 0 || t >= data.len() as f64 / freq as f64 {
            return None;
        }
        let v = interp_at_time(data, freq, t)? * scale + offset;
        v.is_finite().then_some(v)
    };
    let (x, y, value) = (
        sample(x, time)?,
        sample(y, time + delay)?,
        sample(value, time)?,
    );
    if minimum_y.is_some_and(|min| y < min) {
        return None;
    }
    Some((x, y, value))
}

#[allow(clippy::too_many_arguments)]
fn compute_heatmap(
    x: Series,
    y: Series,
    value: Series,
    target_freq: u16,
    (t0, t1): (f64, f64),
    bins: usize,
    y_fixed_range: Option<(f64, f64)>,
    sensor_delay: f64,
    minimum_y: Option<f64>,
) -> Heatmap {
    let ((x_data, x_freq, ..), (y_data, y_freq, ..), (v_data, v_freq, ..)) = (x, y, value);
    let mut cells: Vec<HeatmapCell> = (0..bins * bins)
        .map(|_| HeatmapCell {
            sum: 0.0,
            count: 0,
            representative_time: None,
            representative_dist2: f64::INFINITY,
        })
        .collect();

    let sensor_delay = normalized_sensor_delay(sensor_delay);
    let start = (t0.max(0.) * target_freq as f64).ceil() as usize;
    let duration = (x_data.len() as f64 / x_freq.max(1) as f64)
        .min(v_data.len() as f64 / v_freq.max(1) as f64)
        .min(y_data.len() as f64 / y_freq.max(1) as f64 - sensor_delay);
    let end =
        if [x_freq, y_freq, v_freq, target_freq].contains(&0) || !t0.is_finite() || !t1.is_finite()
        {
            0
        } else {
            ((t1 - sensor_delay).min(duration).max(0.) * target_freq as f64).ceil() as usize
        };

    let mut triples = Vec::new();
    let mut x_min = f64::MAX;
    let mut x_max = f64::MIN;
    let mut y_min = f64::MAX;
    let mut y_max = f64::MIN;
    let mut v_min = f64::MAX;
    let mut v_max = f64::MIN;

    // Count every valid sample; display aggregation must not alter statistics.
    for i in start..end {
        let t = i as f64 / target_freq as f64;
        if let Some((x, y, v)) = mixture_sample(x, y, value, t, (t0, t1), sensor_delay, minimum_y) {
            if x < x_min {
                x_min = x;
            }
            if x > x_max {
                x_max = x;
            }
            if y < y_min {
                y_min = y;
            }
            if y > y_max {
                y_max = y;
            }
            if v < v_min {
                v_min = v;
            }
            if v > v_max {
                v_max = v;
            }
            triples.push((x, y, v, t));
        }
    }

    if triples.is_empty() {
        let (fixed_y_min, fixed_y_max) = y_fixed_range.unwrap_or((0.0, 1.0));
        return Heatmap {
            cells,
            triples,
            x_min: 0.0,
            x_max: 1.0,
            y_min: fixed_y_min,
            y_max: fixed_y_max,
            value_min: 0.0,
            value_range: 1.0,
            max_count: 0,
        };
    }

    if let Some((fixed_y_min, fixed_y_max)) = y_fixed_range {
        y_min = fixed_y_min;
        y_max = fixed_y_max;
    }

    let x_range = (x_max - x_min).max(f64::EPSILON);
    let y_range = (y_max - y_min).max(f64::EPSILON);
    let v_range = v_max - v_min;
    let mut max_count = 0;

    for (x, y, v, t) in &triples {
        if *y < y_min || *y > y_max {
            continue;
        }
        let xi = (((x - x_min) / x_range * bins as f64) as usize).min(bins - 1);
        let yi = (((y - y_min) / y_range * bins as f64).clamp(0.0, bins as f64 - 1.0)) as usize;
        let cell = &mut cells[yi * bins + xi];
        cell.sum += v;
        cell.count += 1;
        max_count = max_count.max(cell.count);
        let cx = x_min + (xi as f64 + 0.5) / bins as f64 * x_range;
        let cy = y_min + (yi as f64 + 0.5) / bins as f64 * y_range;
        let dist2 = (x - cx).powi(2) + (y - cy).powi(2);
        if dist2 < cell.representative_dist2 {
            cell.representative_dist2 = dist2;
            cell.representative_time = Some(*t);
        }
    }

    Heatmap {
        cells,
        triples,
        x_min,
        x_max,
        y_min,
        y_max,
        value_min: v_min,
        value_range: v_range,
        max_count,
    }
}

fn axis_config(name: &str, unit: &str, dec_places: i16) -> AxisConfig {
    let lower_name = name.to_ascii_lowercase();
    let lower_unit = unit.to_ascii_lowercase();
    if lower_name.contains("lambda") || lower_unit == "la" {
        return AxisConfig {
            fixed_range: Some((0.7, 1.2)),
            tick_count: 6,
            dec_places: dec_places.max(2),
        };
    }

    AxisConfig {
        fixed_range: None,
        tick_count: 5,
        dec_places,
    }
}

fn axis_fraction(index: usize, tick_count: usize) -> f64 {
    if tick_count <= 1 {
        0.0
    } else {
        index as f64 / (tick_count - 1) as f64
    }
}

fn format_axis_value(value: f64, dec_places: i16) -> String {
    let precision = dec_places.clamp(0, 4) as usize;
    format!("{value:.precision$}")
}

fn legend_title(name: &str, unit: &str) -> String {
    if unit.is_empty() {
        name.to_string()
    } else {
        format!("{name} ({unit})")
    }
}

fn legend_step_count(unit: &str, value_range: f64) -> usize {
    if unit == "%" && value_range >= 20.0 {
        11
    } else {
        9
    }
}

fn step_fraction(frac: f32, steps: usize) -> f32 {
    let steps = steps.max(2);
    let scaled = (frac.clamp(0.0, 1.0) * (steps - 1) as f32).round();
    scaled / (steps - 1) as f32
}

fn normalized_fraction(value: f64, min: f64, max: f64) -> f64 {
    if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn occupancy_alpha(count: u32, max_count: u32) -> f32 {
    if count == 0 || max_count == 0 {
        return 0.0;
    }
    let density = (count as f32 / max_count as f32).sqrt();
    0.2 + 0.8 * density
}

/// Map a 0.0–1.0 fraction to a blue→cyan→green→yellow→red heat color.
fn heat_color(frac: f32) -> egui::Color32 {
    let frac = frac.clamp(0.0, 1.0);
    let (r, g, b) = if frac < 0.25 {
        let t = frac / 0.25;
        (0.0, t, 1.0)
    } else if frac < 0.5 {
        let t = (frac - 0.25) / 0.25;
        (0.0, 1.0, 1.0 - t)
    } else if frac < 0.75 {
        let t = (frac - 0.5) / 0.25;
        (t, 1.0, 0.0)
    } else {
        let t = (frac - 0.75) / 0.25;
        (1.0, 1.0 - t, 0.0)
    };
    egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cursor_matches_delayed_points_and_excludes_filtered_or_out_of_range_samples() {
        let x = [1000.0; 4];
        let y = [1.0, 2.0, 3.0, 4.0];
        let v = [0.0, 10.0, 20.0, 30.0];
        let map = super::compute_heatmap(
            (&x, 1, 1.0, 0.0),
            (&y, 1, 2.0, 10.0),
            (&v, 1, 1.0, 0.0),
            1,
            (0.0, 4.0),
            2,
            None,
            1.0,
            Some(15.0),
        );
        let cursor = |time, range| {
            super::mixture_sample(
                (&x, 1, 1.0, 0.0),
                (&y, 1, 2.0, 10.0),
                (&v, 1, 1.0, 0.0),
                time,
                range,
                1.0,
                Some(15.0),
            )
        };
        assert_eq!(map.triples.len(), 2);
        for &(x, y, v, t) in &map.triples {
            assert_eq!(cursor(t, (0.0, 4.0)), Some((x, y, v)));
        }
        assert_eq!(cursor(1.0, (0.0, 4.0)), Some((1000.0, 16.0, 10.0)));
        assert_eq!(cursor(0.0, (0.0, 4.0)), None); // below transformed Y threshold
        assert_eq!(cursor(1.0, (2.0, 4.0)), None); // before selection
        assert_eq!(cursor(3.0, (0.0, 4.0)), None); // delayed Y outside selection
        assert_eq!(cursor(4.0, (0.0, 10.0)), None); // outside recorded data
        assert_eq!(cursor(f64::NAN, (0.0, 4.0)), None);
        assert_eq!(
            super::mixture_sample(
                (&x, 1, 1.0, 0.0),
                (&[1.0, f64::NAN, 3.0, 4.0], 1, 1.0, 0.0),
                (&v, 1, 1.0, 0.0),
                0.0,
                (0.0, 4.0),
                1.0,
                None,
            ),
            None
        );
    }

    #[test]
    #[ignore = "requires local i2 comparison recording"]
    fn i2_outlap_mixture_sample_count() {
        let ld = i3rs_core::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        ))
        .unwrap();
        let load = |name| {
            let ch = ld.find_channel_by_name(name).unwrap();
            (ld.read_channel_data(ch).unwrap(), ch.freq)
        };
        let (rpm, rf) = load("Engine Speed");
        let (throttle, tf) = load("Throttle Position");
        for (name, expected, low) in [
            ("Exhaust Lambda Bank 1", 6821, 6794),
            ("Exhaust Lambda Bank 2", 6833, 6806),
        ] {
            let (lambda, lf) = load(name);
            let map = super::compute_heatmap(
                (&rpm, rf, 1., 0.),
                (&lambda, lf, 1., 0.),
                (&throttle, tf, 1., 0.),
                20,
                (0., 351.680),
                32,
                Some((0.7, 1.2)),
                0.15,
                Some(0.5),
            );
            assert_eq!(map.triples.len(), expected);
            let bands = super::ColorBands::default();
            let mut counts = vec![0; bands.len()];
            for &(_, _, v, _) in &map.triples {
                counts[bands.index(v)] += 1;
            }
            assert_eq!(&counts[1..12], &[low, 3, 3, 2, 1, 1, 0, 1, 4, 6, 6]);
        }
    }
    #[test]
    fn display_bounds_do_not_filter_statistics_and_delay_stays_inside_range() {
        let map = super::compute_heatmap(
            (&[1000.; 4], 1, 1., 0.),
            (&[1., 2., 3., 4.], 1, 1., 0.),
            (&[0., 10., 20., 30.], 1, 1., 0.),
            1,
            (0., 4.),
            2,
            Some((0.7, 1.2)),
            1.,
            None,
        );
        assert_eq!(
            map.triples,
            [
                (1000., 2., 0., 0.),
                (1000., 3., 10., 1.),
                (1000., 4., 20., 2.)
            ]
        );
        assert!(map.cells.iter().all(|c| c.count == 0));
    }
}

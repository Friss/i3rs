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
    display_transform_fingerprint, refresh_plotted_channel, resolve_plotted_channel_display_meta,
    segmented_channel_button, show_plotted_channel_display_menu,
};

#[derive(PartialEq)]
struct HeatmapFingerprint {
    filter: String,
    policies: [i3rs_core::Interpolation; 3],
    x_data_ptr: usize,
    y_data_ptr: usize,
    value_data_ptr: usize,
    zoom_key: Option<(u64, u64)>,
    bins: usize,
    delay: u64,
    minimum_y: Option<u64>,
    frequencies: [f64; 3],
    x_transform: DisplayTransformFingerprint,
    y_transform: DisplayTransformFingerprint,
    value_transform: DisplayTransformFingerprint,
}

struct HeatmapCache {
    fingerprint: HeatmapFingerprint,
    heatmap: Heatmap,
}

pub struct MixtureMapPanel {
    pub band_presentation: super::distribution::BandPresentation,
    pub show_trends: bool,
    pub trend_width: f64,
    pub filter: super::distribution::ComponentFilter,
    pub gear_filter: i3rs_core::distribution::GearFilter,
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
            band_presentation: Default::default(),
            show_trends: false,
            trend_width: 100.,
            filter: Default::default(),
            gear_filter: Default::default(),
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

        self.band_presentation.controls(ui, self.color_bands);
        self.filter.controls(ui);
        let filter = self.filter.prepare(ui, shared);
        ui.collapsing("Gear shift exclusion", |ui| {
            ui.checkbox(
                &mut self.gear_filter.enabled,
                "Exclude high RPM rates (X must be RPM)",
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("Rising rpm/s");
                ui.add(
                    egui::DragValue::new(&mut self.gear_filter.rising_rate).range(0.0..=100000.0),
                );
                ui.label("Falling rpm/s");
                ui.add(
                    egui::DragValue::new(&mut self.gear_filter.falling_rate).range(-100000.0..=0.0),
                );
                ui.label("Hold s");
                ui.add(
                    egui::DragValue::new(&mut self.gear_filter.exclusion_seconds).range(0.0..=5.0),
                );
                ui.label("RPM average s");
                ui.add(
                    egui::DragValue::new(&mut self.gear_filter.rpm_filter_seconds).range(0.0..=5.0),
                );
            });
        });

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
        if self.gear_filter.enabled
            && !matches!(x_unit.to_lowercase().as_str(), "rpm" | "r/min" | "1/min")
        {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Gear filtering requires an RPM X channel; no samples included",
            );
        }

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
                ui.weak(if self.gear_filter.enabled {
                    "RPM-rate shift exclusion is applied before binning"
                } else {
                    "RPM-rate shift exclusion is disabled"
                });
            }
        });
        let target_freq = x_freq;
        if target_freq == 0.0 {
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

        let zoom_key = Some((t0.to_bits(), t1.to_bits()));
        let fingerprint = HeatmapFingerprint {
            filter: format!("{}:{:?}", filter.key, self.gear_filter),
            policies: [
                build_plotted_channel_info(x_ch, shared).interpolation,
                build_plotted_channel_info(y_ch, shared).interpolation,
                build_plotted_channel_info(v_ch, shared).interpolation,
            ],
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
            let rpm: Vec<_> = x_ch
                .data
                .iter()
                .map(|v| v * x_ch.display_scale + x_ch.display_offset)
                .collect();
            let mask = i3rs_core::distribution::gear_filter_mask(&rpm, x_freq, self.gear_filter);
            let rpm_unit = matches!(x_unit.to_lowercase().as_str(), "rpm" | "r/min" | "1/min");
            let heatmap = compute_heatmap_filtered_with_policies(
                x_series,
                y_series,
                v_series,
                target_freq,
                (t0, t1),
                self.bins,
                y_axis.fixed_range,
                self.sensor_delay,
                self.minimum_y,
                [
                    build_plotted_channel_info(x_ch, shared).interpolation,
                    build_plotted_channel_info(y_ch, shared).interpolation,
                    build_plotted_channel_info(v_ch, shared).interpolation,
                ],
                |t| {
                    filter.includes(t)
                        && (!self.gear_filter.enabled
                            || (rpm_unit
                                && mask
                                    .get((t * x_freq).round() as usize)
                                    .copied()
                                    .unwrap_or(false)))
                },
            );
            self.cache = Some(Box::new(HeatmapCache {
                fingerprint,
                heatmap,
            }));
        }
        let overlays =
            crate::comparison::overlay_populations(shared, &[x_ch, y_ch, v_ch], &self.filter);
        let mut overlay_maps = Vec::new();
        for overlay in overlays {
            let x = &overlay.channels[0];
            let y = &overlay.channels[1];
            let v = &overlay.channels[2];
            let rpm: Vec<_> = x.data.iter().map(|n| x.transform_value(*n)).collect();
            let mask = i3rs_core::distribution::gear_filter_mask(&rpm, x.freq, self.gear_filter);
            let map = compute_heatmap_filtered_with_policies(
                (&x.data, x.freq, x.display_scale, x.display_offset),
                (&y.data, y.freq, y.display_scale, y.display_offset),
                (&v.data, v.freq, v.display_scale, v.display_offset),
                x.freq,
                overlay.range,
                self.bins,
                axis_config(&y.name, &y.unit, y.dec_places).fixed_range,
                self.sensor_delay,
                self.minimum_y,
                [x.interpolation, y.interpolation, v.interpolation],
                |t| {
                    overlay.filter.includes(t)
                        && (!self.gear_filter.enabled
                            || (matches!(
                                x.unit.to_lowercase().as_str(),
                                "rpm" | "r/min" | "1/min"
                            ) && mask
                                .get((t * x.freq).round() as usize)
                                .copied()
                                .unwrap_or(false)))
                },
            );
            ui.colored_label(
                overlay.color,
                format!("{} · {} samples", overlay.label, map.triples.len()),
            );
            overlay_maps.push((map, overlay.color, overlay.label, overlay.range));
        }
        if shared
            .comparisons
            .sessions
            .iter()
            .any(|s| !s.requests.is_empty())
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
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
        ui.collapsing("Load band trends / averages", |ui| {
            ui.checkbox(&mut self.show_trends, "Show band means by X interval");
            ui.horizontal(|ui| {
                ui.label(format!("Interval width ({x_unit})"));
                ui.add(egui::DragValue::new(&mut self.trend_width).range(0.000001..=1e9));
            });
            if self.show_trends {
                let mut csv = String::new();
                for (band, rows) in band_trends(heatmap, self.color_bands, self.trend_width)
                    .iter()
                    .enumerate()
                {
                    csv.push_str(&i3rs_core::distribution::x_interval_csv(
                        &format!(
                            "{} · {} · {} {}",
                            shared.file_name,
                            y_name,
                            v_name,
                            self.color_bands.label(band)
                        ),
                        &x_unit,
                        &y_unit,
                        (t0, t1),
                        rows,
                        csv.is_empty(),
                    ));
                }
                for (map, _, label, range) in &overlay_maps {
                    for (band, rows) in band_trends(map, self.color_bands, self.trend_width)
                        .iter()
                        .enumerate()
                    {
                        csv.push_str(&i3rs_core::distribution::x_interval_csv(
                            &format!("{} · {} · {}", label, y_name, self.color_bands.label(band)),
                            &x_unit,
                            &y_unit,
                            *range,
                            rows,
                            false,
                        ));
                    }
                }
                super::calculations::export_buttons(ui, &csv);
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
                if self.band_presentation.visible(band) {
                    painter.circle_filled(
                        to_screen(x, y),
                        1.75,
                        self.band_presentation.color(self.color_bands, band),
                    );
                }
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
        for (map, color, _, _) in &overlay_maps {
            for &(x, y, value, _) in &map.triples {
                if self
                    .band_presentation
                    .visible(self.color_bands.index(value))
                    && x >= heatmap.x_min
                    && x <= heatmap.x_max
                    && y >= heatmap.y_min
                    && y <= heatmap.y_max
                {
                    painter.circle_stroke(to_screen(x, y), 2.0, egui::Stroke::new(1., *color));
                }
            }
        }
        if self.scatter_mode && self.show_trends {
            let clipped = painter.with_clip_rect(plot_rect);
            let draw_trends = |map: &Heatmap| {
                for (band, rows) in band_trends(map, self.color_bands, self.trend_width)
                    .iter()
                    .enumerate()
                {
                    if !self.band_presentation.visible(band) {
                        continue;
                    }
                    let color = self.band_presentation.color(self.color_bands, band);
                    let points: Vec<_> = rows
                        .iter()
                        .map(|r| to_screen((r.lower + r.upper) / 2., r.mean_y))
                        .collect();
                    for pair in points.windows(2) {
                        clipped.line_segment([pair[0], pair[1]], egui::Stroke::new(1.5, color));
                    }
                    for point in points {
                        clipped.add(egui::Shape::convex_polygon(
                            vec![
                                point + egui::vec2(0., -4.),
                                point + egui::vec2(4., 0.),
                                point + egui::vec2(0., 4.),
                                point + egui::vec2(-4., 0.),
                            ],
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                }
            };
            draw_trends(heatmap);
            for (map, _, _, _) in &overlay_maps {
                draw_trends(map);
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
            && {
                let index = heatmap.triples.partition_point(|p| p.3 < cursor_time);
                [index.checked_sub(1), Some(index)]
                    .into_iter()
                    .flatten()
                    .filter_map(|i| heatmap.triples.get(i))
                    .any(|p| (p.3 - cursor_time).abs() <= 0.5 / x_freq.max(1.0))
            }
            && let Some((x_val, y_val, _)) = mixture_sample_with_policies(
                x_series,
                y_series,
                v_series,
                cursor_time,
                (t0, t1),
                self.sensor_delay,
                self.minimum_y,
                [
                    build_plotted_channel_info(x_ch, shared).interpolation,
                    build_plotted_channel_info(y_ch, shared).interpolation,
                    build_plotted_channel_info(v_ch, shared).interpolation,
                ],
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
type Series<'a> = (&'a [f64], f64, f64, f64);

/// Selection is in lambda time: X/load at t pair with lambda at t + delay.
/// Therefore an interior selection can use RPM/load preceding its start.
/// Never extrapolate before the recording or beyond any recorded channel.
#[cfg(test)]
fn mixture_sample(
    x: Series,
    y: Series,
    value: Series,
    time: f64,
    range: (f64, f64),
    sensor_delay: f64,
    minimum_y: Option<f64>,
) -> Option<(f64, f64, f64)> {
    mixture_sample_with_policies(
        x,
        y,
        value,
        time,
        range,
        sensor_delay,
        minimum_y,
        [i3rs_core::Interpolation::Linear; 3],
    )
}
#[allow(clippy::too_many_arguments)]
fn mixture_sample_with_policies(
    x: Series,
    y: Series,
    value: Series,
    time: f64,
    range: (f64, f64),
    sensor_delay: f64,
    minimum_y: Option<f64>,
    policies: [i3rs_core::Interpolation; 3],
) -> Option<(f64, f64, f64)> {
    let delay = normalized_sensor_delay(sensor_delay);
    if !time.is_finite()
        || !range.0.is_finite()
        || !range.1.is_finite()
        || time < (range.0 - delay).max(0.0)
        || time >= range.1 - delay
    {
        return None;
    }
    let sample = |(data, freq, scale, offset): Series, t: f64, policy| {
        if freq == 0.0 || t >= data.len() as f64 / freq {
            return None;
        }
        let v = i3rs_core::channels::sample_with_policy(data, freq, t, policy)? * scale + offset;
        v.is_finite().then_some(v)
    };
    let (x, y, value) = (
        sample(x, time, policies[0])?,
        sample(y, time + delay, policies[1])?,
        sample(value, time, policies[2])?,
    );
    if minimum_y.is_some_and(|min| y < min) {
        return None;
    }
    Some((x, y, value))
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn compute_heatmap(
    x: Series,
    y: Series,
    value: Series,
    target_freq: f64,
    range: (f64, f64),
    bins: usize,
    y_fixed_range: Option<(f64, f64)>,
    sensor_delay: f64,
    minimum_y: Option<f64>,
) -> Heatmap {
    compute_heatmap_filtered(
        x,
        y,
        value,
        target_freq,
        range,
        bins,
        y_fixed_range,
        sensor_delay,
        minimum_y,
        |_| true,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn compute_heatmap_filtered(
    x: Series,
    y: Series,
    value: Series,
    target_freq: f64,
    (t0, t1): (f64, f64),
    bins: usize,
    y_fixed_range: Option<(f64, f64)>,
    sensor_delay: f64,
    minimum_y: Option<f64>,
    includes: impl Fn(f64) -> bool,
) -> Heatmap {
    compute_heatmap_filtered_with_policies(
        x,
        y,
        value,
        target_freq,
        (t0, t1),
        bins,
        y_fixed_range,
        sensor_delay,
        minimum_y,
        [i3rs_core::Interpolation::Linear; 3],
        includes,
    )
}
#[allow(clippy::too_many_arguments)]
fn compute_heatmap_filtered_with_policies(
    x: Series,
    y: Series,
    value: Series,
    target_freq: f64,
    (t0, t1): (f64, f64),
    bins: usize,
    y_fixed_range: Option<(f64, f64)>,
    sensor_delay: f64,
    minimum_y: Option<f64>,
    policies: [i3rs_core::Interpolation; 3],
    includes: impl Fn(f64) -> bool,
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
    let start = ((t0 - sensor_delay).max(0.) * target_freq).ceil() as usize;
    let duration = (x_data.len() as f64 / x_freq.max(1.0))
        .min(v_data.len() as f64 / v_freq.max(1.0))
        .min(y_data.len() as f64 / y_freq.max(1.0) - sensor_delay);
    let end = if [x_freq, y_freq, v_freq, target_freq].contains(&0.0)
        || !t0.is_finite()
        || !t1.is_finite()
    {
        0
    } else {
        ((t1 - sensor_delay).min(duration).max(0.) * target_freq).ceil() as usize
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
        let t = i as f64 / target_freq;
        if !includes(t) {
            continue;
        }
        if let Some((x, y, v)) = mixture_sample_with_policies(
            x,
            y,
            value,
            t,
            (t0, t1),
            sensor_delay,
            minimum_y,
            policies,
        ) {
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
            (&x, 1.0, 1.0, 0.0),
            (&y, 1.0, 2.0, 10.0),
            (&v, 1.0, 1.0, 0.0),
            1.0,
            (0.0, 4.0),
            2,
            None,
            1.0,
            Some(15.0),
        );
        let cursor = |time, range| {
            super::mixture_sample(
                (&x, 1.0, 1.0, 0.0),
                (&y, 1.0, 2.0, 10.0),
                (&v, 1.0, 1.0, 0.0),
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
        assert_eq!(cursor(1.0, (2.0, 4.0)), Some((1000.0, 16.0, 10.0))); // lambda timestamp is inside selection
        assert_eq!(cursor(0.5, (2.0, 4.0)), None); // before delayed selection
        assert_eq!(cursor(3.0, (0.0, 4.0)), None); // delayed Y outside selection
        assert_eq!(cursor(4.0, (0.0, 10.0)), None); // outside recorded data
        assert_eq!(cursor(f64::NAN, (0.0, 4.0)), None);
        assert_eq!(
            super::mixture_sample(
                (&x, 1.0, 1.0, 0.0),
                (&[1.0, f64::NAN, 3.0, 4.0], 1.0, 1.0, 0.0),
                (&v, 1.0, 1.0, 0.0),
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
                20.0,
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
            (&[1000.; 4], 1.0, 1., 0.),
            (&[1., 2., 3., 4.], 1.0, 1., 0.),
            (&[0., 10., 20., 30.], 1.0, 1., 0.),
            1.0,
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

#[cfg(test)]
mod shift_reference_tests {
    use super::*;
    #[test]
    #[ignore = "requires local May 8 reference recording"]
    fn matches_live_i2_shift_laps_at_two_filter_times() {
        let ld = i3rs_core::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        ))
        .unwrap();
        let laps = i3rs_core::detect_laps(&ld);
        let read = |name| {
            let c = ld.find_channel_by_name(name).unwrap();
            (ld.read_channel_data(c).unwrap(), c.freq)
        };
        let (rpm, hz) = read("Engine Speed");
        let (load, lf) = read("Throttle Position");
        for (lap, seconds, expected) in [
            (3, 0., vec![733, 452, 122, 56, 56, 24, 25, 14, 18, 39, 1143]),
            (
                3,
                0.25,
                vec![686, 442, 116, 54, 56, 23, 25, 14, 17, 38, 1141],
            ),
            (
                3,
                0.5,
                vec![702, 441, 115, 54, 56, 23, 25, 14, 17, 38, 1137],
            ),
            (
                4,
                0.25,
                vec![747, 180, 91, 100, 28, 11, 15, 11, 14, 22, 1237],
            ),
        ] {
            let mask = i3rs_core::distribution::gear_filter_mask(
                &rpm,
                hz,
                i3rs_core::distribution::GearFilter {
                    enabled: seconds > 0.,
                    rpm_filter_seconds: seconds,
                    ..Default::default()
                },
            );
            for bank in ["Exhaust Lambda Bank 1", "Exhaust Lambda Bank 2"] {
                let (lambda, yf) = read(bank);
                let l = &laps[lap];
                let map = compute_heatmap_filtered(
                    (&rpm, hz, 1., 0.),
                    (&lambda, yf, 1., 0.),
                    (&load, lf, 1., 0.),
                    hz,
                    (l.start_time, l.end_time),
                    32,
                    Some((0.7, 1.2)),
                    0.15,
                    Some(0.5),
                    |t| {
                        mask.get((t * hz).round() as usize)
                            .copied()
                            .unwrap_or(false)
                    },
                );
                let mut counts = vec![0; ColorBands::default().len()];
                for &(_, _, v, _) in &map.triples {
                    counts[ColorBands::default().index(v)] += 1;
                }
                assert_eq!(
                    &counts[1..12],
                    expected.as_slice(),
                    "{bank} lap {lap} filter {seconds}"
                );
                assert_eq!(map.triples.len(), expected.iter().sum::<usize>());
            }
        }
    }
    #[test]
    fn lambda_time_selection_uses_available_prior_rpm_without_extrapolation() {
        let x = [1000.; 5];
        let y = [1., 2., 3., 4., 5.];
        let load = [0.; 5];
        let map = compute_heatmap(
            (&x, 1.0, 1., 0.),
            (&y, 1.0, 1., 0.),
            (&load, 1.0, 1., 0.),
            1.0,
            (1., 3.),
            5,
            None,
            1.,
            None,
        );
        assert_eq!(map.triples, [(1000., 2., 0., 0.), (1000., 3., 0., 1.)]);
    }
}

#[cfg(test)]
mod rate_policy_tests {
    use super::*;
    #[test]
    fn mixed_rate_state_load_is_held_and_missing_lambda_leaves_population() {
        let x = [1000.; 8];
        let y = [1., f64::NAN, 1., 1.];
        let load = [0., 100.];
        let map = compute_heatmap_filtered_with_policies(
            (&x, 4.0, 1., 0.),
            (&y, 2.0, 1., 0.),
            (&load, 1.0, 1., 0.),
            4.0,
            (0., 2.),
            2,
            None,
            0.,
            None,
            [
                i3rs_core::Interpolation::Linear,
                i3rs_core::Interpolation::Linear,
                i3rs_core::Interpolation::Previous,
            ],
            |_| true,
        );
        assert_eq!(map.triples.len(), 5);
        assert!(map.triples.iter().filter(|p| p.3 < 1.).all(|p| p.2 == 0.));
        assert!(
            map.triples
                .iter()
                .filter(|p| p.3 >= 1.)
                .all(|p| p.2 == 100.)
        );
    }
}

fn band_trends(
    map: &Heatmap,
    bands: ColorBands,
    width: f64,
) -> Vec<Vec<i3rs_core::distribution::XIntervalAverage>> {
    let mut groups = vec![Vec::new(); bands.len()];
    for &(x, y, load, _) in &map.triples {
        groups[bands.index(load)].push((x, y));
    }
    groups
        .into_iter()
        .map(|pairs| i3rs_core::distribution::x_interval_averages(pairs, width, 0.))
        .collect()
}

#[cfg(test)]
mod band_trend_tests {
    use super::*;
    #[test]
    fn load_band_x_means_retain_each_filtered_native_sample() {
        let x = [50., 99., 100., 199., 200.];
        let y = [1., 3., 5., 7., 9.];
        let load = [0., 0., 100., 100., 100.];
        let map = compute_heatmap(
            (&x, 1.0, 1., 0.),
            (&y, 1.0, 1., 0.),
            (&load, 1.0, 1., 0.),
            1.0,
            (0., 5.),
            5,
            None,
            0.,
            None,
        );
        let trends = band_trends(&map, ColorBands::default(), 100.);
        assert_eq!((trends[1][0].count, trends[1][0].mean_y), (2, 2.));
        assert_eq!((trends[11][0].count, trends[11][0].mean_y), (2, 6.));
        assert_eq!(
            trends.iter().flatten().map(|r| r.count).sum::<usize>(),
            map.triples.len()
        );
    }
}

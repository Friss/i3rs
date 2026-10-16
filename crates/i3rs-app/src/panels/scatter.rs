//! Scatter/XY plot panel: channel vs channel visualization.

use eframe::egui;
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotBounds, PlotPoint, PlotPoints, Points};

use crate::state::{ChannelId, PlottedChannel, SharedState};

use super::utils::{
    DisplayTransformFingerprint, build_plotted_channel_info, create_plotted_channel,
    display_transform_fingerprint, interp_at_time, refresh_plotted_channel,
    resolve_plotted_channel_display_meta, segmented_channel_button,
    show_plotted_channel_display_menu, transform_channel_value,
};

#[derive(PartialEq, Eq)]
struct ScatterFingerprint {
    filter: String,
    policies: [i3rs_core::Interpolation; 2],
    x_data_ptr: usize,
    x_data_len: usize,
    y_data_ptr: usize,
    y_data_len: usize,
    frequencies: (u16, u16),
    zoom_key: Option<(u64, u64)>,
    x_transform: DisplayTransformFingerprint,
    y_transform: DisplayTransformFingerprint,
}

struct ScatterCache {
    fingerprint: ScatterFingerprint,
    points: Vec<ScatterPoint>,
    plot_points: Vec<PlotPoint>,
    /// Unpadded ((x_min, x_max), (y_min, y_max)) of `plot_points`.
    data_bounds: Option<((f64, f64), (f64, f64))>,
}

#[derive(Clone, Copy)]
struct ScatterPoint {
    x: f64,
    y: f64,
    time: f64,
}

pub struct ScatterPanel {
    pub band_presentation: super::distribution::BandPresentation,
    pub filter: super::distribution::ComponentFilter,
    pub color_channel: Option<PlottedChannel>,
    pub color_bands: super::distribution::ColorBands,
    pub show_regression: bool,
    pub show_interval_averages: bool,
    pub interval_width: f64,
    pub interval_origin: f64,
    pub density: bool,
    color_filter: String,
    pub id: u64,
    pub title: String,
    /// X-axis channel.
    pub x_channel: Option<PlottedChannel>,
    /// Y-axis channel.
    pub y_channel: Option<PlottedChannel>,
    /// Point size.
    pub point_size: f32,
    /// Optional fractional padding applied to the data bounds on each axis.
    pub bounds_padding_frac: f64,
    /// When true, the plot stays fixed to its computed bounds.
    pub lock_bounds: bool,
    cache: Option<ScatterCache>,
}

impl ScatterPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            band_presentation: Default::default(),
            filter: Default::default(),
            color_channel: None,
            color_bands: Default::default(),
            show_regression: false,
            show_interval_averages: false,
            interval_width: 100.,
            interval_origin: 0.,
            density: false,
            color_filter: String::new(),
            id,
            title: title.into(),
            x_channel: None,
            y_channel: None,
            point_size: 1.5,
            bounds_padding_frac: 0.0,
            lock_bounds: false,
            cache: None,
        }
    }

    pub fn clear_channels(&mut self) {
        self.x_channel = None;
        self.y_channel = None;
        self.cache = None;
    }

    fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if let Some(pc) = create_plotted_channel(channel_id, shared, 0) {
            if self.x_channel.is_none() {
                self.x_channel = Some(pc);
            } else if self.y_channel.is_none()
                || self
                    .y_channel
                    .as_ref()
                    .is_some_and(|y| y.channel_id != channel_id)
            {
                self.y_channel = Some(pc);
            }
        }
    }

    fn sync_series_color_from_x(&mut self) {
        let Some(color) = self.x_channel.as_ref().map(|channel| channel.color) else {
            return;
        };
        if let Some(channel) = self.y_channel.as_mut() {
            channel.color = color;
        }
    }

    fn sync_series_color_from_y(&mut self) {
        let Some(color) = self.y_channel.as_ref().map(|channel| channel.color) else {
            return;
        };
        if let Some(channel) = self.x_channel.as_mut() {
            channel.color = color;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        let _perf = crate::perf_metrics::scope("scatter draw");

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

        // Toolbar
        ui.horizontal(|ui| {
            ui.label("X:");
            let mut clear_x = false;
            let mut sync_x_color = false;
            if let Some(channel) = self.x_channel.as_mut() {
                let (x_name, _, _, _, _) = resolve_plotted_channel_display_meta(channel, shared);
                let (x_resp, clear_clicked) =
                    segmented_channel_button(ui, &x_name, None, "Clear X channel");
                x_resp.context_menu(|ui| {
                    let old_color = channel.color;
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_x = true;
                    } else if channel.color != old_color {
                        sync_x_color = true;
                    }
                });
                if clear_clicked {
                    clear_x = true;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Drop X channel"));
            }
            if clear_x {
                self.x_channel = None;
                self.cache = None;
            } else if sync_x_color {
                self.sync_series_color_from_x();
            }

            ui.label("Y:");
            let mut clear_y = false;
            let mut sync_y_color = false;
            if let Some(channel) = self.y_channel.as_mut() {
                let (y_name, _, _, _, _) = resolve_plotted_channel_display_meta(channel, shared);
                let (y_resp, clear_clicked) =
                    segmented_channel_button(ui, &y_name, None, "Clear Y channel");
                y_resp.context_menu(|ui| {
                    let old_color = channel.color;
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_y = true;
                    } else if channel.color != old_color {
                        sync_y_color = true;
                    }
                });
                if clear_clicked {
                    clear_y = true;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Drop Y channel"));
            }
            if clear_y {
                self.y_channel = None;
                self.cache = None;
            } else if sync_y_color {
                self.sync_series_color_from_y();
            }

            ui.label("Size:");
            ui.add(
                egui::DragValue::new(&mut self.point_size)
                    .range(0.5..=10.0)
                    .speed(0.1),
            );
        });
        ui.separator();

        // Register channels for readout
        for pc in self.x_channel.iter().chain(self.y_channel.iter()) {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(pc, shared));
        }

        ui.checkbox(
            &mut self.density,
            "Density display (native population retained)",
        );
        self.band_presentation.controls(ui, self.color_bands);
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_interval_averages, "X interval averages");
            if self.show_interval_averages {
                ui.label("Width (X units)");
                ui.add(egui::DragValue::new(&mut self.interval_width).range(0.000001..=1e9));
                ui.label("Origin");
                ui.add(egui::DragValue::new(&mut self.interval_origin));
            }
        });
        self.filter.controls(ui);
        let filter = self.filter.prepare(ui, shared);
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_regression, "Linear regression");
            let mut selected = self.color_channel.as_ref().map(|c| c.channel_id);
            if super::utils::channel_picker(
                ui,
                ("scatter_color", self.id),
                &mut selected,
                &mut self.color_filter,
                shared,
            ) {
                self.color_channel = selected.and_then(|id| create_plotted_channel(id, shared, 0));
            }
            if let Some(c) = &mut self.color_channel {
                refresh_plotted_channel(c, shared);
                ui.label(format!(
                    "Color: {}",
                    resolve_plotted_channel_display_meta(c, shared).0
                ));
            }
            if ui.small_button("Clear color").clicked() {
                self.color_channel = None;
            }
        });
        if self.color_channel.is_some() {
            ui.horizontal_wrapped(|ui| self.color_bands.controls(ui));
        }

        let (Some(x_ch), Some(y_ch)) = (&self.x_channel, &self.y_channel) else {
            ui.centered_and_justified(|ui| {
                ui.label("Drop two channels: first for X axis, second for Y axis");
            });
            return;
        };

        let (x_name, x_unit, x_freq, _, _) = resolve_plotted_channel_display_meta(x_ch, shared);
        let (y_name, y_unit, y_freq, _, _) = resolve_plotted_channel_display_meta(y_ch, shared);

        let x_label = if x_unit.is_empty() {
            x_name.clone()
        } else {
            format!("{} ({})", x_name, x_unit)
        };
        let y_label = if y_unit.is_empty() {
            y_name.clone()
        } else {
            format!("{} ({})", y_name, y_unit)
        };

        let plot = Plot::new(format!("scatter_{}", self.id))
            .legend(Legend::default())
            .x_axis_label(x_label)
            .y_axis_label(y_label);

        let point_size = self.point_size;

        // Preserve each sample of the faster signal; interpolate the slower axis.
        let target_freq = x_freq.max(y_freq);
        let color = self
            .y_channel
            .as_ref()
            .map(|channel| channel.color)
            .unwrap_or(egui::Color32::WHITE);

        // Get time range for visible data
        let (t0, t1) = shared.analysis_range();

        let zoom_key = Some((t0.to_bits(), t1.to_bits()));
        let fingerprint = ScatterFingerprint {
            filter: filter.key.clone(),
            policies: [
                build_plotted_channel_info(x_ch, shared).interpolation,
                build_plotted_channel_info(y_ch, shared).interpolation,
            ],
            x_data_ptr: x_ch.data.as_ptr() as usize,
            x_data_len: x_ch.data.len(),
            y_data_ptr: y_ch.data.as_ptr() as usize,
            y_data_len: y_ch.data.len(),
            frequencies: (x_freq, y_freq),
            zoom_key,
            x_transform: display_transform_fingerprint(x_ch),
            y_transform: display_transform_fingerprint(y_ch),
        };

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let points: Vec<_> = scatter_info_points(
                &build_plotted_channel_info(x_ch, shared),
                &build_plotted_channel_info(y_ch, shared),
                target_freq,
                (t0, t1),
            )
            .into_iter()
            .filter(|p| filter.includes(p.time))
            .collect();
            let plot_points: Vec<_> = points.iter().map(|p| PlotPoint::new(p.x, p.y)).collect();
            let data_bounds = data_bounds(&plot_points);
            self.cache = Some(ScatterCache {
                fingerprint,
                points,
                plot_points,
                data_bounds,
            });
        }
        let overlay_slots: Vec<_> = self
            .x_channel
            .iter()
            .chain(self.y_channel.iter())
            .chain(self.color_channel.iter())
            .collect();
        let overlays = crate::comparison::overlay_populations(shared, &overlay_slots, &self.filter);
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
        let overlay_points: Vec<_> = overlays
            .iter()
            .map(|overlay| {
                let x = &overlay.channels[0];
                let y = &overlay.channels[1];
                let points: Vec<_> = scatter_info_points(x, y, x.freq.max(y.freq), overlay.range)
                    .into_iter()
                    .filter(|p| overlay.filter.includes(p.time))
                    .collect();
                let fit =
                    i3rs_core::distribution::linear_regression(points.iter().map(|p| (p.x, p.y)));
                (overlay, points, fit)
            })
            .collect();
        let cache = self.cache.as_ref().unwrap();
        let points = &cache.points;
        ui.label(format!(
            "{} samples · range follows lap / zoom",
            points.len()
        ));
        let regression =
            i3rs_core::distribution::linear_regression(points.iter().map(|p| (p.x, p.y)));
        if self.show_regression {
            if let Some(r) = &regression {
                ui.label(format!(
                    "y = {:.6} x {:+.6} · R² {} · n={}",
                    r.slope,
                    r.intercept,
                    r.r_squared
                        .map_or_else(|| "undefined".into(), |v| format!("{v:.6}")),
                    r.count
                ));
            } else {
                ui.weak("Regression requires two finite samples with different X values");
            }
        }
        if self.show_regression {
            let mut rows = Vec::new();
            if let Some(r) = &regression {
                rows.extend(regression_rows(
                    &format!("{} vs {}", y_name, x_name),
                    r,
                    shared.analysis_range(),
                    &x_unit,
                    &y_unit,
                ));
            }
            for (overlay, points, fit) in &overlay_points {
                ui.colored_label(
                    overlay.color,
                    format!("{} · {} samples", overlay.label, points.len()),
                );
                if let Some(r) = fit {
                    ui.label(format!(
                        "y = {:.6} x {:+.6} · R² {} · n={}",
                        r.slope,
                        r.intercept,
                        r.r_squared
                            .map_or_else(|| "undefined".into(), |v| format!("{v:.6}")),
                        r.count
                    ));
                    rows.extend(regression_rows(
                        &format!("{} · {} vs {}", overlay.label, y_name, x_name),
                        r,
                        overlay.range,
                        &x_unit,
                        &y_unit,
                    ));
                } else {
                    ui.weak("Regression undefined for this comparison population");
                }
            }
            super::calculations::export_buttons(ui, &super::calculations::result_csv(&rows));
            if ui.button("Create scoped regression maths").on_hover_text("Creates editable slope, intercept and R² channels for each recording using this range and current gate/trust settings.").clicked() {
                let x_expression=channel_expression(x_ch,shared);let y_expression=channel_expression(y_ch,shared);
                match x_expression.zip(y_expression) {
                    Some((x,y))=>{
                        let mut definitions=Vec::new();
                        if regression.is_some(){definitions.push((shared.recording_key().to_owned(),shared.analysis_range(),target_freq,filter.trusted_ranges.clone()));}
                        for (o,_,fit) in &overlay_points {if fit.is_some(){definitions.push((o.recording.clone(),o.range,o.channels[0].freq.max(o.channels[1].freq),o.filter.trusted_ranges.clone()));}}
                        for (recording,range,hz,trusted) in definitions {
                            let Some(gate)=regression_gate_expression(&self.filter.gate,trusted.as_deref())else{shared.message("Cannot quote gate reference containing both quote characters");continue;};
                            for (suffix,function,unit) in [("Slope","regression_slope",format!("{y_unit}/({x_unit})")),("Intercept","regression_intercept",y_unit.clone()),("R squared","regression_r_squared",String::new())] {
                                let name=format!("{} regression {} {suffix}",self.title,self.id);
                                let expression=format!("{function}({x},{y},{gate},{:.12},{:.12})",range.0,range.1);
                                let id=shared.math_channels.iter().position(|m|m.name==name&&m.options.recording.as_deref()==Some(&recording));
                                if let Some(index)=id {let def=&mut shared.math_channels[index];def.expression=expression;def.unit=unit;def.options.output_freq=Some(hz);def.options.comment="Regression over the captured component range and gate/trust configuration".into();}
                                else{let mut def=shared.create_math_channel_def(name,expression,unit,6);def.options.recording=Some(recording.clone());def.options.output_freq=Some(hz);def.options.comment="Regression over the captured component range and gate/trust configuration".into();shared.math_channels.push(def);}
                            }
                        }
                        crate::panels::math_editor::evaluate_all_math_channels(shared);
                        shared.message("Scoped regression channels created; the expressions retain the selected range and gate/trust configuration");
                    },None=>shared.message("Cannot quote channel reference containing both quote characters"),
                }
            }
        }
        let averages = i3rs_core::distribution::x_interval_averages(
            points.iter().map(|p| (p.x, p.y)),
            self.interval_width,
            self.interval_origin,
        );
        let overlay_averages: Vec<_> = overlay_points
            .iter()
            .map(|(o, p, _)| {
                (
                    *o,
                    i3rs_core::distribution::x_interval_averages(
                        p.iter().map(|p| (p.x, p.y)),
                        self.interval_width,
                        self.interval_origin,
                    ),
                )
            })
            .collect();
        if self.show_interval_averages {
            ui.label(format!(
                "{} populated X intervals · {} native samples",
                averages.len(),
                points.len()
            ));
            let mut csv = i3rs_core::distribution::x_interval_csv(
                &format!("{} · {} vs {}", shared.file_name, y_name, x_name),
                &x_unit,
                &y_unit,
                shared.analysis_range(),
                &averages,
                true,
            );
            for (o, rows) in &overlay_averages {
                csv.push_str(&i3rs_core::distribution::x_interval_csv(
                    &o.label, &x_unit, &y_unit, o.range, rows, false,
                ));
            }
            super::calculations::export_buttons(ui, &csv);
        }
        let colored: Option<Vec<Vec<[f64; 2]>>> = self.color_channel.as_ref().map(|c| {
            let meta = resolve_plotted_channel_display_meta(c, shared);
            let mut bands = vec![Vec::new(); self.color_bands.len()];
            let info = build_plotted_channel_info(c, shared);
            for p in points {
                let v = i3rs_core::channels::sample_with_policy(
                    &c.data,
                    meta.2,
                    p.time,
                    info.interpolation,
                )
                .map(|v| transform_channel_value(c, v))
                .unwrap_or(f64::NAN);
                bands[self.color_bands.index(v)].push([p.x, p.y]);
            }
            bands
        });
        let plot_points = &cache.plot_points;

        let allowed_bounds = padded_bounds(cache.data_bounds, self.bounds_padding_frac).map(
            |((x_min, x_max), (y_min, y_max))| {
                PlotBounds::from_min_max([x_min, y_min], [x_max, y_max])
            },
        );
        let plot = if let Some(bounds) = allowed_bounds {
            plot.default_x_bounds(bounds.min()[0], bounds.max()[0])
                .default_y_bounds(bounds.min()[1], bounds.max()[1])
        } else {
            plot
        };

        // Cursor highlight point
        let cursor_point = shared.cursor_time.and_then(|t| {
            let x_val = transform_channel_value(x_ch, interp_at_time(&x_ch.data, x_freq, t)?);
            let y_val = transform_channel_value(y_ch, interp_at_time(&y_ch.data, y_freq, t)?);
            Some([x_val, y_val])
        });
        let mut clicked_cursor_time = None;

        let series_name = format!("{} vs {}", y_name, x_name);

        let plot_response = plot.show(ui, |plot_ui| {
            for (overlay, samples, fit) in &overlay_points {
                if self.show_regression
                    && let Some(r) = fit
                {
                    let low = samples.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
                    let high = samples
                        .iter()
                        .map(|p| p.x)
                        .fold(f64::NEG_INFINITY, f64::max);
                    if high > low {
                        plot_ui.line(
                            Line::new(
                                format!("{} · regression", overlay.label),
                                vec![
                                    [low, low * r.slope + r.intercept],
                                    [high, high * r.slope + r.intercept],
                                ],
                            )
                            .color(overlay.color),
                        );
                    }
                }
                if let Some(c) = overlay.channels.get(2) {
                    let mut bands = vec![Vec::new(); self.color_bands.len()];
                    for p in samples {
                        let value = i3rs_core::channels::sample_with_policy(
                            &c.data,
                            c.freq,
                            p.time,
                            c.interpolation,
                        )
                        .map(|v| c.transform_value(v))
                        .unwrap_or(f64::NAN);
                        bands[self.color_bands.index(value)].push([p.x, p.y]);
                    }
                    for (band, values) in bands.into_iter().enumerate() {
                        if !self.band_presentation.visible(band) {
                            continue;
                        }
                        if self.density {
                            draw_density(
                                plot_ui,
                                &values,
                                cache.data_bounds,
                                self.band_presentation.color(self.color_bands, band),
                                point_size,
                                &format!("{} · {}", overlay.label, self.color_bands.label(band)),
                            );
                            continue;
                        }
                        plot_ui.points(
                            Points::new(
                                format!(
                                    "{} · {} · {} samples",
                                    overlay.label,
                                    self.color_bands.label(band),
                                    values.len()
                                ),
                                values,
                            )
                            .radius(point_size)
                            .filled(false)
                            .color(self.band_presentation.color(self.color_bands, band)),
                        );
                    }
                } else if self.density {
                    draw_density(
                        plot_ui,
                        &samples.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
                        cache.data_bounds,
                        overlay.color,
                        point_size,
                        &overlay.label,
                    );
                } else {
                    plot_ui.points(
                        Points::new(
                            format!("{} · {} samples", overlay.label, samples.len()),
                            PlotPoints::new(samples.iter().map(|p| [p.x, p.y]).collect()),
                        )
                        .radius(point_size)
                        .color(overlay.color),
                    );
                }
            }
            if self.show_interval_averages {
                draw_interval_averages(
                    plot_ui,
                    "X interval averages",
                    &averages,
                    egui::Color32::LIGHT_RED,
                );
                for (o, rows) in &overlay_averages {
                    draw_interval_averages(
                        plot_ui,
                        &format!("{} · averages", o.label),
                        rows,
                        o.color,
                    );
                }
            }
            if let Some(bands) = &colored {
                for (i, values) in bands.iter().enumerate() {
                    if !self.band_presentation.visible(i) {
                        continue;
                    }
                    if self.density {
                        draw_density(
                            plot_ui,
                            values,
                            cache.data_bounds,
                            self.band_presentation.color(self.color_bands, i),
                            point_size,
                            &self.color_bands.label(i),
                        );
                        continue;
                    }
                    plot_ui.points(
                        Points::new(
                            format!("{} · n={}", self.color_bands.label(i), values.len()),
                            PlotPoints::new(values.clone()),
                        )
                        .radius(point_size)
                        .color(self.band_presentation.color(self.color_bands, i)),
                    );
                }
            } else if self.density {
                draw_density(
                    plot_ui,
                    &points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
                    cache.data_bounds,
                    color,
                    point_size,
                    &series_name,
                );
            } else {
                plot_ui.points(
                    Points::new(&series_name, PlotPoints::Borrowed(plot_points.as_slice()))
                        .shape(MarkerShape::Circle)
                        .filled(false)
                        .radius(point_size)
                        .color(color),
                );
            }
            if self.show_regression
                && let (Some(r), Some(((lo, hi), _))) = (&regression, cache.data_bounds)
            {
                plot_ui.line(
                    Line::new(
                        "Regression",
                        vec![
                            [lo, lo * r.slope + r.intercept],
                            [hi, hi * r.slope + r.intercept],
                        ],
                    )
                    .color(egui::Color32::WHITE),
                );
            }

            if let Some(cp) = cursor_point {
                plot_ui.points(
                    Points::new("", PlotPoints::new(vec![cp]))
                        .radius(point_size * 4.0)
                        .color(egui::Color32::WHITE),
                );
            }

            if plot_ui.response().clicked()
                && let Some(pointer_pos) = plot_ui.response().interact_pointer_pos()
                && let Some(point) = nearest_scatter_point(
                    points,
                    pointer_pos,
                    plot_ui,
                    point_size.max(1.0) * 2.5 + 4.0,
                )
            {
                clicked_cursor_time = Some(point.time);
            }

            if self.lock_bounds
                && let Some(allowed_bounds) = allowed_bounds
            {
                let clamped = clamp_plot_bounds(plot_ui.plot_bounds(), allowed_bounds);
                plot_ui.set_plot_bounds(clamped);
            }
        });

        plot_response.response.context_menu(|ui| {
            let mut clear_x = false;
            let mut sync_x_color = false;
            if let Some(channel) = self.x_channel.as_mut() {
                ui.menu_button("X Channel", |ui| {
                    let old_color = channel.color;
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_x = true;
                    } else if channel.color != old_color {
                        sync_x_color = true;
                    }
                });
            }

            let mut clear_y = false;
            let mut sync_y_color = false;
            if let Some(channel) = self.y_channel.as_mut() {
                ui.menu_button("Y Channel", |ui| {
                    let old_color = channel.color;
                    if show_plotted_channel_display_menu(ui, channel, shared) {
                        clear_y = true;
                    } else if channel.color != old_color {
                        sync_y_color = true;
                    }
                });
            }

            if clear_x {
                self.x_channel = None;
                self.cache = None;
                ui.close();
            } else if sync_x_color {
                self.sync_series_color_from_x();
            }
            if clear_y {
                self.y_channel = None;
                self.cache = None;
                ui.close();
            } else if sync_y_color {
                self.sync_series_color_from_y();
            }
        });

        if let Some(cursor_time) = clicked_cursor_time {
            shared.seek(cursor_time);
        }
    }
}

fn data_bounds(points: &[PlotPoint]) -> Option<((f64, f64), (f64, f64))> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for point in points {
        x_min = x_min.min(point.x);
        x_max = x_max.max(point.x);
        y_min = y_min.min(point.y);
        y_max = y_max.max(point.y);
    }

    (x_min.is_finite() && x_max.is_finite() && y_min.is_finite() && y_max.is_finite())
        .then_some(((x_min, x_max), (y_min, y_max)))
}

fn padded_bounds(
    bounds: Option<((f64, f64), (f64, f64))>,
    padding_frac: f64,
) -> Option<((f64, f64), (f64, f64))> {
    let ((x_min, x_max), (y_min, y_max)) = bounds?;
    let x_pad = axis_padding(x_min, x_max, padding_frac);
    let y_pad = axis_padding(y_min, y_max, padding_frac);
    Some((
        (x_min - x_pad, x_max + x_pad),
        (y_min - y_pad, y_max + y_pad),
    ))
}

fn axis_padding(min: f64, max: f64, padding_frac: f64) -> f64 {
    let span = (max - min).abs();
    if span > f64::EPSILON {
        span * padding_frac.max(0.0)
    } else {
        let anchor = max.abs().max(1.0);
        anchor * padding_frac.max(0.0)
    }
}

fn clamp_plot_bounds(current: PlotBounds, allowed: PlotBounds) -> PlotBounds {
    PlotBounds::from_min_max(
        [
            clamp_axis_bounds(
                current.min()[0],
                current.max()[0],
                allowed.min()[0],
                allowed.max()[0],
            )
            .0,
            clamp_axis_bounds(
                current.min()[1],
                current.max()[1],
                allowed.min()[1],
                allowed.max()[1],
            )
            .0,
        ],
        [
            clamp_axis_bounds(
                current.min()[0],
                current.max()[0],
                allowed.min()[0],
                allowed.max()[0],
            )
            .1,
            clamp_axis_bounds(
                current.min()[1],
                current.max()[1],
                allowed.min()[1],
                allowed.max()[1],
            )
            .1,
        ],
    )
}

fn clamp_axis_bounds(
    current_min: f64,
    current_max: f64,
    allowed_min: f64,
    allowed_max: f64,
) -> (f64, f64) {
    let allowed_span = allowed_max - allowed_min;
    let current_span = current_max - current_min;

    if !allowed_span.is_finite() || allowed_span <= 0.0 {
        return (current_min, current_max);
    }

    if !current_span.is_finite() || current_span >= allowed_span {
        return (allowed_min, allowed_max);
    }

    let mut min = current_min;
    let mut max = current_max;

    if min < allowed_min {
        let delta = allowed_min - min;
        min += delta;
        max += delta;
    }
    if max > allowed_max {
        let delta = max - allowed_max;
        min -= delta;
        max -= delta;
    }

    (min, max)
}

#[cfg(test)]
/// Build scatter plot points by resampling both channels to a common frequency.
fn build_scatter_points(
    x_channel: &PlottedChannel,
    x_freq: u16,
    y_channel: &PlottedChannel,
    y_freq: u16,
    target_freq: u16,
    t0: f64,
    t1: f64,
) -> Vec<ScatterPoint> {
    if target_freq == 0
        || x_freq == 0
        || y_freq == 0
        || !t0.is_finite()
        || !t1.is_finite()
        || t1 <= t0
    {
        return Vec::new();
    }

    let start_sample = (t0.max(0.0) * target_freq as f64).ceil() as usize;
    let end_sample = (t1 * target_freq as f64).ceil() as usize;
    let end_sample = end_sample
        .min((x_channel.data.len() as f64 * target_freq as f64 / x_freq as f64) as usize)
        .min((y_channel.data.len() as f64 * target_freq as f64 / y_freq as f64) as usize);

    (start_sample..end_sample)
        .filter_map(|i| {
            let time = i as f64 / target_freq as f64;
            let x =
                transform_channel_value(x_channel, interp_at_time(&x_channel.data, x_freq, time)?);
            let y =
                transform_channel_value(y_channel, interp_at_time(&y_channel.data, y_freq, time)?);
            (x.is_finite() && y.is_finite()).then_some(ScatterPoint { x, y, time })
        })
        .collect()
}

fn nearest_scatter_point(
    points: &[ScatterPoint],
    pointer_pos: egui::Pos2,
    plot_ui: &egui_plot::PlotUi<'_>,
    max_dist_px: f32,
) -> Option<ScatterPoint> {
    let max_dist_sq = max_dist_px * max_dist_px;
    points
        .iter()
        .copied()
        .filter_map(|point| {
            let screen_pos = plot_ui.screen_from_plot(PlotPoint::new(point.x, point.y));
            let dist_sq = screen_pos.distance_sq(pointer_pos);
            (dist_sq <= max_dist_sq).then_some((point, dist_sq))
        })
        .min_by(|(_, da), (_, db)| da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(point, _)| point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::YAxis;
    use std::sync::Arc;
    fn sample_channel(data: Arc<[f64]>) -> PlottedChannel {
        PlottedChannel {
            channel_id: ChannelId::Physical(0),
            color: egui::Color32::WHITE,
            data,
            tile_group: 0,
            y_axis: YAxis::Left,
            display_scale: 1.0,
            display_offset: 0.0,
            display_unit: None,
            math_id: None,
            requested_reference: None,
            portable_source_unit: None,
            recording_display_base: None,
            math_display_signature: None,
            scale_mode: crate::state::ScaleMode::Auto,
            manual_min: 0.0,
            manual_max: 1.0,
            cached_min: 0.0,
            cached_max: 1.0,
            cached_avg: 0.5,
        }
    }

    #[test]
    fn faster_samples_are_preserved_and_empty_ranges_are_safe() {
        let x = sample_channel(Arc::from(vec![0., 1., 2.]));
        let y = sample_channel(Arc::from(vec![0., 9., 0., 8., 0., 7.]));
        let points = build_scatter_points(&x, 1, &y, 2, 2, 0., 3.);
        assert_eq!(points.len(), 6);
        assert_eq!(points[1].x, 0.5);
        assert_eq!(points[1].y, 9.);
        assert!(build_scatter_points(&x, 1, &y, 2, 2, 10., 11.).is_empty());
        assert!(build_scatter_points(&x, 0, &y, 2, 2, 0., 1.).is_empty());
        let long = sample_channel(Arc::from(vec![1.; 120_001]));
        assert_eq!(
            build_scatter_points(&long, 1, &long, 1, 1, 0., 120_001.).len(),
            120_001
        );
    }
    #[test]
    #[ignore = "requires local i2 comparison recording"]
    fn i2_oil_scatter_counts() {
        let ld = i3rs_core::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../S1_#28299_20260508_174455_02.ld"
        ))
        .unwrap();
        let x = ld
            .find_channel_by_name("CP Lotus ESP Lateral Acceleration")
            .unwrap();
        let y = ld.find_channel_by_name("Engine Oil Pressure").unwrap();
        let xc = sample_channel(Arc::from(ld.read_channel_data(x).unwrap()));
        let yc = sample_channel(Arc::from(ld.read_channel_data(y).unwrap()));
        let points =
            build_scatter_points(&xc, x.freq, &yc, y.freq, x.freq.max(y.freq), 0., 351.680);
        println!(
            "Oil samples {}, first {:.3}, last {:.3}",
            points.len(),
            points[0].time,
            points.last().unwrap().time
        );
        assert_eq!(points.len(), 17576);
        let laps = i3rs_core::detect_laps(&ld);
        let lap = &laps[3];
        let timed = build_scatter_points(
            &xc,
            x.freq,
            &yc,
            y.freq,
            x.freq.max(y.freq),
            lap.start_time,
            lap.end_time,
        );
        assert_eq!(timed.len(), 6706);
    }
}

fn density_cells(
    points: &[[f64; 2]],
    bounds: Option<((f64, f64), (f64, f64))>,
) -> Vec<([f64; 2], usize)> {
    let Some(((x0, x1), (y0, y1))) = bounds else {
        return Vec::new();
    };
    let mut cells = std::collections::BTreeMap::<(usize, usize), (f64, f64, usize)>::new();
    for &[x, y] in points {
        let key = (
            ((x - x0) / (x1 - x0).max(f64::EPSILON) * 100.).clamp(0., 99.) as usize,
            ((y - y0) / (y1 - y0).max(f64::EPSILON) * 100.).clamp(0., 99.) as usize,
        );
        let cell = cells.entry(key).or_default();
        cell.0 += x;
        cell.1 += y;
        cell.2 += 1;
    }
    cells
        .values()
        .map(|(x, y, n)| ([x / *n as f64, y / *n as f64], *n))
        .collect()
}
fn draw_density(
    plot: &mut egui_plot::PlotUi,
    points: &[[f64; 2]],
    bounds: Option<((f64, f64), (f64, f64))>,
    color: egui::Color32,
    size: f32,
    name: &str,
) {
    let cells = density_cells(points, bounds);
    let maximum = cells.iter().map(|(_, n)| *n).max().unwrap_or(1) as f32;
    // Bucket equal occupancies to keep rendering bounded without losing weights.
    let mut groups = std::collections::BTreeMap::<usize, Vec<[f64; 2]>>::new();
    for (point, n) in cells {
        groups.entry(n).or_default().push(point);
    }
    for (n, points) in groups {
        let weight = (1. + n as f32).ln() / (1. + maximum).ln();
        plot.points(
            Points::new(format!("{name} · {n} samples/cell"), points)
                .radius(size * (1. + weight * 3.))
                .color(color.gamma_multiply(0.2 + 0.8 * weight)),
        );
    }
}

fn scatter_info_points(
    x: &crate::state::PlottedChannelInfo,
    y: &crate::state::PlottedChannelInfo,
    hz: u16,
    range: (f64, f64),
) -> Vec<ScatterPoint> {
    if hz == 0 || x.freq == 0 || y.freq == 0 {
        return Vec::new();
    }
    let duration = (x.data.len() as f64 / x.freq as f64).min(y.data.len() as f64 / y.freq as f64);
    let window = i3rs_core::sample_window_for_time(
        hz,
        (duration * hz as f64).ceil() as usize,
        range.0,
        range.1.min(duration),
    );
    (window.start_sample..window.end_sample)
        .filter_map(|i| {
            let time = i as f64 / hz as f64;
            let sample = |info: &crate::state::PlottedChannelInfo| {
                i3rs_core::channels::sample_with_policy(
                    &info.data,
                    info.freq,
                    time,
                    info.interpolation,
                )
                .map(|v| info.transform_value(v))
            };
            let (x, y) = (sample(x)?, sample(y)?);
            (x.is_finite() && y.is_finite()).then_some(ScatterPoint { x, y, time })
        })
        .collect()
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn density_occupancies_sum_to_native_population() {
        let points = [[0., 0.], [0., 0.], [1., 1.], [1., 1.], [1., 1.]];
        let cells = density_cells(&points, Some(((0., 1.), (0., 1.))));
        assert_eq!(cells.len(), 2);
        assert_eq!(cells.iter().map(|(_, n)| *n).sum::<usize>(), 5);
    }
}

fn regression_rows(
    name: &str,
    r: &i3rs_core::distribution::Regression,
    range: (f64, f64),
    x_unit: &str,
    y_unit: &str,
) -> Vec<super::calculations::ResultRow> {
    [
        ("slope", Some(r.slope)),
        ("intercept", Some(r.intercept)),
        ("R squared", r.r_squared),
    ]
    .into_iter()
    .map(|(result, value)| super::calculations::ResultRow {
        channel: name.into(),
        result: result.into(),
        start: range.0,
        end: range.1,
        count: r.count,
        value,
        unit: match result {
            "slope" => format!("{y_unit}/({x_unit})"),
            "intercept" => y_unit.into(),
            _ => String::new(),
        },
    })
    .collect()
}

#[cfg(test)]
mod export_tests {
    use super::*;
    #[test]
    fn regression_export_includes_source_range_population_and_units() {
        let r =
            i3rs_core::distribution::linear_regression([(1., 5.), (2., 8.), (3., 11.)]).unwrap();
        let rows = regression_rows(
            "Overlay A · pressure vs speed",
            &r,
            (10., 13.),
            "m/s",
            "kPa",
        );
        let csv = super::super::calculations::result_csv(&rows);
        assert_eq!(rows.len(), 3);
        assert!(csv.contains("10.000000000,13.000000000,3,3.000000000000,\"kPa/(m/s)\""));
        assert!(csv.contains("\"R squared\",10.000000000,13.000000000,3,1.000000000000"));
    }
}

fn draw_interval_averages(
    plot: &mut egui_plot::PlotUi,
    label: &str,
    rows: &[i3rs_core::distribution::XIntervalAverage],
    color: egui::Color32,
) {
    let points: Vec<_> = rows
        .iter()
        .map(|r| [(r.lower + r.upper) / 2., r.mean_y])
        .collect();
    plot.line(
        Line::new(label, points.clone())
            .color(color)
            .style(egui_plot::LineStyle::dashed_dense()),
    );
    plot.points(
        Points::new(label, points)
            .shape(MarkerShape::Diamond)
            .radius(4.)
            .color(color),
    );
}

fn quoted_reference(name: &str) -> Option<String> {
    if !name.contains('"') {
        Some(format!("\"{name}\""))
    } else if !name.contains('\'') {
        Some(format!("'{name}'"))
    } else {
        None
    }
}
fn channel_expression(pc: &PlottedChannel, shared: &SharedState) -> Option<String> {
    let (name, unit, _, _) = super::utils::resolve_channel_meta(pc.channel_id, shared);
    let reference = pc
        .requested_reference
        .as_ref()
        .map_or(name.as_str(), |r| r.0.as_str());
    let quoted = quoted_reference(reference)?;
    let explicit_unit = if unit.is_empty() {
        quoted
    } else {
        format!("{quoted}[{unit}]")
    };
    Some(format!(
        "(({explicit_unit})*{:.15}+{:.15})",
        pc.display_scale, pc.display_offset
    ))
}
fn regression_gate_expression(gate: &str, trusted: Option<&[(f64, f64)]>) -> Option<String> {
    let base = if gate.trim().is_empty() {
        "1".into()
    } else {
        quoted_reference(gate.trim())?
    };
    Some(if let Some(ranges) = trusted {
        let mask = ranges.iter().fold("0".to_owned(), |acc, (a, b)| {
            format!("max({acc},((time()>={a:.12})*(time()<{b:.12})))")
        });
        format!("({base})*({mask})")
    } else {
        base
    })
}

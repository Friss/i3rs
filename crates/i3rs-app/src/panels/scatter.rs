//! Scatter/XY plot panel: channel vs channel visualization.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Legend, MarkerShape, Plot, PlotBounds, PlotPoint, PlotPoints, Points};

use crate::state::{CHANNEL_COLORS, ChannelId, PlottedChannel, SharedState};

use super::utils::{
    build_plotted_channel_info, create_plotted_channel, interp_at_time, resolve_channel_meta,
};

struct ScatterCache {
    fingerprint: (usize, usize, usize, usize, Option<(u64, u64)>),
    points: Vec<ScatterPoint>,
}

#[derive(Clone, Copy)]
struct ScatterPoint {
    x: f64,
    y: f64,
    time: f64,
}

pub struct ScatterPanel {
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

        // Toolbar
        ui.horizontal(|ui| {
            let x_name = self
                .x_channel
                .as_ref()
                .map(|c| resolve_channel_meta(c.channel_id, shared).0)
                .unwrap_or_else(|| "Drop X channel".into());
            let y_name = self
                .y_channel
                .as_ref()
                .map(|c| resolve_channel_meta(c.channel_id, shared).0)
                .unwrap_or_else(|| "Drop Y channel".into());

            ui.label("X:");
            let x_resp = ui.button(&x_name);
            if x_resp.secondary_clicked() {
                self.x_channel = None;
            }
            x_resp.on_hover_text("Right-click to clear");

            ui.label("Y:");
            let y_resp = ui.button(&y_name);
            if y_resp.secondary_clicked() {
                self.y_channel = None;
            }
            y_resp.on_hover_text("Right-click to clear");

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

        let (Some(x_ch), Some(y_ch)) = (&self.x_channel, &self.y_channel) else {
            ui.centered_and_justified(|ui| {
                ui.label("Drop two channels: first for X axis, second for Y axis");
            });
            return;
        };

        let (x_name, x_unit, x_freq, _) = resolve_channel_meta(x_ch.channel_id, shared);
        let (y_name, y_unit, y_freq, _) = resolve_channel_meta(y_ch.channel_id, shared);

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

        // Build scatter points, resampling to the lower frequency
        let target_freq = x_freq.min(y_freq);
        let color = CHANNEL_COLORS[4]; // purple for scatter

        // Get time range for visible data
        let (t0, t1) = shared
            .zoom_range
            .unwrap_or_else(|| (0.0, shared.data_duration.unwrap_or(0.0)));

        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let fingerprint = (
            Arc::as_ptr(&x_ch.data) as usize,
            x_ch.data.len(),
            Arc::as_ptr(&y_ch.data) as usize,
            y_ch.data.len(),
            zoom_key,
        );

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let points =
                build_scatter_points(&x_ch.data, x_freq, &y_ch.data, y_freq, target_freq, t0, t1);
            self.cache = Some(ScatterCache {
                fingerprint,
                points,
            });
        }
        let points = &self.cache.as_ref().unwrap().points;
        let plot_points: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();

        let allowed_bounds = padded_bounds(&plot_points, self.bounds_padding_frac).map(
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
            let x_val = interp_at_time(&x_ch.data, x_freq, t)?;
            let y_val = interp_at_time(&y_ch.data, y_freq, t)?;
            Some([x_val, y_val])
        });
        let mut clicked_cursor_time = None;

        let series_name = format!("{} vs {}", y_name, x_name);

        plot.show(ui, |plot_ui| {
            plot_ui.points(
                Points::new(&series_name, PlotPoints::new(plot_points.clone()))
                    .shape(MarkerShape::Circle)
                    .filled(false)
                    .radius(point_size)
                    .color(color),
            );

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

            if self.lock_bounds && let Some(allowed_bounds) = allowed_bounds {
                let clamped = clamp_plot_bounds(plot_ui.plot_bounds(), allowed_bounds);
                plot_ui.set_plot_bounds(clamped);
            }
        });

        if let Some(cursor_time) = clicked_cursor_time {
            shared.cursor_time = Some(cursor_time);
        }
    }
}

fn padded_bounds(
    points: &[[f64; 2]],
    padding_frac: f64,
) -> Option<((f64, f64), (f64, f64))> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for [x, y] in points {
        x_min = x_min.min(*x);
        x_max = x_max.max(*x);
        y_min = y_min.min(*y);
        y_max = y_max.max(*y);
    }

    if !x_min.is_finite() || !x_max.is_finite() || !y_min.is_finite() || !y_max.is_finite() {
        return None;
    }

    let x_pad = axis_padding(x_min, x_max, padding_frac);
    let y_pad = axis_padding(y_min, y_max, padding_frac);
    Some(((x_min - x_pad, x_max + x_pad), (y_min - y_pad, y_max + y_pad)))
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

/// Build scatter plot points by resampling both channels to a common frequency.
fn build_scatter_points(
    x_data: &[f64],
    x_freq: u16,
    y_data: &[f64],
    y_freq: u16,
    target_freq: u16,
    t0: f64,
    t1: f64,
) -> Vec<ScatterPoint> {
    if target_freq == 0 {
        return Vec::new();
    }

    let start_sample = (t0 * target_freq as f64).floor() as usize;
    let end_sample = (t1 * target_freq as f64).ceil() as usize;
    let end_sample = end_sample
        .min((x_data.len() as f64 * target_freq as f64 / x_freq.max(1) as f64) as usize)
        .min((y_data.len() as f64 * target_freq as f64 / y_freq.max(1) as f64) as usize);

    // Limit to a reasonable number of points for rendering
    let max_points = 50_000;
    let step = ((end_sample - start_sample) / max_points).max(1);

    let mut points = Vec::new();
    let mut i = start_sample;
    while i < end_sample {
        let t = i as f64 / target_freq as f64;
        if let (Some(x), Some(y)) = (
            interp_at_time(x_data, x_freq, t),
            interp_at_time(y_data, y_freq, t),
        ) && x.is_finite()
            && y.is_finite()
        {
            points.push(ScatterPoint { x, y, time: t });
        }
        i += step;
    }
    points
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

//! Mixture map panel: 2D heatmap (e.g., AFR vs RPM vs TPS).
//!
//! User drops three channels: X axis, Y axis, and value (color).
//! The panel bins the data into a 2D grid and colors each cell by the
//! average value channel reading in that bin.

use std::sync::Arc;

use eframe::egui;

use crate::state::{ChannelId, PlottedChannel, PlottedChannelInfo, SharedState};

use super::utils::{create_plotted_channel, interp_at_time, resolve_channel_meta};

struct HeatmapCache {
    fingerprint: (usize, usize, usize, Option<(u64, u64)>, usize),
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
    cache: Option<HeatmapCache>,
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

        // Toolbar
        ui.horizontal(|ui| {
            let x_name = self
                .x_channel
                .as_ref()
                .map(|c| resolve_channel_meta(c.channel_id, shared).0)
                .unwrap_or_else(|| "Drop X".into());
            let y_name = self
                .y_channel
                .as_ref()
                .map(|c| resolve_channel_meta(c.channel_id, shared).0)
                .unwrap_or_else(|| "Drop Y".into());
            let v_name = self
                .value_channel
                .as_ref()
                .map(|c| resolve_channel_meta(c.channel_id, shared).0)
                .unwrap_or_else(|| "Drop Value".into());

            ui.label("X:");
            let xr = ui.button(&x_name);
            if xr.secondary_clicked() {
                self.x_channel = None;
            }
            xr.on_hover_text("Right-click to clear");

            ui.label("Y:");
            let yr = ui.button(&y_name);
            if yr.secondary_clicked() {
                self.y_channel = None;
            }
            yr.on_hover_text("Right-click to clear");

            ui.label("Value:");
            let vr = ui.button(&v_name);
            if vr.secondary_clicked() {
                self.value_channel = None;
            }
            vr.on_hover_text("Right-click to clear");

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
            let (name, unit, freq, dec_places) = resolve_channel_meta(pc.channel_id, shared);
            shared.plotted_channel_registry.push(PlottedChannelInfo {
                name,
                unit,
                freq,
                dec_places,
                color: pc.color,
                data: pc.data.clone(),
            });
        }

        let (Some(x_ch), Some(y_ch), Some(v_ch)) =
            (&self.x_channel, &self.y_channel, &self.value_channel)
        else {
            ui.centered_and_justified(|ui| {
                ui.label("Drop three channels: X axis, Y axis, then Value (color)");
            });
            return;
        };

        let (x_name, x_unit, x_freq, _) = resolve_channel_meta(x_ch.channel_id, shared);
        let (y_name, y_unit, y_freq, _) = resolve_channel_meta(y_ch.channel_id, shared);
        let (v_name, _, v_freq, _) = resolve_channel_meta(v_ch.channel_id, shared);

        // Compute the heatmap
        let target_freq = x_freq.min(y_freq).min(v_freq);
        if target_freq == 0 {
            ui.label("Channel frequency is 0");
            return;
        }

        let (t0, t1) = shared
            .zoom_range
            .unwrap_or_else(|| (0.0, shared.data_duration.unwrap_or(0.0)));

        let zoom_key = shared.zoom_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
        let fingerprint = (
            Arc::as_ptr(&x_ch.data) as usize,
            Arc::as_ptr(&y_ch.data) as usize,
            Arc::as_ptr(&v_ch.data) as usize,
            zoom_key,
            self.bins,
        );

        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.fingerprint != fingerprint)
        {
            let heatmap = compute_heatmap(
                &x_ch.data,
                x_freq,
                &y_ch.data,
                y_freq,
                &v_ch.data,
                v_freq,
                target_freq,
                t0,
                t1,
                self.bins,
            );
            self.cache = Some(HeatmapCache {
                fingerprint,
                heatmap,
            });
        }
        let heatmap = &self.cache.as_ref().unwrap().heatmap;

        // Draw the heatmap
        let available = ui.available_rect_before_wrap();
        let margin = 50.0;
        let plot_rect = egui::Rect::from_min_max(
            egui::pos2(available.min.x + margin, available.min.y + 10.0),
            egui::pos2(available.max.x - 20.0, available.max.y - margin),
        );

        if plot_rect.width() < 10.0 || plot_rect.height() < 10.0 {
            return;
        }

        let (_response, painter) = ui.allocate_painter(
            egui::vec2(available.width(), available.height()),
            egui::Sense::hover(),
        );

        let cell_w = plot_rect.width() / self.bins as f32;
        let cell_h = plot_rect.height() / self.bins as f32;

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

                let color = heat_color(frac);
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

        // Outline
        painter.rect_stroke(
            plot_rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
            egui::StrokeKind::Outside,
        );

        // X axis labels
        for i in 0..=4 {
            let frac = i as f64 / 4.0;
            let val = heatmap.x_min + frac * (heatmap.x_max - heatmap.x_min);
            let x = plot_rect.min.x + frac as f32 * plot_rect.width();
            painter.text(
                egui::pos2(x, plot_rect.max.y + 4.0),
                egui::Align2::CENTER_TOP,
                format!("{:.0}", val),
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }

        // Y axis labels
        for i in 0..=4 {
            let frac = i as f64 / 4.0;
            let val = heatmap.y_min + frac * (heatmap.y_max - heatmap.y_min);
            let y = plot_rect.max.y - frac as f32 * plot_rect.height();
            painter.text(
                egui::pos2(plot_rect.min.x - 4.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0}", val),
                egui::FontId::proportional(10.0),
                egui::Color32::GRAY,
            );
        }

        // Axis labels
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
        // Vertical Y label
        let y_label_pos = egui::pos2(available.min.x + 8.0, plot_rect.center().y);
        // Draw chars vertically
        let char_height = 13.0;
        let total_h = y_label.len() as f32 * char_height;
        let start_y = y_label_pos.y - total_h / 2.0;
        for (i, c) in y_label.chars().enumerate() {
            painter.text(
                egui::pos2(y_label_pos.x, start_y + i as f32 * char_height),
                egui::Align2::CENTER_TOP,
                c.to_string(),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
        }

        // Color scale legend
        let legend_x = plot_rect.max.x + 5.0;
        let legend_h = plot_rect.height().min(150.0);
        let legend_top = plot_rect.center().y - legend_h / 2.0;
        let legend_w = 10.0;
        for i in 0..20 {
            let frac = i as f32 / 19.0;
            let color = heat_color(1.0 - frac); // top = high
            let y = legend_top + frac * legend_h;
            let r = egui::Rect::from_min_size(
                egui::pos2(legend_x, y),
                egui::vec2(legend_w, legend_h / 20.0 + 1.0),
            );
            painter.rect_filled(r, 0.0, color);
        }
        painter.text(
            egui::pos2(legend_x + legend_w + 2.0, legend_top),
            egui::Align2::LEFT_TOP,
            format!("{:.1}", heatmap.value_min + heatmap.value_range),
            egui::FontId::proportional(9.0),
            egui::Color32::GRAY,
        );
        painter.text(
            egui::pos2(legend_x + legend_w + 2.0, legend_top + legend_h),
            egui::Align2::LEFT_BOTTOM,
            format!("{:.1}", heatmap.value_min),
            egui::FontId::proportional(9.0),
            egui::Color32::GRAY,
        );
        painter.text(
            egui::pos2(legend_x + legend_w / 2.0, legend_top - 4.0),
            egui::Align2::CENTER_BOTTOM,
            &v_name,
            egui::FontId::proportional(10.0),
            egui::Color32::LIGHT_GRAY,
        );

        // Cursor highlight
        if let Some(cursor_time) = shared.cursor_time
            && let (Some(x_val), Some(y_val)) = (
                interp_at_time(&x_ch.data, x_freq, cursor_time),
                interp_at_time(&y_ch.data, y_freq, cursor_time),
            )
        {
            let x_frac = if heatmap.x_max > heatmap.x_min {
                ((x_val - heatmap.x_min) / (heatmap.x_max - heatmap.x_min)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let y_frac = if heatmap.y_max > heatmap.y_min {
                ((y_val - heatmap.y_min) / (heatmap.y_max - heatmap.y_min)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let cx = plot_rect.min.x + x_frac as f32 * plot_rect.width();
            let cy = plot_rect.max.y - y_frac as f32 * plot_rect.height();
            painter.circle_stroke(
                egui::pos2(cx, cy),
                6.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE),
            );
        }
    }
}

struct HeatmapCell {
    sum: f64,
    count: u32,
}

struct Heatmap {
    cells: Vec<HeatmapCell>,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    value_min: f64,
    value_range: f64,
}

#[allow(clippy::too_many_arguments)]
fn compute_heatmap(
    x_data: &[f64],
    x_freq: u16,
    y_data: &[f64],
    y_freq: u16,
    v_data: &[f64],
    v_freq: u16,
    target_freq: u16,
    t0: f64,
    t1: f64,
    bins: usize,
) -> Heatmap {
    let start = (t0 * target_freq as f64).floor() as usize;
    let end = (t1 * target_freq as f64).ceil() as usize;

    // First pass: collect all valid (x, y, v) triples and find ranges
    let mut triples = Vec::new();
    let mut x_min = f64::MAX;
    let mut x_max = f64::MIN;
    let mut y_min = f64::MAX;
    let mut y_max = f64::MIN;
    let mut v_min = f64::MAX;
    let mut v_max = f64::MIN;

    let step = ((end - start) / 100_000).max(1);
    let mut i = start;
    while i < end {
        let t = i as f64 / target_freq as f64;
        if let (Some(x), Some(y), Some(v)) = (
            interp_at_time(x_data, x_freq, t),
            interp_at_time(y_data, y_freq, t),
            interp_at_time(v_data, v_freq, t),
        ) && x.is_finite()
            && y.is_finite()
            && v.is_finite()
        {
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
            triples.push((x, y, v));
        }
        i += step;
    }

    let mut cells: Vec<HeatmapCell> = (0..bins * bins)
        .map(|_| HeatmapCell { sum: 0.0, count: 0 })
        .collect();

    let x_range = x_max - x_min;
    let y_range = y_max - y_min;
    let v_range = v_max - v_min;

    if x_range > 0.0 && y_range > 0.0 {
        for (x, y, v) in &triples {
            let xi = (((x - x_min) / x_range * bins as f64) as usize).min(bins - 1);
            let yi = (((y - y_min) / y_range * bins as f64) as usize).min(bins - 1);
            let cell = &mut cells[yi * bins + xi];
            cell.sum += v;
            cell.count += 1;
        }
    }

    Heatmap {
        cells,
        x_min,
        x_max,
        y_min,
        y_max,
        value_min: v_min,
        value_range: v_range,
    }
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

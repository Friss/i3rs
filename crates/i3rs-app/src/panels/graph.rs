//! Graph panel: time-series plotting with overlay and tiled modes.

use std::collections::HashMap;

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, VLine};
use i3rs_core::{Lap, downsample_minmax, format_state_value, is_state_channel};

use crate::state::{CHANNEL_COLORS, ChannelId, GraphMode, PlottedChannel, SharedState, YAxis};

use super::utils::{
    ChannelDisplayMeta, build_plotted_channel_info, create_plotted_channel,
    resolve_channel_display_meta, resolve_channel_meta,
};

/// Format a value using file-parsed enum labels, falling back to hardcoded labels.
fn format_enum_value(name: &str, enum_labels: &HashMap<i64, String>, value: f64) -> Option<String> {
    if !enum_labels.is_empty() {
        let v = value.round() as i64;
        if let Some(label) = enum_labels.get(&v) {
            return Some(label.clone());
        }
    }
    format_state_value(name, value)
}

fn uses_discrete_values(name: &str, enum_labels: &HashMap<i64, String>) -> bool {
    !enum_labels.is_empty() || is_state_channel(name)
}

/// Build a freq map for a set of plotted channels (used to pass into closures).
fn build_freq_map(channels: &[&PlottedChannel], shared: &SharedState) -> HashMap<ChannelId, u16> {
    channels
        .iter()
        .map(|pc| {
            let freq = match pc.channel_id {
                ChannelId::Physical(idx) => shared
                    .ld_file
                    .as_ref()
                    .and_then(|ld| ld.channels.get(idx))
                    .map_or(0, |ch| ch.freq),
                ChannelId::Math(idx) => shared.math_channels.get(idx).map_or(0, |mc| mc.freq),
            };
            (pc.channel_id, freq)
        })
        .collect()
}

/// Actions from context menus.
enum ContextAction {
    Remove(ChannelId),
    ChangeColor(ChannelId, egui::Color32),
    SetYAxis(ChannelId, YAxis),
}

/// A single graph panel with its own set of plotted channels.
pub struct GraphPanel {
    pub id: u64,
    pub title: String,
    pub plotted_channels: Vec<PlottedChannel>,
    pub colors: Vec<egui::Color32>,
    pub graph_mode: GraphMode,
    /// Set when the first channel is added; consumed on next render to reset zoom.
    needs_zoom_reset: bool,
}

impl GraphPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            plotted_channels: Vec::new(),
            colors: CHANNEL_COLORS.to_vec(),
            graph_mode: GraphMode::Tiled,
            needs_zoom_reset: false,
        }
    }

    pub fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self.is_channel_plotted(channel_id) {
            return;
        }
        let color_idx = self.plotted_channels.len() % self.colors.len();
        if let Some(mut pc) = create_plotted_channel(channel_id, shared, color_idx) {
            pc.color = self.colors[color_idx];
            self.plotted_channels.push(pc);
        }
    }

    pub fn remove_channel(&mut self, channel_id: ChannelId) {
        self.plotted_channels
            .retain(|pc| pc.channel_id != channel_id);
    }

    pub fn toggle_channel(&mut self, channel_id: ChannelId, shared: &SharedState) {
        if self.is_channel_plotted(channel_id) {
            self.remove_channel(channel_id);
        } else {
            self.add_channel(channel_id, shared);
        }
    }

    pub fn is_channel_plotted(&self, channel_id: ChannelId) -> bool {
        self.plotted_channels
            .iter()
            .any(|pc| pc.channel_id == channel_id)
    }

    /// Render the graph panel UI.
    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        // Handle pending channel toggle from browser
        if let Some(ch_id) = shared.pending_toggle_channel.take() {
            let was_empty = self.plotted_channels.is_empty();
            self.toggle_channel(ch_id, shared);
            if was_empty && !self.plotted_channels.is_empty() {
                self.needs_zoom_reset = true;
            }
        }

        // Handle drop from channel browser
        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
            && let Some(ch_id) = shared.dragging_channel.take()
        {
            let was_empty = self.plotted_channels.is_empty();
            self.add_channel(ch_id, shared);
            if was_empty && !self.plotted_channels.is_empty() {
                self.needs_zoom_reset = true;
            }
        }

        if self.plotted_channels.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Click channels in the browser to plot them, or drag and drop");
            });
            return;
        }

        for pc in &self.plotted_channels {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(
                    pc.channel_id,
                    pc.color,
                    pc.data.clone(),
                    shared,
                ));
        }

        let needs_zoom_reset = self.needs_zoom_reset;
        self.needs_zoom_reset = false;

        match self.graph_mode {
            GraphMode::Overlay => self.show_overlay_graph(ui, shared, needs_zoom_reset),
            GraphMode::Tiled => self.show_tiled_graphs(ui, shared, needs_zoom_reset),
        }
    }

    fn show_overlay_graph(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
    ) {
        let cursor_group = egui::Id::new("global_cursor_link");

        let mut plot = Plot::new(format!("overlay_{}", self.id))
            .x_axis_label("Time (s)")
            .allow_drag(egui::Vec2b::new(true, false))
            .allow_zoom(egui::Vec2b::new(true, false))
            .allow_scroll(false)
            .show_axes(true)
            .show_grid(true)
            .y_axis_min_width(60.0)
            .link_cursor(cursor_group, egui::Vec2b::new(true, false));

        if needs_zoom_reset {
            plot = plot
                .include_x(0.0)
                .include_x(shared.data_duration.unwrap_or(1.0))
                .auto_bounds(egui::Vec2b::new(true, true));
        }

        let laps = shared.laps.clone();
        let show_markers = shared.show_lap_markers;
        let cursor_time = shared.cursor_time;
        let plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();

        let mut new_cursor_time = None;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;
        let data_duration = shared.data_duration.unwrap_or(f64::MAX);

        let y_range = Self::compute_y_range(&plotted);
        let freq_map = build_freq_map(&plotted, shared);

        let response = plot.show(ui, |plot_ui| {
            if needs_zoom_reset {
                plot_ui.set_plot_bounds_x(0.0..=data_duration);
            } else if let Some((x_min, x_max)) = zoom_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }

            if let Some((y_min, y_max)) = y_range {
                let padding = if (y_max - y_min).abs() < 1e-10 {
                    1.0
                } else {
                    (y_max - y_min) * 0.05
                };
                plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
            }

            Self::draw_channels(plot_ui, &plotted, &freq_map);

            if show_markers {
                Self::draw_lap_markers(plot_ui, &laps);
            }

            if let Some(t) = cursor_time {
                Self::draw_cursor_line(plot_ui, t);
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                new_cursor_time = Some(coord.x);
            }
        });

        if response.response.hovered()
            && let Some(t) = new_cursor_time
        {
            shared.cursor_time = Some(t);
        }

        Self::draw_legend(
            ui,
            response.response.rect,
            &plotted,
            shared,
            shared.cursor_time,
        );

        if needs_zoom_reset {
            shared.zoom_range = Some((0.0, data_duration));
        } else if !zoom_from_timeline {
            let bounds = response.transform.bounds();
            shared.zoom_range = Some(Self::clamp_x_range(
                bounds.min()[0],
                bounds.max()[0],
                data_duration,
            ));
        }

        self.handle_context_menu(&response.response, shared);
    }

    fn show_tiled_graphs(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        needs_zoom_reset: bool,
    ) {
        let cursor_group = egui::Id::new("global_cursor_link");
        let laps = shared.laps.clone();
        let show_markers = shared.show_lap_markers;
        let cursor_time = shared.cursor_time;
        let zoom_from_timeline = shared.zoom_from_timeline;
        let zoom_range = shared.zoom_range;
        let data_duration = shared.data_duration.unwrap_or(f64::MAX);
        let n = self.plotted_channels.len();

        // Pre-compute metadata for each channel to avoid borrowing shared inside closures
        let channel_meta: Vec<ChannelDisplayMeta> = self
            .plotted_channels
            .iter()
            .map(|pc| resolve_channel_display_meta(pc.channel_id, shared))
            .collect();

        let all_plotted: Vec<&PlottedChannel> = self.plotted_channels.iter().collect();
        let freq_map = build_freq_map(&all_plotted, shared);

        let available_height = ui.available_height();
        let tile_height = (available_height / n as f32).max(80.0);

        let mut any_hovered_cursor: Option<f64> = None;
        let mut hovered_x_bounds: Option<(f64, f64)> = None;
        let mut first_x_bounds: Option<(f64, f64)> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
            .show(ui, |ui| {
                let mut responses = Vec::new();

                for (i, pc) in self.plotted_channels.iter().enumerate() {
                    let (ref name, ref unit, _freq, _dec, _) = channel_meta[i];
                    let plot_id = format!("tile_{}_{}", self.id, i);
                    let y_label = if unit.is_empty() {
                        name.clone()
                    } else {
                        format!("{} ({})", name, unit)
                    };

                    let mut plot = Plot::new(plot_id)
                        .height(tile_height)
                        .y_axis_label(y_label)
                        .allow_drag(egui::Vec2b::new(true, false))
                        .allow_zoom(egui::Vec2b::new(true, false))
                        .allow_scroll(false)
                        .show_axes(true)
                        .show_grid(true)
                        .y_axis_min_width(60.0)
                        .link_cursor(cursor_group, egui::Vec2b::new(true, false));

                    if i == n - 1 {
                        plot = plot.x_axis_label("Time (s)");
                    }

                    if needs_zoom_reset {
                        plot = plot
                            .include_x(0.0)
                            .include_x(data_duration)
                            .auto_bounds(egui::Vec2b::new(true, true));
                    }

                    let single: Vec<&PlottedChannel> = vec![pc];
                    let y_range = Self::compute_y_range(&single);
                    let mut tile_cursor = None;

                    let resp = plot.show(ui, |plot_ui| {
                        if needs_zoom_reset {
                            plot_ui.set_plot_bounds_x(0.0..=data_duration);
                        } else if let Some((x_min, x_max)) = zoom_range {
                            plot_ui.set_plot_bounds_x(x_min..=x_max);
                        }

                        if let Some((y_min, y_max)) = y_range {
                            let padding = if (y_max - y_min).abs() < 1e-10 {
                                1.0
                            } else {
                                (y_max - y_min) * 0.05
                            };
                            plot_ui.set_plot_bounds_y((y_min - padding)..=(y_max + padding));
                        }

                        Self::draw_channels(plot_ui, &single, &freq_map);

                        if show_markers {
                            Self::draw_lap_markers(plot_ui, &laps);
                        }

                        if let Some(t) = cursor_time {
                            Self::draw_cursor_line(plot_ui, t);
                        }

                        if let Some(coord) = plot_ui.pointer_coordinate() {
                            tile_cursor = Some(coord.x);
                        }
                    });

                    let bounds = resp.transform.bounds();
                    let x_pair = (bounds.min()[0], bounds.max()[0]);
                    if first_x_bounds.is_none() {
                        first_x_bounds = Some(x_pair);
                    }

                    if resp.response.hovered() {
                        hovered_x_bounds = Some(x_pair);
                        if let Some(t) = tile_cursor {
                            any_hovered_cursor = Some(t);
                        }
                    }

                    // Draw legend with pre-computed metadata
                    Self::draw_tile_legend(
                        ui,
                        resp.response.rect,
                        pc,
                        &channel_meta[i],
                        cursor_time,
                    );

                    responses.push((pc.channel_id, resp.response));
                }

                for (ch_id, resp) in &responses {
                    self.handle_tile_context_menu_with_meta(resp, *ch_id, &channel_meta);
                }
            });

        if let Some(t) = any_hovered_cursor {
            shared.cursor_time = Some(t);
        }

        if needs_zoom_reset {
            shared.zoom_range = Some((0.0, data_duration));
        } else if !zoom_from_timeline
            && let Some((x_min, x_max)) = hovered_x_bounds.or(first_x_bounds)
        {
            shared.zoom_range = Some(Self::clamp_x_range(x_min, x_max, data_duration));
        }
    }

    /// Clamp an X range to [0, duration], preserving width.
    fn clamp_x_range(x_min: f64, x_max: f64, duration: f64) -> (f64, f64) {
        let width = x_max - x_min;
        if width >= duration {
            return (0.0, duration);
        }
        let mut min = x_min;
        let mut max = x_max;
        if min < 0.0 {
            min = 0.0;
            max = width;
        }
        if max > duration {
            max = duration;
            min = duration - width;
        }
        (min, max)
    }

    /// Compute Y range from cached min/max values (O(n_channels), not O(n_samples)).
    fn compute_y_range(channels: &[&PlottedChannel]) -> Option<(f64, f64)> {
        let mut global_min = f64::MAX;
        let mut global_max = f64::MIN;
        let mut has_data = false;

        for pc in channels {
            if !pc.data.is_empty() {
                if pc.cached_min < global_min {
                    global_min = pc.cached_min;
                }
                if pc.cached_max > global_max {
                    global_max = pc.cached_max;
                }
                has_data = true;
            }
        }

        if has_data {
            Some((global_min, global_max))
        } else {
            None
        }
    }

    fn draw_channels(
        plot_ui: &mut egui_plot::PlotUi,
        channels: &[&PlottedChannel],
        freq_map: &HashMap<ChannelId, u16>,
    ) {
        let bounds = plot_ui.plot_bounds();
        let x_min = bounds.min()[0];
        let x_max = bounds.max()[0];
        let pixels_wide = plot_ui.response().rect.width() as usize;
        let target_width = pixels_wide.max(100);

        for pc in channels {
            let freq = freq_map.get(&pc.channel_id).copied().unwrap_or(0);
            if freq == 0 {
                continue;
            }

            let total_samples = pc.data.len();
            let start_sample = if x_min > 0.0 {
                ((x_min * freq as f64) as usize).min(total_samples)
            } else {
                0
            };
            let end_sample = if x_max > 0.0 {
                ((x_max * freq as f64) as usize + 1).min(total_samples)
            } else {
                total_samples
            };

            if start_sample >= end_sample {
                continue;
            }

            let visible_data = &pc.data[start_sample..end_sample];
            let downsampled = downsample_minmax(visible_data, freq, start_sample, target_width);

            let points: Vec<[f64; 2]> = downsampled
                .iter()
                .map(|p| [p.time, (p.min + p.max) / 2.0])
                .collect();
            let line = Line::new("", PlotPoints::new(points))
                .color(pc.color)
                .width(1.5);
            plot_ui.line(line);
        }
    }

    fn draw_legend(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        channels: &[&PlottedChannel],
        shared: &SharedState,
        cursor_time: Option<f64>,
    ) {
        let line_height = 15.0;
        let pad = 4.0;
        for (i, pc) in channels.iter().enumerate() {
            let meta = resolve_channel_display_meta(pc.channel_id, shared);
            let y = plot_rect.top() + pad + i as f32 * line_height;
            Self::draw_legend_entry(ui, plot_rect, pc, &meta, cursor_time, y);
        }
    }

    fn draw_tile_legend(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        pc: &PlottedChannel,
        meta: &ChannelDisplayMeta,
        cursor_time: Option<f64>,
    ) {
        let pad = 4.0;
        let y = plot_rect.top() + pad;
        Self::draw_legend_entry(ui, plot_rect, pc, meta, cursor_time, y);
    }

    fn draw_legend_entry(
        ui: &egui::Ui,
        plot_rect: egui::Rect,
        pc: &PlottedChannel,
        meta: &ChannelDisplayMeta,
        cursor_time: Option<f64>,
        y: f32,
    ) {
        let (ref name, ref unit, freq, dec_places, ref enum_labels) = *meta;
        let painter = ui.painter();
        let font = egui::FontId::proportional(12.0);
        let pad = 4.0;

        let min_color = egui::Color32::from_rgb(80, 140, 255);
        let max_color = egui::Color32::from_rgb(255, 80, 80);
        let avg_color = egui::Color32::from_rgb(255, 180, 50);

        let swatch = egui::Rect::from_min_size(
            egui::pos2(plot_rect.left() + pad, y + 2.0),
            egui::vec2(8.0, 8.0),
        );
        painter.rect_filled(swatch, 1.0, pc.color);

        let name_x = swatch.right() + 4.0;
        let label = if unit.is_empty() {
            name.clone()
        } else {
            format!("{} [{}]", name, unit)
        };

        let label_rect = painter.text(
            egui::pos2(name_x, y),
            egui::Align2::LEFT_TOP,
            &label,
            font.clone(),
            pc.color,
        );

        let dec = dec_places.max(0) as usize;
        let fmt = |v: f64| -> String {
            format_enum_value(name, enum_labels, v)
                .unwrap_or_else(|| format!("{:.prec$}", v, prec = dec))
        };

        if let Some(t) = cursor_time {
            let val = crate::panels::cursor_readout::value_at_time(
                &pc.data,
                freq,
                t,
                uses_discrete_values(name, enum_labels),
            );
            painter.text(
                egui::pos2(label_rect.right() + 16.0, y),
                egui::Align2::LEFT_TOP,
                fmt(val),
                font.clone(),
                pc.color,
            );
        }

        // Right side: colored min/max/avg stats (i2 style)
        if !pc.data.is_empty() {
            let icon_size = 7.0;
            let spacing = 6.0;
            let mut x = plot_rect.right() - pad;
            let icon_y = y + 3.0;

            // Helper: draw a stat value with a colored square icon, positioned right-to-left
            let draw_stat = |x: &mut f32, value: f64, color: egui::Color32| {
                let text = fmt(value);
                let rect = painter.text(
                    egui::pos2(*x, y),
                    egui::Align2::RIGHT_TOP,
                    &text,
                    font.clone(),
                    color,
                );
                *x = rect.left() - spacing;
                let icon = egui::Rect::from_min_size(
                    egui::pos2(*x - icon_size, icon_y),
                    egui::vec2(icon_size, icon_size),
                );
                painter.rect_filled(icon, 1.0, color);
                *x = icon.left() - spacing * 2.0;
            };

            draw_stat(&mut x, pc.cached_avg, avg_color);

            // Max uses a triangle icon instead of a square
            let max_text = fmt(pc.cached_max);
            let max_rect = painter.text(
                egui::pos2(x, y),
                egui::Align2::RIGHT_TOP,
                &max_text,
                font.clone(),
                max_color,
            );
            x = max_rect.left() - spacing;
            let tri_cx = x - icon_size / 2.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(tri_cx, icon_y),
                    egui::pos2(tri_cx - icon_size / 2.0, icon_y + icon_size),
                    egui::pos2(tri_cx + icon_size / 2.0, icon_y + icon_size),
                ],
                max_color,
                egui::Stroke::NONE,
            ));
            x = tri_cx - icon_size / 2.0 - spacing * 2.0;

            draw_stat(&mut x, pc.cached_min, min_color);
        }
    }

    fn draw_cursor_line(plot_ui: &mut egui_plot::PlotUi, time: f64) {
        let cursor_line = VLine::new("cursor", time)
            .color(egui::Color32::from_rgb(255, 255, 0))
            .width(1.0);
        plot_ui.vline(cursor_line);
    }

    fn draw_lap_markers(plot_ui: &mut egui_plot::PlotUi, laps: &[Lap]) {
        let marker_color = egui::Color32::from_rgba_premultiplied(200, 200, 200, 80);
        for lap in laps {
            let vline = VLine::new(&lap.name, lap.start_time)
                .color(marker_color)
                .width(1.0)
                .style(egui_plot::LineStyle::dashed_dense());
            plot_ui.vline(vline);
        }
    }

    fn handle_context_menu(&mut self, response: &egui::Response, shared: &SharedState) {
        response.context_menu(|ui| {
            ui.label("Channels:");
            ui.separator();

            let mut action: Option<ContextAction> = None;

            for pc in &self.plotted_channels {
                let (name, _, _, _) = resolve_channel_meta(pc.channel_id, shared);
                ui.menu_button(&name, |ui| {
                    if ui.button("Remove").clicked() {
                        action = Some(ContextAction::Remove(pc.channel_id));
                        ui.close();
                    }
                    ui.separator();
                    ui.label("Color:");
                    for (i, &c) in CHANNEL_COLORS.iter().enumerate() {
                        let label = format!("Color {}", i + 1);
                        let resp = ui.selectable_label(pc.color == c, &label);
                        let rect = resp.rect;
                        let swatch = egui::Rect::from_min_size(
                            egui::pos2(rect.right() - 14.0, rect.center().y - 5.0),
                            egui::vec2(10.0, 10.0),
                        );
                        ui.painter().rect_filled(swatch, 2.0, c);
                        if resp.clicked() {
                            action = Some(ContextAction::ChangeColor(pc.channel_id, c));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Move to Left Y-axis").clicked() {
                        action = Some(ContextAction::SetYAxis(pc.channel_id, YAxis::Left));
                        ui.close();
                    }
                    if ui.button("Move to Right Y-axis").clicked() {
                        action = Some(ContextAction::SetYAxis(pc.channel_id, YAxis::Right));
                        ui.close();
                    }
                });
            }

            if let Some(act) = action {
                self.apply_context_action(act);
            }
        });
    }

    fn handle_tile_context_menu_with_meta(
        &mut self,
        response: &egui::Response,
        ch_id: ChannelId,
        all_meta: &[ChannelDisplayMeta],
    ) {
        let name = self
            .plotted_channels
            .iter()
            .position(|pc| pc.channel_id == ch_id)
            .and_then(|i| all_meta.get(i))
            .map(|(n, _, _, _, _)| n.clone())
            .unwrap_or_default();

        response.context_menu(|ui| {
            ui.label(&name);
            ui.separator();

            let mut action: Option<ContextAction> = None;

            if ui.button("Remove").clicked() {
                action = Some(ContextAction::Remove(ch_id));
                ui.close();
            }
            ui.separator();
            ui.label("Color:");
            let current_color = self
                .plotted_channels
                .iter()
                .find(|pc| pc.channel_id == ch_id)
                .map(|pc| pc.color);
            for (i, &c) in CHANNEL_COLORS.iter().enumerate() {
                let label = format!("Color {}", i + 1);
                let resp = ui.selectable_label(current_color == Some(c), &label);
                let rect = resp.rect;
                let swatch = egui::Rect::from_min_size(
                    egui::pos2(rect.right() - 14.0, rect.center().y - 5.0),
                    egui::vec2(10.0, 10.0),
                );
                ui.painter().rect_filled(swatch, 2.0, c);
                if resp.clicked() {
                    action = Some(ContextAction::ChangeColor(ch_id, c));
                    ui.close();
                }
            }

            if let Some(act) = action {
                self.apply_context_action(act);
            }
        });
    }

    fn apply_context_action(&mut self, action: ContextAction) {
        match action {
            ContextAction::Remove(id) => self.remove_channel(id),
            ContextAction::ChangeColor(id, color) => {
                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    pc.color = color;
                }
            }
            ContextAction::SetYAxis(id, axis) => {
                if let Some(pc) = self
                    .plotted_channels
                    .iter_mut()
                    .find(|pc| pc.channel_id == id)
                {
                    pc.y_axis = axis;
                }
            }
        }
    }
}

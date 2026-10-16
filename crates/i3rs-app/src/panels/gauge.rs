//! Gauges panel: analog/digital/bar gauges showing channel values at cursor time.
//! Includes a steering wheel angle widget.

use eframe::egui;

use crate::state::{ChannelId, PlottedChannel, SharedState};

use super::utils::{
    build_plotted_channel_info, create_plotted_channel, refresh_plotted_channel,
    resolve_channel_meta, resolve_plotted_channel_display_meta, uses_discrete_values,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GaugeStyle {
    /// Radial/arc gauge.
    Analog,
    /// Horizontal/vertical bar.
    Bar,
    /// Digital text display.
    Digital,
    /// Steering wheel angle widget.
    SteeringWheel,
}

pub(crate) fn default_style_for_name(name: &str) -> GaugeStyle {
    let name_lower = name.to_lowercase();
    if name_lower.contains("steer") || name_lower.contains("steering") {
        GaugeStyle::SteeringWheel
    } else {
        GaugeStyle::Analog
    }
}

pub struct GaugeChannel {
    pub channel: PlottedChannel,
    pub style: GaugeStyle,
}

pub struct GaugePanel {
    pub id: u64,
    pub title: String,
    pub gauges: Vec<GaugeChannel>,
}

impl GaugePanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            gauges: Vec::new(),
        }
    }

    pub fn clear_channels(&mut self) {
        self.gauges.clear();
    }

    fn add_channel(&mut self, channel_id: ChannelId, shared: &SharedState, style: GaugeStyle) {
        if self
            .gauges
            .iter()
            .any(|g| g.channel.channel_id == channel_id)
        {
            return;
        }
        if let Some(pc) = create_plotted_channel(channel_id, shared, self.gauges.len()) {
            self.gauges.push(GaugeChannel { channel: pc, style });
        }
    }

    fn remove_channel(&mut self, channel_id: ChannelId) {
        self.gauges.retain(|g| g.channel.channel_id != channel_id);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        // Handle drop from channel browser — default to analog gauge
        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
            && let Some(ch_id) = shared.dragging_channel.take()
        {
            // Auto-detect steering wheel by channel name
            let (name, _, _, _) = resolve_channel_meta(ch_id, shared);
            let style = default_style_for_name(&name);
            self.add_channel(ch_id, shared, style);
        }

        // Handle pending toggle
        if let Some(ch_id) = shared.pending_toggle_channel.take() {
            if self.gauges.iter().any(|g| g.channel.channel_id == ch_id) {
                self.remove_channel(ch_id);
            } else {
                self.add_channel(ch_id, shared, GaugeStyle::Analog);
            }
        }

        for gauge in &mut self.gauges {
            refresh_plotted_channel(&mut gauge.channel, shared);
        }

        if self.gauges.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Drag channels here to display as gauges");
            });
            return;
        }

        // Register channels for readout
        for gc in &self.gauges {
            shared
                .plotted_channel_registry
                .push(build_plotted_channel_info(&gc.channel, shared));
        }

        let spacing = 8.0_f32;
        let clip_rect = ui.clip_rect();
        let available_width = ui.available_width().min(clip_rect.width()).max(1.0);
        let available_height = (clip_rect.bottom() - ui.cursor().top()).max(1.0);
        let (cols, gauge_size) = best_gauge_grid(
            self.gauges.len(),
            available_width,
            available_height,
            spacing,
        );

        egui::Grid::new(format!("gauge_grid_{}", self.id))
            .num_columns(cols)
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                let mut context_action = None;

                for (i, gc) in self.gauges.iter().enumerate() {
                    let reading = GaugeReading::new(&gc.channel, shared);

                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(gauge_size, gauge_size),
                        egui::Sense::click(),
                    );

                    // Context menu for style change / removal
                    response.context_menu(|ui| {
                        ui.label(&reading.name);
                        ui.separator();
                        if ui.button("Analog").clicked() {
                            context_action = Some((i, Some(GaugeStyle::Analog)));
                            ui.close();
                        }
                        if ui.button("Bar").clicked() {
                            context_action = Some((i, Some(GaugeStyle::Bar)));
                            ui.close();
                        }
                        if ui.button("Digital").clicked() {
                            context_action = Some((i, Some(GaugeStyle::Digital)));
                            ui.close();
                        }
                        if ui.button("Steering Wheel").clicked() {
                            context_action = Some((i, Some(GaugeStyle::SteeringWheel)));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Remove").clicked() {
                            context_action = Some((i, None));
                            ui.close();
                        }
                    });

                    let painter = ui.painter_at(rect);

                    let ctx = reading.draw_context(gc.channel.color);
                    draw_gauge(&painter, rect, gc.style, &ctx);

                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }

                // Apply context menu action
                if let Some((idx, action)) = context_action {
                    if let Some(style) = action {
                        self.gauges[idx].style = style;
                    } else {
                        self.gauges.remove(idx);
                    }
                }
            });
    }
}

pub(crate) fn best_gauge_grid(
    count: usize,
    available_width: f32,
    available_height: f32,
    spacing: f32,
) -> (usize, f32) {
    let count = count.max(1);
    let available_width = available_width.max(1.0);
    let available_height = available_height.max(1.0);

    let mut best_cols = 1;
    let mut best_size = 1.0_f32;

    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let width_budget = available_width - spacing * (cols.saturating_sub(1)) as f32;
        let height_budget = available_height - spacing * (rows.saturating_sub(1)) as f32;
        if width_budget <= 0.0 || height_budget <= 0.0 {
            continue;
        }

        let candidate = (width_budget / cols as f32)
            .min(height_budget / rows as f32)
            .min(200.0);
        if candidate > best_size {
            best_size = candidate;
            best_cols = cols;
        }
    }

    (best_cols, best_size.max(1.0))
}

/// Display parameters for drawing a gauge.
/// A gauge channel's labels, cursor value and display range, in display units.
pub(crate) struct GaugeReading {
    pub name: String,
    unit: String,
    dec_places: i16,
    value: Option<f64>,
    min: f64,
    max: f64,
}

impl GaugeReading {
    pub(crate) fn new(channel: &PlottedChannel, shared: &SharedState) -> Self {
        let (name, unit, freq, dec_places, enum_labels) =
            resolve_plotted_channel_display_meta(channel, shared);
        let discrete = uses_discrete_values(&name, &enum_labels);
        let value = shared
            .cursor_time
            .map(|t| super::cursor_readout::value_at_time(&channel.data, freq, t, discrete))
            .filter(|value| value.is_finite())
            .map(|v| {
                if discrete {
                    v
                } else {
                    v * channel.display_scale + channel.display_offset
                }
            });
        let mut min = channel.cached_min * channel.display_scale + channel.display_offset;
        let mut max = channel.cached_max * channel.display_scale + channel.display_offset;
        if channel.display_scale < 0.0 {
            std::mem::swap(&mut min, &mut max);
        }
        Self {
            name,
            unit,
            dec_places,
            value,
            min,
            max,
        }
    }

    pub(crate) fn draw_context(&self, color: egui::Color32) -> GaugeDrawContext<'_> {
        GaugeDrawContext {
            name: &self.name,
            unit: &self.unit,
            value: self.value,
            min: self.min,
            max: self.max,
            dec_places: self.dec_places,
            color,
        }
    }
}

pub(crate) struct GaugeDrawContext<'a> {
    pub name: &'a str,
    pub unit: &'a str,
    pub value: Option<f64>,
    pub min: f64,
    pub max: f64,
    pub dec_places: i16,
    pub color: egui::Color32,
}

pub(crate) fn draw_gauge(
    painter: &egui::Painter,
    rect: egui::Rect,
    style: GaugeStyle,
    ctx: &GaugeDrawContext,
) {
    match style {
        GaugeStyle::Analog => draw_analog_gauge(painter, rect, ctx),
        GaugeStyle::Bar => draw_bar_gauge(painter, rect, ctx),
        GaugeStyle::Digital => draw_digital_gauge(painter, rect, ctx),
        GaugeStyle::SteeringWheel => draw_steering_wheel(painter, rect, ctx),
    }
}

fn draw_analog_gauge(painter: &egui::Painter, rect: egui::Rect, ctx: &GaugeDrawContext) {
    let GaugeDrawContext {
        name,
        unit,
        value,
        min,
        max,
        dec_places,
        color,
        ..
    } = *ctx;
    let center = egui::pos2(rect.center().x, rect.center().y + 10.0);
    let radius = rect.width().min(rect.height()) * 0.38;

    // Background arc (270 degrees, from 135° to 405°)
    let start_angle = std::f32::consts::PI * 0.75; // 135 degrees
    let sweep = std::f32::consts::PI * 1.5; // 270 degrees

    let segments = 60;
    for i in 0..segments {
        let t0 = i as f32 / segments as f32;
        let t1 = (i + 1) as f32 / segments as f32;
        let a0 = start_angle + t0 * sweep;
        let a1 = start_angle + t1 * sweep;
        let p0 = center + egui::vec2(a0.cos(), a0.sin()) * radius;
        let p1 = center + egui::vec2(a1.cos(), a1.sin()) * radius;
        painter.line_segment(
            [p0, p1],
            egui::Stroke::new(3.0_f32, egui::Color32::from_gray(60)),
        );
    }

    // Value arc
    if let Some(val) = value {
        let range = max - min;
        let frac = if range > 0.0 {
            ((val - min) / range).clamp(0.0, 1.0) as f32
        } else {
            0.5
        };

        let val_sweep = frac * sweep;
        let val_segments = (frac * segments as f32).ceil() as usize;
        for i in 0..val_segments {
            let t0 = i as f32 / segments as f32;
            let t1 = ((i + 1) as f32 / segments as f32).min(frac);
            let a0 = start_angle + t0 * sweep;
            let a1 = start_angle + t1 * sweep;
            let p0 = center + egui::vec2(a0.cos(), a0.sin()) * radius;
            let p1 = center + egui::vec2(a1.cos(), a1.sin()) * radius;
            painter.line_segment([p0, p1], egui::Stroke::new(4.0_f32, color));
        }

        // Needle
        let needle_angle = start_angle + val_sweep;
        let needle_end =
            center + egui::vec2(needle_angle.cos(), needle_angle.sin()) * (radius * 0.85);
        painter.line_segment(
            [center, needle_end],
            egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
        );
        painter.circle_filled(center, 4.0, egui::Color32::WHITE);
    }

    // Name label
    painter.text(
        egui::pos2(rect.center().x, rect.min.y + 12.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(12.0),
        egui::Color32::LIGHT_GRAY,
    );

    // Value label
    let dec = dec_places.max(0) as usize;
    let value_text = value
        .map(|v| format!("{:.prec$} {}", v, unit, prec = dec))
        .unwrap_or_else(|| "---".into());
    painter.text(
        egui::pos2(rect.center().x, rect.max.y - 14.0),
        egui::Align2::CENTER_CENTER,
        value_text,
        egui::FontId::monospace(16.0),
        color,
    );

    // Min/Max labels
    let min_pos = center + egui::vec2(start_angle.cos(), start_angle.sin()) * (radius + 12.0);
    painter.text(
        min_pos,
        egui::Align2::CENTER_CENTER,
        format!("{:.0}", min),
        egui::FontId::proportional(9.0),
        egui::Color32::GRAY,
    );
    let max_angle = start_angle + sweep;
    let max_pos = center + egui::vec2(max_angle.cos(), max_angle.sin()) * (radius + 12.0);
    painter.text(
        max_pos,
        egui::Align2::CENTER_CENTER,
        format!("{:.0}", max),
        egui::FontId::proportional(9.0),
        egui::Color32::GRAY,
    );
}

fn draw_bar_gauge(painter: &egui::Painter, rect: egui::Rect, ctx: &GaugeDrawContext) {
    let GaugeDrawContext {
        name,
        unit,
        value,
        min,
        max,
        dec_places,
        color,
        ..
    } = *ctx;
    let margin = 10.0;
    let bar_height = 20.0;
    let bar_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + margin, rect.center().y - bar_height / 2.0),
        egui::vec2(rect.width() - 2.0 * margin, bar_height),
    );

    // Background
    painter.rect_filled(bar_rect, 4.0, egui::Color32::from_gray(40));

    // Fill
    if let Some(val) = value {
        let range = max - min;
        let frac = if range > 0.0 {
            ((val - min) / range).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let fill_rect = egui::Rect::from_min_size(
            bar_rect.min,
            egui::vec2(bar_rect.width() * frac as f32, bar_rect.height()),
        );
        painter.rect_filled(fill_rect, 4.0, color);
    }

    // Name
    painter.text(
        egui::pos2(rect.center().x, bar_rect.min.y - 20.0),
        egui::Align2::CENTER_BOTTOM,
        name,
        egui::FontId::proportional(13.0),
        egui::Color32::LIGHT_GRAY,
    );

    // Value
    let dec = dec_places.max(0) as usize;
    let value_text = value
        .map(|v| format!("{:.prec$} {}", v, unit, prec = dec))
        .unwrap_or_else(|| "---".into());
    painter.text(
        egui::pos2(rect.center().x, bar_rect.max.y + 8.0),
        egui::Align2::CENTER_TOP,
        value_text,
        egui::FontId::monospace(16.0),
        color,
    );

    // Min/Max
    painter.text(
        egui::pos2(bar_rect.min.x, bar_rect.min.y - 12.0),
        egui::Align2::LEFT_TOP,
        format!("{:.0}", min),
        egui::FontId::proportional(9.0),
        egui::Color32::GRAY,
    );
    painter.text(
        egui::pos2(bar_rect.max.x, bar_rect.min.y - 12.0),
        egui::Align2::RIGHT_TOP,
        format!("{:.0}", max),
        egui::FontId::proportional(9.0),
        egui::Color32::GRAY,
    );
}

fn draw_digital_gauge(painter: &egui::Painter, rect: egui::Rect, ctx: &GaugeDrawContext) {
    let GaugeDrawContext {
        name,
        unit,
        value,
        dec_places,
        color,
        ..
    } = *ctx;
    // Background
    let inner = rect.shrink(8.0);
    painter.rect_filled(inner, 6.0, egui::Color32::from_gray(25));
    painter.rect_stroke(
        inner,
        6.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(60)),
        egui::StrokeKind::Outside,
    );

    // Name
    painter.text(
        egui::pos2(rect.center().x, inner.min.y + 10.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(13.0),
        egui::Color32::LIGHT_GRAY,
    );

    // Large value
    let dec = dec_places.max(0) as usize;
    let value_text = value
        .map(|v| format!("{:.prec$}", v, prec = dec))
        .unwrap_or_else(|| "---".into());
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        &value_text,
        egui::FontId::monospace(28.0),
        color,
    );

    // Unit
    painter.text(
        egui::pos2(rect.center().x, rect.center().y + 22.0),
        egui::Align2::CENTER_TOP,
        unit,
        egui::FontId::proportional(12.0),
        egui::Color32::GRAY,
    );
}

fn draw_steering_wheel(painter: &egui::Painter, rect: egui::Rect, ctx: &GaugeDrawContext) {
    let GaugeDrawContext {
        name, unit, value, ..
    } = *ctx;
    let style = painter.ctx().global_style();
    let visuals = &style.visuals;
    let size = rect.width().min(rect.height());
    let scale = (size / 200.0).clamp(0.4, 1.5);
    let text_color = visuals.text_color();
    let muted_color = visuals.weak_text_color();
    let accent = if visuals.dark_mode {
        egui::Color32::from_rgb(255, 204, 80)
    } else {
        egui::Color32::from_rgb(145, 85, 0)
    };
    let wheel_color = if value.is_some() {
        visuals.strong_text_color()
    } else {
        muted_color
    };

    // Reserve separate bands for the title, wheel and readout, even in small tiles.
    let mut title = egui::text::LayoutJob::simple(
        name.to_owned(),
        egui::FontId::proportional((12.0 * scale).max(10.0)),
        text_color,
        (rect.width() - 12.0 * scale).max(1.0),
    );
    title.wrap.max_rows = 1;
    title.wrap.break_anywhere = true;
    let title = painter.layout_job(title);
    let title_top = rect.top() + 5.0 * scale;
    painter.galley(
        egui::pos2(rect.center().x - title.size().x * 0.5, title_top),
        title.clone(),
        text_color,
    );

    let readout_bottom = rect.bottom() - 18.0 * scale;
    let readout_size = (24.0 * scale).max(14.0);
    let wheel_top = title_top + title.size().y + 12.0 * scale;
    let wheel_bottom = readout_bottom - readout_size - 9.0 * scale;
    let radius = ((wheel_bottom - wheel_top) * 0.5).min(rect.width() * 0.5 - 12.0 * scale);
    if radius <= 0.0 {
        return;
    }
    let center = egui::pos2(rect.center().x, (wheel_top + wheel_bottom) * 0.5);
    let rim_width = (radius * 0.13).max(2.0);

    // Preserve the existing positive/anticlockwise convention. Display-unit
    // preferences can supply radians, so use the matching angle for rotation.
    let radians = unit.trim().eq_ignore_ascii_case("rad");
    let angle = value.map(|v| if radians { v } else { v.to_radians() });
    let rotation = egui::emath::Rot2::from_angle(-angle.unwrap_or(0.0) as f32);
    let rotate = |x: f32, y: f32| center + rotation * egui::vec2(x, y) * radius;

    // A fixed index at twelve o'clock makes small changes from center visible.
    painter.line_segment(
        [
            center - egui::vec2(0.0, radius + 9.0 * scale),
            center - egui::vec2(0.0, radius + 5.0 * scale),
        ],
        egui::Stroke::new(2.0 * scale, muted_color),
    );
    painter.circle_filled(center, radius, visuals.faint_bg_color);

    for spoke in [
        [(-0.98, -0.10), (-0.98, 0.12), (-0.16, 0.18), (-0.16, -0.13)],
        [(0.16, -0.13), (0.16, 0.18), (0.98, 0.12), (0.98, -0.10)],
        [(-0.15, 0.12), (0.15, 0.12), (0.10, 0.98), (-0.10, 0.98)],
    ] {
        painter.add(egui::Shape::convex_polygon(
            spoke.iter().map(|&(x, y)| rotate(x, y)).collect(),
            wheel_color,
            egui::Stroke::NONE,
        ));
    }
    painter.circle_stroke(center, radius, egui::Stroke::new(rim_width, wheel_color));
    painter.circle_filled(center, radius * 0.24, visuals.panel_fill);
    painter.circle_stroke(
        center,
        radius * 0.24,
        egui::Stroke::new((radius * 0.035).max(1.0), wheel_color),
    );

    // The contrasting rim stripe turns with the wheel; text stays upright.
    if value.is_some() {
        let stripe = (0..=8)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + (i as f32 / 8.0 - 0.5) * 0.18;
                rotate(a.cos(), a.sin())
            })
            .collect();
        painter.add(egui::Shape::line(
            stripe,
            egui::Stroke::new(rim_width + 1.0, accent),
        ));
    }

    let value_text = match value {
        Some(v) if radians => format!("{v:+.2} rad"),
        Some(v) => format!("{v:+.1}°"),
        None => "—".into(),
    };
    let value_color = if value.is_some() { accent } else { muted_color };
    let mut readout = egui::text::LayoutJob::simple(
        value_text,
        egui::FontId::monospace(readout_size),
        value_color,
        (rect.width() - 8.0 * scale).max(1.0),
    );
    readout.wrap.max_rows = 1;
    readout.wrap.break_anywhere = true;
    let readout = painter.layout_job(readout);
    painter.galley(
        egui::pos2(
            rect.center().x - readout.size().x * 0.5,
            readout_bottom - readout.size().y,
        ),
        readout,
        value_color,
    );

    let direction = match angle {
        Some(a) if a.abs() < 0.05_f64.to_radians() => "Centered",
        Some(a) if a > 0.0 => "Left",
        Some(_) => "Right",
        None => "No reading",
    };
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 3.0 * scale),
        egui::Align2::CENTER_BOTTOM,
        direction,
        egui::FontId::proportional((11.0 * scale).max(9.0)),
        text_color,
    );
}

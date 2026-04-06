//! Gauges panel: analog/digital/bar gauges showing channel values at cursor time.
//! Includes a steering wheel angle widget.

use eframe::egui;

use crate::state::{ChannelId, PlottedChannel, SharedState};

use super::utils::{
    build_plotted_channel_info, create_plotted_channel, interp_at_time, resolve_channel_meta,
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
            let name_lower = name.to_lowercase();
            let style = if name_lower.contains("steer") || name_lower.contains("steering") {
                GaugeStyle::SteeringWheel
            } else {
                GaugeStyle::Analog
            };
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
                .push(build_plotted_channel_info(
                    gc.channel.channel_id,
                    gc.channel.color,
                    gc.channel.data.clone(),
                    shared,
                ));
        }

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Layout gauges in a grid
                let available_width = ui.available_width();
                let gauge_size = 200.0_f32;
                let cols = (available_width / gauge_size).floor().max(1.0) as usize;

                egui::Grid::new(format!("gauge_grid_{}", self.id))
                    .num_columns(cols)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        let mut context_action = None;

                        for (i, gc) in self.gauges.iter().enumerate() {
                            let (name, unit, freq, dec_places) =
                                resolve_channel_meta(gc.channel.channel_id, shared);
                            let value = shared
                                .cursor_time
                                .and_then(|t| interp_at_time(&gc.channel.data, freq, t));
                            let min = gc.channel.cached_min;
                            let max = gc.channel.cached_max;

                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(gauge_size, gauge_size),
                                egui::Sense::click(),
                            );

                            // Context menu for style change / removal
                            response.context_menu(|ui| {
                                ui.label(&name);
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

                            let ctx = GaugeDrawContext {
                                name: &name,
                                unit: &unit,
                                value,
                                min,
                                max,
                                dec_places,
                                color: gc.channel.color,
                            };

                            match gc.style {
                                GaugeStyle::Analog => draw_analog_gauge(&painter, rect, &ctx),
                                GaugeStyle::Bar => draw_bar_gauge(&painter, rect, &ctx),
                                GaugeStyle::Digital => draw_digital_gauge(&painter, rect, &ctx),
                                GaugeStyle::SteeringWheel => {
                                    draw_steering_wheel(&painter, rect, &ctx)
                                }
                            }

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
            });
    }
}

/// Display parameters for drawing a gauge.
struct GaugeDrawContext<'a> {
    name: &'a str,
    unit: &'a str,
    value: Option<f64>,
    min: f64,
    max: f64,
    dec_places: i16,
    color: egui::Color32,
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
            egui::Stroke::new(3.0, egui::Color32::from_gray(60)),
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
            painter.line_segment([p0, p1], egui::Stroke::new(4.0, color));
        }

        // Needle
        let needle_angle = start_angle + val_sweep;
        let needle_end =
            center + egui::vec2(needle_angle.cos(), needle_angle.sin()) * (radius * 0.85);
        painter.line_segment(
            [center, needle_end],
            egui::Stroke::new(2.0, egui::Color32::WHITE),
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
        egui::pos2(rect.center().x, center.y + radius * 0.5),
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
        egui::pos2(bar_rect.min.x, bar_rect.max.y + 8.0),
        egui::Align2::LEFT_TOP,
        format!("{:.0}", min),
        egui::FontId::proportional(9.0),
        egui::Color32::GRAY,
    );
    painter.text(
        egui::pos2(bar_rect.max.x, bar_rect.max.y + 8.0),
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
        egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
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
        name, value, color, ..
    } = *ctx;
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.35;

    // Wheel rim (circle)
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(6.0, egui::Color32::from_gray(80)),
    );

    // Steering spokes (3-spoke pattern)
    let angle_offset = value.map(|v| (v as f32).to_radians()).unwrap_or(0.0);
    let spoke_angles = [0.0_f32, 2.094, 4.189]; // 0°, 120°, 240°
    let inner_radius = radius * 0.35;

    for &base_angle in &spoke_angles {
        let angle = base_angle + angle_offset;
        let start = center + egui::vec2(angle.cos(), angle.sin()) * inner_radius;
        let end = center + egui::vec2(angle.cos(), angle.sin()) * radius;
        painter.line_segment(
            [start, end],
            egui::Stroke::new(4.0, egui::Color32::from_gray(80)),
        );
    }

    // Center hub
    painter.circle_filled(center, inner_radius, egui::Color32::from_gray(50));
    painter.circle_stroke(
        center,
        inner_radius,
        egui::Stroke::new(2.0, egui::Color32::from_gray(80)),
    );

    // Top marker (fixed, shows straight ahead)
    let marker_pos = center + egui::vec2(0.0, -radius - 10.0);
    painter.text(
        marker_pos,
        egui::Align2::CENTER_BOTTOM,
        "\u{25BC}", // down triangle
        egui::FontId::proportional(12.0),
        color,
    );

    // Rotation indicator: highlight the rim at the current angle
    if let Some(val) = value {
        let angle_rad = (val as f32).to_radians();
        // Draw a bright arc segment at the top position after rotation
        let indicator_pos = center
            + egui::vec2(
                (-angle_rad + std::f32::consts::FRAC_PI_2).cos(),
                (-angle_rad + std::f32::consts::FRAC_PI_2).sin(),
            ) * radius;
        painter.circle_filled(indicator_pos, 5.0, color);
    }

    // Name
    painter.text(
        egui::pos2(rect.center().x, rect.min.y + 6.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(12.0),
        egui::Color32::LIGHT_GRAY,
    );

    // Value text
    let value_text = value
        .map(|v| format!("{:.1}\u{00B0}", v))
        .unwrap_or_else(|| "---".into());
    painter.text(
        egui::pos2(rect.center().x, rect.max.y - 10.0),
        egui::Align2::CENTER_BOTTOM,
        value_text,
        egui::FontId::monospace(14.0),
        color,
    );
}

//! Timeline overview: a thin strip showing the full session with a draggable zoom window.

use eframe::egui;
use i3rs_core::downsample_minmax;

use crate::state::SharedState;

const TIMELINE_HEIGHT: f32 = 50.0;
const HANDLE_WIDTH: f32 = 6.0;

/// Which part of the zoom window is being dragged.
#[derive(Clone, Copy, PartialEq)]
enum DragTarget {
    LeftEdge,
    RightEdge,
    Body,
}

/// Persistent state for the timeline widget.
pub struct TimelinePanel {
    drag_target: Option<DragTarget>,
    drag_start_x: f32,
    drag_start_range: (f64, f64),
}

impl TimelinePanel {
    pub fn new() -> Self {
        Self {
            drag_target: None,
            drag_start_x: 0.0,
            drag_start_range: (0.0, 0.0),
        }
    }

    /// Show the timeline overview strip. Returns true if it was displayed.
    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) -> bool {
        let duration = match shared.data_duration {
            Some(d) if d > 0.0 => d,
            _ => return false,
        };

        let zoom = shared.zoom_range.unwrap_or((0.0, duration));

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TIMELINE_HEIGHT),
            egui::Sense::click_and_drag(),
        );

        if !ui.is_rect_visible(rect) {
            return true;
        }

        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 35));

        // Draw miniature waveform if we have a plotted channel
        self.draw_waveform(&painter, rect, shared, duration);

        // Draw lap markers
        for lap in &shared.laps {
            let x = rect.left() + (lap.start_time / duration) as f32 * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_premultiplied(200, 200, 200, 40),
                ),
            );
        }

        // Zoom window rectangle
        let zoom_left = rect.left() + (zoom.0 / duration) as f32 * rect.width();
        let zoom_right = rect.left() + (zoom.1 / duration) as f32 * rect.width();
        let zoom_rect = egui::Rect::from_x_y_ranges(zoom_left..=zoom_right, rect.y_range());

        // Darken areas outside the zoom window
        let left_dark = egui::Rect::from_x_y_ranges(rect.left()..=zoom_left, rect.y_range());
        let right_dark = egui::Rect::from_x_y_ranges(zoom_right..=rect.right(), rect.y_range());
        let dim = egui::Color32::from_rgba_premultiplied(0, 0, 0, 140);
        painter.rect_filled(left_dark, 0.0, dim);
        painter.rect_filled(right_dark, 0.0, dim);

        // Zoom window border
        painter.rect_stroke(
            zoom_rect,
            0.0,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 180, 255)),
            egui::StrokeKind::Outside,
        );

        // Draw cursor position
        if let Some(t) = shared.cursor_time {
            let cx = rect.left() + (t / duration) as f32 * rect.width();
            painter.line_segment(
                [egui::pos2(cx, rect.top()), egui::pos2(cx, rect.bottom())],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 255, 0)),
            );
        }

        // Handle drag interactions
        self.handle_drag(&response, rect, zoom_rect, shared, duration);

        // Set cursor icon based on hover position
        if let Some(pos) = response.hover_pos() {
            let target = self.hit_test(pos, zoom_rect);
            match target {
                Some(DragTarget::LeftEdge) | Some(DragTarget::RightEdge) => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                Some(DragTarget::Body) => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                None => {}
            }
        }

        true
    }

    fn draw_waveform(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        shared: &SharedState,
        duration: f64,
    ) {
        // Find first available channel data from any graph panel's plotted channels.
        // We access through shared state — for now, use the first channel we can find.
        let ld = match &shared.ld_file {
            Some(ld) => ld,
            None => return,
        };

        // Try to find a good representative channel (prefer something with "Speed" or "RPM")
        let representative_ch = ld
            .channels
            .iter()
            .find(|ch| {
                let name_lower = ch.name.to_lowercase();
                name_lower.contains("speed") || name_lower.contains("rpm")
            })
            .or_else(|| ld.channels.first());

        let ch = match representative_ch {
            Some(ch) => ch,
            None => return,
        };

        let data = match ld.read_channel_data(ch) {
            Some(d) => d,
            None => return,
        };

        if data.is_empty() {
            return;
        }

        let target_width = rect.width() as usize;
        let downsampled = downsample_minmax(&data, ch.freq, 0, target_width);

        if downsampled.is_empty() {
            return;
        }

        // Find value range for normalization
        let mut v_min = f64::MAX;
        let mut v_max = f64::MIN;
        for p in &downsampled {
            v_min = v_min.min(p.min);
            v_max = v_max.max(p.max);
        }
        if (v_max - v_min).abs() < 1e-10 {
            return;
        }

        let color = egui::Color32::from_rgba_premultiplied(100, 180, 255, 80);
        let margin = 4.0;
        let draw_height = rect.height() - margin * 2.0;

        for p in &downsampled {
            let x = rect.left() + (p.time / duration) as f32 * rect.width();
            let y_top = rect.top()
                + margin
                + (1.0 - ((p.max - v_min) / (v_max - v_min)) as f32) * draw_height;
            let y_bot = rect.top()
                + margin
                + (1.0 - ((p.min - v_min) / (v_max - v_min)) as f32) * draw_height;
            painter.line_segment(
                [egui::pos2(x, y_top), egui::pos2(x, y_bot.max(y_top + 0.5))],
                egui::Stroke::new(1.0, color),
            );
        }
    }

    fn hit_test(&self, pos: egui::Pos2, zoom_rect: egui::Rect) -> Option<DragTarget> {
        let left = zoom_rect.left();
        let right = zoom_rect.right();

        if (pos.x - left).abs() < HANDLE_WIDTH {
            Some(DragTarget::LeftEdge)
        } else if (pos.x - right).abs() < HANDLE_WIDTH {
            Some(DragTarget::RightEdge)
        } else if zoom_rect.contains(pos) {
            Some(DragTarget::Body)
        } else {
            None
        }
    }

    fn handle_drag(
        &mut self,
        response: &egui::Response,
        rect: egui::Rect,
        zoom_rect: egui::Rect,
        shared: &mut SharedState,
        duration: f64,
    ) {
        let zoom = shared.zoom_range.unwrap_or((0.0, duration));

        // Signal timeline-driven zoom changes
        if response.dragged() || response.double_clicked() {
            shared.zoom_from_timeline = true;
        }

        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.drag_target = self.hit_test(pos, zoom_rect);
            if self.drag_target.is_none() {
                // Clicked outside zoom window — center the view on click
                let t = ((pos.x - rect.left()) / rect.width()) as f64 * duration;
                let half_width = (zoom.1 - zoom.0) / 2.0;
                let new_start = (t - half_width).clamp(0.0, duration - (zoom.1 - zoom.0));
                let new_end = new_start + (zoom.1 - zoom.0);
                shared.zoom_range = Some((new_start, new_end.min(duration)));
                // Now start dragging the body from this new position
                self.drag_target = Some(DragTarget::Body);
            }
            self.drag_start_x = pos.x;
            self.drag_start_range = shared.zoom_range.unwrap_or((0.0, duration));
        }

        if response.dragged()
            && let (Some(target), Some(pos)) = (self.drag_target, response.interact_pointer_pos())
        {
            let dx_pixels = pos.x - self.drag_start_x;
            let dt = (dx_pixels / rect.width()) as f64 * duration;
            let (start0, end0) = self.drag_start_range;
            let width = end0 - start0;

            match target {
                DragTarget::LeftEdge => {
                    let new_start = (start0 + dt).clamp(0.0, end0 - 0.1);
                    shared.zoom_range = Some((new_start, end0));
                }
                DragTarget::RightEdge => {
                    let new_end = (end0 + dt).clamp(start0 + 0.1, duration);
                    shared.zoom_range = Some((start0, new_end));
                }
                DragTarget::Body => {
                    let mut new_start = start0 + dt;
                    if new_start < 0.0 {
                        new_start = 0.0;
                    }
                    if new_start + width > duration {
                        new_start = duration - width;
                    }
                    shared.zoom_range = Some((new_start, new_start + width));
                }
            }
        }

        if response.drag_stopped() {
            self.drag_target = None;
        }

        // Double-click to reset zoom
        if response.double_clicked() {
            shared.zoom_range = Some((0.0, duration));
        }
    }
}

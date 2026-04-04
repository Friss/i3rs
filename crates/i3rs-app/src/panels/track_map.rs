//! Track map panel: GPS track visualization with rainbow coloring and sector editing.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Line, MarkerShape, Plot, PlotPoints, Points};
use i3rs_core::{
    Sector, SectorTime, TrackData, compute_color_map, compute_sector_times, extract_gps_track,
    find_nearest_sample,
};

use crate::state::{CHANNEL_COLORS, SharedState};

pub struct TrackMapPanel {
    pub id: u64,
    pub title: String,
    track_data: Option<Arc<TrackData>>,
    /// Channel index for rainbow coloring (None = solid color).
    pub color_channel_idx: Option<usize>,
    /// Cached per-sample RGBA colors (wrapped in Arc to avoid per-frame clone).
    cached_colors: Option<Arc<Vec<[u8; 4]>>>,
    cached_color_range: Option<(f64, f64)>,
    color_channel_name: String,
    editing_sectors: bool,
    pending_sector_start: Option<usize>,
    /// Cached sector time report (invalidated when sectors/laps change).
    cached_sector_times: Option<CachedSectorReport>,
    /// Fingerprint for track/color cache invalidation.
    cache_fingerprint: Option<(usize, Option<usize>)>,
    /// Search filter for the color channel dropdown.
    color_filter: String,
    /// Whether this panel is currently in a popped-out OS window.
    pub is_popped_out: bool,
    /// Set by the "Pop Out" button; consumed by App to move panel to its own window.
    pub pop_out_requested: bool,
    /// Set by the "Dock" button or window close; consumed by App to move panel back to dock.
    pub dock_requested: bool,
}

struct CachedSectorReport {
    sector_count: usize,
    lap_count: usize,
    times: Vec<Vec<SectorTime>>,
}

impl TrackMapPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            track_data: None,
            color_channel_idx: None,
            cached_colors: None,
            cached_color_range: None,
            color_channel_name: String::new(),
            editing_sectors: false,
            pending_sector_start: None,
            cached_sector_times: None,
            cache_fingerprint: None,
            color_filter: String::new(),
            is_popped_out: false,
            pop_out_requested: false,
            dock_requested: false,
        }
    }

    /// Clear cached data (called when a new file is opened).
    pub fn clear_cache(&mut self) {
        self.track_data = None;
        self.cached_colors = None;
        self.cached_color_range = None;
        self.cached_sector_times = None;
        self.color_channel_idx = None;
        self.cache_fingerprint = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        // If popped out, handle OS window close → dock back
        if self.is_popped_out && ui.input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dock_requested = true;
        }

        self.ensure_track_data(shared);

        let Some(track) = &self.track_data else {
            ui.centered_and_justified(|ui| {
                ui.label("No GPS data found (requires GPS Latitude and GPS Longitude channels)");
            });
            return;
        };

        if track.x.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("GPS track data is empty");
            });
            return;
        }

        self.show_toolbar(ui, shared);
        ui.separator();

        self.ensure_color_map(shared);

        let track = Arc::clone(self.track_data.as_ref().unwrap());
        let cursor_time = shared.cursor_time;
        let sectors = &shared.sectors;

        let mut hover_idx: Option<usize> = None;
        let mut clicked_idx: Option<usize> = None;
        let editing = self.editing_sectors;

        let plot = Plot::new(format!("trackmap_{}", self.id))
            .data_aspect(1.0)
            .allow_drag(true)
            .allow_zoom(true)
            .allow_scroll(false)
            .show_axes(false)
            .show_grid(false);

        let colors_ref = self.cached_colors.clone();

        let response = plot.show(ui, |plot_ui| {
            Self::draw_track_line(plot_ui, &track, colors_ref.as_deref());

            Self::draw_sector_markers(plot_ui, &track, sectors);

            if let Some(t) = cursor_time {
                Self::draw_cursor_marker(plot_ui, &track, t);
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                let idx = find_nearest_sample(&track, coord.x, coord.y);
                hover_idx = Some(idx);

                if plot_ui.response().clicked() {
                    // Reuse the already-computed index instead of calling find_nearest_sample again
                    clicked_idx = Some(idx);
                }
            }
        });

        if response.response.hovered() {
            if let Some(idx) = hover_idx {
                shared.cursor_time = Some(track.time[idx]);
            }
        }

        if editing {
            if let Some(idx) = clicked_idx {
                if let Some(start) = self.pending_sector_start.take() {
                    let sector_num = shared.sectors.len() + 1;
                    shared.sectors.push(Sector {
                        name: format!("S{}", sector_num),
                        start_index: start,
                        end_index: idx,
                    });
                    self.cached_sector_times = None;
                } else {
                    self.pending_sector_start = Some(idx);
                }
            }
        }

        ui.separator();

        self.show_sector_report(ui, shared);
    }

    fn ensure_track_data(&mut self, shared: &SharedState) {
        let ld_ptr = shared
            .ld_file
            .as_ref()
            .map(|ld| Arc::as_ptr(ld) as usize)
            .unwrap_or(0);

        let fingerprint = (ld_ptr, self.color_channel_idx);
        let track_stale = self.track_data.is_none()
            || self
                .cache_fingerprint
                .map(|(p, _)| p != ld_ptr)
                .unwrap_or(true);

        if track_stale {
            if let Some(ld) = &shared.ld_file {
                self.track_data = extract_gps_track(ld).map(Arc::new);
            } else {
                self.track_data = None;
            }
            self.cached_colors = None;
            self.cached_color_range = None;
            self.cached_sector_times = None;
        }

        if self.cache_fingerprint != Some(fingerprint) {
            self.cached_colors = None;
            self.cached_color_range = None;
        }

        self.cache_fingerprint = Some(fingerprint);
    }

    fn ensure_color_map(&mut self, shared: &SharedState) {
        if self.cached_colors.is_some() {
            return;
        }

        let Some(track) = &self.track_data else {
            return;
        };
        let Some(ch_idx) = self.color_channel_idx else {
            return;
        };
        let Some(ld) = &shared.ld_file else { return };
        let Some(ch) = ld.channels.get(ch_idx) else {
            return;
        };
        let Some(data) = ld.read_channel_data(ch) else {
            return;
        };

        self.color_channel_name = ch.name.clone();
        let (colors, vmin, vmax) = compute_color_map(track, &data, ch.freq);
        self.cached_color_range = Some((vmin, vmax));
        self.cached_colors = Some(Arc::new(colors));
    }

    fn draw_track_line(
        plot_ui: &mut egui_plot::PlotUi,
        track: &TrackData,
        colors: Option<&Vec<[u8; 4]>>,
    ) {
        if let Some(colors) = colors {
            for i in 0..track.x.len().saturating_sub(1) {
                let c = colors[i];
                let segment = Line::new(
                    "",
                    PlotPoints::new(vec![
                        [track.x[i], track.y[i]],
                        [track.x[i + 1], track.y[i + 1]],
                    ]),
                )
                .width(3.0)
                .color(egui::Color32::from_rgb(c[0], c[1], c[2]));
                plot_ui.line(segment);
            }
        } else {
            let points: Vec<[f64; 2]> = track
                .x
                .iter()
                .zip(track.y.iter())
                .map(|(&x, &y)| [x, y])
                .collect();
            let line = Line::new("Track", PlotPoints::new(points))
                .width(2.5)
                .color(egui::Color32::from_rgb(50, 255, 200));
            plot_ui.line(line);
        }
    }

    fn draw_cursor_marker(plot_ui: &mut egui_plot::PlotUi, track: &TrackData, time: f64) {
        let sample_idx = (time * track.freq as f64).round() as usize;
        let sample_idx = sample_idx.min(track.x.len().saturating_sub(1));

        if sample_idx < track.x.len() {
            let marker = Points::new(
                "cursor",
                PlotPoints::new(vec![[track.x[sample_idx], track.y[sample_idx]]]),
            )
            .shape(MarkerShape::Circle)
            .radius(6.0)
            .color(egui::Color32::from_rgb(255, 255, 0))
            .filled(true);
            plot_ui.points(marker);
        }
    }

    fn draw_sector_markers(
        plot_ui: &mut egui_plot::PlotUi,
        track: &TrackData,
        sectors: &[Sector],
    ) {
        for (i, sector) in sectors.iter().enumerate() {
            let color = CHANNEL_COLORS[i % CHANNEL_COLORS.len()];

            if sector.start_index < track.x.len() {
                let pt = vec![[track.x[sector.start_index], track.y[sector.start_index]]];
                let marker =
                    Points::new(format!("{} start", sector.name), PlotPoints::new(pt))
                        .shape(MarkerShape::Diamond)
                        .radius(8.0)
                        .color(color)
                        .filled(true);
                plot_ui.points(marker);
            }
        }
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        ui.horizontal(|ui| {
            ui.label("Color by:");
            let current_name = if self.color_channel_idx.is_some() {
                &self.color_channel_name
            } else {
                "None"
            };
            egui::ComboBox::from_id_salt(format!("color_ch_{}", self.id))
                .selected_text(current_name)
                .width(160.0)
                .show_ui(ui, |ui| {
                    ui.text_edit_singleline(&mut self.color_filter)
                        .request_focus();
                    ui.separator();

                    let filter = self.color_filter.to_lowercase();
                    if ui
                        .selectable_value(&mut self.color_channel_idx, None, "None")
                        .clicked()
                    {
                        self.invalidate_color_cache();
                        self.color_filter.clear();
                    }
                    if let Some(ld) = &shared.ld_file {
                        for (i, ch) in ld.channels.iter().enumerate() {
                            if !filter.is_empty()
                                && !ch.name.to_lowercase().contains(&filter)
                                && !ch.unit.to_lowercase().contains(&filter)
                            {
                                continue;
                            }
                            let label = if ch.unit.is_empty() {
                                ch.name.clone()
                            } else {
                                format!("{} ({})", ch.name, ch.unit)
                            };
                            if ui
                                .selectable_value(&mut self.color_channel_idx, Some(i), label)
                                .clicked()
                            {
                                self.invalidate_color_cache();
                                self.color_filter.clear();
                            }
                        }
                    }
                });

            if let Some((vmin, vmax)) = self.cached_color_range {
                ui.separator();
                Self::draw_color_legend(ui, vmin, vmax);
            }

            ui.separator();

            let edit_label = if self.editing_sectors {
                if self.pending_sector_start.is_some() {
                    "Click end point..."
                } else {
                    "Click start point..."
                }
            } else {
                "Edit Sectors"
            };
            if ui
                .selectable_label(self.editing_sectors, edit_label)
                .clicked()
            {
                self.editing_sectors = !self.editing_sectors;
                self.pending_sector_start = None;
            }

            if !shared.sectors.is_empty() && ui.small_button("Clear Sectors").clicked() {
                shared.sectors.clear();
                self.cached_sector_times = None;
            }

            ui.separator();

            ui.label("Ref lap:");
            let ref_label = shared
                .reference_lap
                .map(|i| shared.laps.get(i).map(|l| l.name.clone()).unwrap_or_default())
                .unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt(format!("ref_lap_{}", self.id))
                .selected_text(ref_label)
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut shared.reference_lap, None, "None");
                    for (i, lap) in shared.laps.iter().enumerate() {
                        let dur = lap.end_time - lap.start_time;
                        let label = format!("{} ({})", lap.name, i3rs_core::format_duration(dur));
                        ui.selectable_value(&mut shared.reference_lap, Some(i), label);
                    }
                });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.is_popped_out {
                    if ui
                        .small_button("\u{2B73} Dock")
                        .on_hover_text("Return to main window")
                        .clicked()
                    {
                        self.dock_requested = true;
                    }
                } else if ui
                    .small_button("\u{2B71} Pop Out")
                    .on_hover_text("Open in separate window")
                    .clicked()
                {
                    self.pop_out_requested = true;
                }
            });
        });
    }

    fn invalidate_color_cache(&mut self) {
        self.cached_colors = None;
        self.cached_color_range = None;
        self.cache_fingerprint = None;
    }

    fn draw_color_legend(ui: &mut egui::Ui, vmin: f64, vmax: f64) {
        let gradient_h = 12.0;
        let label_h = 11.0;
        let gap = 1.0;
        let total_h = gradient_h + gap + label_h;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, total_h), egui::Sense::hover());
        let painter = ui.painter();

        let grad_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), gradient_h));
        let n_steps = 60;
        let step_width = grad_rect.width() / n_steps as f32;
        for i in 0..n_steps {
            let t = i as f32 / (n_steps - 1) as f32;
            let hue = (1.0 - t) * 240.0;
            let (r, g, b) = i3rs_core::track::hsv_to_rgb(hue, 1.0, 1.0);
            let color = egui::Color32::from_rgb(r, g, b);
            let x = grad_rect.left() + i as f32 * step_width;
            let step_rect = egui::Rect::from_min_size(
                egui::pos2(x, grad_rect.top()),
                egui::vec2(step_width + 0.5, gradient_h),
            );
            painter.rect_filled(step_rect, 0.0, color);
        }

        let font = egui::FontId::proportional(9.0);
        let label_y = grad_rect.bottom() + gap;
        painter.text(
            egui::pos2(rect.left(), label_y),
            egui::Align2::LEFT_TOP,
            format!("{:.1}", vmin),
            font.clone(),
            egui::Color32::from_gray(200),
        );
        painter.text(
            egui::pos2(rect.right(), label_y),
            egui::Align2::RIGHT_TOP,
            format!("{:.1}", vmax),
            font,
            egui::Color32::from_gray(200),
        );
    }

    fn show_sector_report(&mut self, ui: &mut egui::Ui, shared: &SharedState) {
        if shared.sectors.is_empty() || shared.laps.is_empty() {
            return;
        }

        let Some(track) = &self.track_data else {
            return;
        };

        // Cache sector times — only recompute when sectors or laps change
        let needs_rebuild = self
            .cached_sector_times
            .as_ref()
            .is_none_or(|c| c.sector_count != shared.sectors.len() || c.lap_count != shared.laps.len());

        if needs_rebuild {
            let times = compute_sector_times(&shared.sectors, &shared.laps, track);
            self.cached_sector_times = Some(CachedSectorReport {
                sector_count: shared.sectors.len(),
                lap_count: shared.laps.len(),
                times,
            });
        }

        let sector_times = &self.cached_sector_times.as_ref().unwrap().times;
        if sector_times.is_empty() {
            return;
        }

        ui.strong("Sector Times");

        let ref_lap_times: Option<&Vec<_>> =
            shared.reference_lap.and_then(|i| sector_times.get(i));

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                egui::Grid::new(format!("sector_report_{}", self.id))
                    .striped(true)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        ui.strong("Lap");
                        for sector in &shared.sectors {
                            ui.strong(&sector.name);
                        }
                        ui.strong("Total");
                        ui.end_row();

                        for (lap_idx, lap_sectors) in sector_times.iter().enumerate() {
                            let lap = &shared.laps[lap_idx];
                            ui.label(&lap.name);

                            let mut total = 0.0;
                            for (s_idx, st) in lap_sectors.iter().enumerate() {
                                total += st.time_secs;

                                if let Some(ref_times) = ref_lap_times
                                    && Some(lap_idx) != shared.reference_lap
                                    && let Some(ref_st) = ref_times.get(s_idx)
                                {
                                    let delta = st.time_secs - ref_st.time_secs;
                                    ui.colored_label(
                                        delta_color(delta),
                                        format!("{} ({:+.3})", i3rs_core::format_duration(st.time_secs), delta),
                                    );
                                    continue;
                                }
                                ui.label(i3rs_core::format_duration(st.time_secs));
                            }

                            if let Some(ref_times) = ref_lap_times
                                && Some(lap_idx) != shared.reference_lap
                            {
                                let ref_total: f64 =
                                    ref_times.iter().map(|st| st.time_secs).sum();
                                let delta = total - ref_total;
                                ui.colored_label(
                                    delta_color(delta),
                                    format!("{} ({:+.3})", i3rs_core::format_duration(total), delta),
                                );
                            } else {
                                ui.label(i3rs_core::format_duration(total));
                            }
                            ui.end_row();
                        }
                    });
            });
    }
}

fn delta_color(delta: f64) -> egui::Color32 {
    if delta < -0.01 {
        egui::Color32::from_rgb(100, 255, 100) // green = faster
    } else if delta > 0.01 {
        egui::Color32::from_rgb(255, 100, 100) // red = slower
    } else {
        egui::Color32::from_gray(200)
    }
}

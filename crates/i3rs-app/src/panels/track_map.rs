//! Track map panel: GPS track visualization with rainbow coloring and sector editing.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Line, MarkerShape, Plot, PlotPoint, PlotPoints, Points};
use i3rs_core::{
    Sector, SectorTime, TrackData, compute_color_map, compute_sector_times, find_nearest_sample,
};

use crate::state::{CHANNEL_COLORS, SharedState};

pub struct TrackMapPanel {
    pub id: u64,
    pub title: String,
    track_data: Option<Arc<TrackData>>,
    /// Channel index for rainbow coloring (None = solid color).
    pub color_channel_idx: Option<usize>,
    /// Cached per-sample RGBA colors.
    cached_colors: Option<Arc<[[u8; 4]]>>,
    cached_color_range: Option<(f64, f64)>,
    color_channel_name: String,
    editing_sectors: bool,
    pending_sector_start: Option<usize>,
    /// Cached sector time report (invalidated when sectors/laps change).
    cached_sector_times: Option<CachedSectorReport>,
    /// Fingerprint for track/color cache invalidation.
    cache_fingerprint: Option<(usize, Option<usize>)>,
    track_line_cache: Option<CachedTrackLine>,
    cursor_marker_cache: Option<CachedTrackMarker>,
    sector_marker_cache: Option<CachedSectorMarkers>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
struct TrackLineFingerprint {
    track_ptr: usize,
    track_len: usize,
    colors_ptr: usize,
    colors_len: usize,
}

struct CachedTrackLine {
    fingerprint: TrackLineFingerprint,
    solid_points: Vec<PlotPoint>,
    colored_segments: Vec<CachedColoredTrackSegment>,
}

struct CachedColoredTrackSegment {
    color: egui::Color32,
    points: [PlotPoint; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TrackMarkerFingerprint {
    track_ptr: usize,
    track_len: usize,
    sample_idx: usize,
}

struct CachedTrackMarker {
    fingerprint: TrackMarkerFingerprint,
    points: [PlotPoint; 1],
}

#[derive(Clone, PartialEq, Eq)]
struct SectorMarkersFingerprint {
    track_ptr: usize,
    track_len: usize,
    start_indices: Vec<usize>,
}

struct CachedSectorMarkers {
    fingerprint: SectorMarkersFingerprint,
    markers: Vec<CachedSectorMarker>,
}

struct CachedSectorMarker {
    name: String,
    color: egui::Color32,
    points: [PlotPoint; 1],
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
            track_line_cache: None,
            cursor_marker_cache: None,
            sector_marker_cache: None,
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
        self.clear_render_caches();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        let _perf = crate::perf_metrics::scope("track-map draw");

        // If popped out, handle OS window close → dock back
        if self.is_popped_out && ui.input(|i| i.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dock_requested = true;
        }

        self.ensure_track_data(shared);

        let Some(track) = &self.track_data else {
            ui.centered_and_justified(|ui| {
                if shared.is_track_data_build_pending() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Building GPS track...");
                    });
                } else {
                    ui.label(
                        "No GPS data found (requires GPS Latitude and GPS Longitude channels)",
                    );
                }
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

        let colors = self.cached_colors.clone();
        let response = plot.show(ui, |plot_ui| {
            Self::draw_track_line(
                plot_ui,
                &mut self.track_line_cache,
                &track,
                colors.as_deref(),
            );

            Self::draw_sector_markers(plot_ui, &mut self.sector_marker_cache, &track, sectors);

            if let Some(t) = cursor_time {
                Self::draw_cursor_marker(plot_ui, &mut self.cursor_marker_cache, &track, t);
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                let _hover_perf = crate::perf_metrics::scope("track hover lookup");
                let idx = find_nearest_sample(&track, coord.x, coord.y);
                hover_idx = Some(idx);

                if plot_ui.response().clicked() {
                    // Reuse the already-computed index instead of calling find_nearest_sample again
                    clicked_idx = Some(idx);
                }
            }
        });

        if response.response.hovered()
            && let Some(idx) = hover_idx
        {
            shared.cursor_time = Some(track.time[idx]);
        }

        if editing && let Some(idx) = clicked_idx {
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
            if shared.ld_file.is_some() {
                if let Some(track_data) = shared.track_data_if_ready() {
                    self.track_data = Some(track_data);
                } else {
                    shared.request_track_data_build();
                    self.track_data = None;
                }
            } else {
                self.track_data = None;
            }
            self.cached_colors = None;
            self.cached_color_range = None;
            self.cached_sector_times = None;
            self.clear_render_caches();
        } else if self.track_data.is_none() {
            if let Some(track_data) = shared.track_data_if_ready() {
                self.track_data = Some(track_data);
            } else if shared.ld_file.is_some() {
                shared.request_track_data_build();
            }
        }

        if self.cache_fingerprint != Some(fingerprint) {
            self.cached_colors = None;
            self.cached_color_range = None;
            self.track_line_cache = None;
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
        let Some(decoded) = shared.decoded_physical_channel_if_ready(ch_idx) else {
            shared.request_physical_channel_decode(ch_idx);
            return;
        };

        self.color_channel_name = ch.name.clone();
        let (colors, vmin, vmax) = compute_color_map(track, &decoded.data, ch.freq);
        self.cached_color_range = Some((vmin, vmax));
        self.cached_colors = Some(Arc::from(colors));
        self.track_line_cache = None;
    }

    fn draw_track_line<'a>(
        plot_ui: &mut egui_plot::PlotUi<'a>,
        cache: &'a mut Option<CachedTrackLine>,
        track: &TrackData,
        colors: Option<&[[u8; 4]]>,
    ) {
        let fingerprint = TrackLineFingerprint {
            track_ptr: track as *const TrackData as usize,
            track_len: track.x.len(),
            colors_ptr: colors.map_or(0, |colors| colors.as_ptr() as usize),
            colors_len: colors.map_or(0, |colors| colors.len()),
        };

        let cached =
            cache.get_or_insert_with(|| CachedTrackLine::build(track, colors, fingerprint));
        if cached.fingerprint != fingerprint {
            *cached = CachedTrackLine::build(track, colors, fingerprint);
        }

        if cached.colored_segments.is_empty() {
            if !cached.solid_points.is_empty() {
                let line = Line::new(
                    "Track",
                    PlotPoints::Borrowed(cached.solid_points.as_slice()),
                )
                .width(2.5)
                .color(egui::Color32::from_rgb(50, 255, 200));
                plot_ui.line(line);
            }
        } else {
            for segment in &cached.colored_segments {
                let line = Line::new("", PlotPoints::Borrowed(segment.points.as_slice()))
                    .width(3.0)
                    .color(segment.color);
                plot_ui.line(line);
            }
        }
    }

    fn draw_cursor_marker<'a>(
        plot_ui: &mut egui_plot::PlotUi<'a>,
        cache: &'a mut Option<CachedTrackMarker>,
        track: &TrackData,
        time: f64,
    ) {
        let sample_idx = (time * track.freq as f64).round() as usize;
        let sample_idx = sample_idx.min(track.x.len().saturating_sub(1));

        if sample_idx >= track.x.len() {
            return;
        }

        let fingerprint = TrackMarkerFingerprint {
            track_ptr: track as *const TrackData as usize,
            track_len: track.x.len(),
            sample_idx,
        };
        let cached =
            cache.get_or_insert_with(|| CachedTrackMarker::build(track, sample_idx, fingerprint));
        if cached.fingerprint != fingerprint {
            *cached = CachedTrackMarker::build(track, sample_idx, fingerprint);
        }

        let marker = Points::new("cursor", PlotPoints::Borrowed(cached.points.as_slice()))
            .shape(MarkerShape::Circle)
            .radius(6.0)
            .color(egui::Color32::from_rgb(255, 255, 0))
            .filled(true);
        plot_ui.points(marker);
    }

    fn draw_sector_markers<'a>(
        plot_ui: &mut egui_plot::PlotUi<'a>,
        cache: &'a mut Option<CachedSectorMarkers>,
        track: &TrackData,
        sectors: &[Sector],
    ) {
        let fingerprint = SectorMarkersFingerprint {
            track_ptr: track as *const TrackData as usize,
            track_len: track.x.len(),
            start_indices: sectors.iter().map(|sector| sector.start_index).collect(),
        };
        let cached = cache
            .get_or_insert_with(|| CachedSectorMarkers::build(track, sectors, fingerprint.clone()));
        if cached.fingerprint != fingerprint {
            *cached = CachedSectorMarkers::build(track, sectors, fingerprint);
        }

        for marker in &cached.markers {
            let points = Points::new(&marker.name, PlotPoints::Borrowed(marker.points.as_slice()))
                .shape(MarkerShape::Diamond)
                .radius(8.0)
                .color(marker.color)
                .filled(true);
            plot_ui.points(points);
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
                .map(|i| {
                    shared
                        .laps
                        .get(i)
                        .map(|l| l.name.clone())
                        .unwrap_or_default()
                })
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
        self.track_line_cache = None;
    }

    fn clear_render_caches(&mut self) {
        self.track_line_cache = None;
        self.cursor_marker_cache = None;
        self.sector_marker_cache = None;
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
        let needs_rebuild = self.cached_sector_times.as_ref().is_none_or(|c| {
            c.sector_count != shared.sectors.len() || c.lap_count != shared.laps.len()
        });

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

        let ref_lap_times: Option<&Vec<_>> = shared.reference_lap.and_then(|i| sector_times.get(i));

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
                                        format!(
                                            "{} ({:+.3})",
                                            i3rs_core::format_duration(st.time_secs),
                                            delta
                                        ),
                                    );
                                    continue;
                                }
                                ui.label(i3rs_core::format_duration(st.time_secs));
                            }

                            if let Some(ref_times) = ref_lap_times
                                && Some(lap_idx) != shared.reference_lap
                            {
                                let ref_total: f64 = ref_times.iter().map(|st| st.time_secs).sum();
                                let delta = total - ref_total;
                                ui.colored_label(
                                    delta_color(delta),
                                    format!(
                                        "{} ({:+.3})",
                                        i3rs_core::format_duration(total),
                                        delta
                                    ),
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

impl CachedTrackLine {
    fn build(
        track: &TrackData,
        colors: Option<&[[u8; 4]]>,
        fingerprint: TrackLineFingerprint,
    ) -> Self {
        if let Some(colors) = colors {
            let colored_segments = colors
                .iter()
                .enumerate()
                .take(track.x.len().saturating_sub(1))
                .map(|(idx, color)| CachedColoredTrackSegment {
                    color: egui::Color32::from_rgb(color[0], color[1], color[2]),
                    points: [
                        PlotPoint::new(track.x[idx], track.y[idx]),
                        PlotPoint::new(track.x[idx + 1], track.y[idx + 1]),
                    ],
                })
                .collect();

            Self {
                fingerprint,
                solid_points: Vec::new(),
                colored_segments,
            }
        } else {
            let solid_points = track
                .x
                .iter()
                .zip(track.y.iter())
                .map(|(&x, &y)| PlotPoint::new(x, y))
                .collect();

            Self {
                fingerprint,
                solid_points,
                colored_segments: Vec::new(),
            }
        }
    }
}

impl CachedTrackMarker {
    fn build(track: &TrackData, sample_idx: usize, fingerprint: TrackMarkerFingerprint) -> Self {
        Self {
            fingerprint,
            points: [PlotPoint::new(track.x[sample_idx], track.y[sample_idx])],
        }
    }
}

impl CachedSectorMarkers {
    fn build(track: &TrackData, sectors: &[Sector], fingerprint: SectorMarkersFingerprint) -> Self {
        let markers = sectors
            .iter()
            .enumerate()
            .filter_map(|(idx, sector)| {
                (sector.start_index < track.x.len()).then(|| CachedSectorMarker {
                    name: format!("{} start", sector.name),
                    color: CHANNEL_COLORS[idx % CHANNEL_COLORS.len()],
                    points: [PlotPoint::new(
                        track.x[sector.start_index],
                        track.y[sector.start_index],
                    )],
                })
            })
            .collect();

        Self {
            fingerprint,
            markers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_panel() -> TrackMapPanel {
        TrackMapPanel::new(1, "Track")
    }

    #[test]
    fn clear_cache_clears_track_render_caches() {
        let mut panel = sample_panel();
        panel.track_line_cache = Some(CachedTrackLine {
            fingerprint: TrackLineFingerprint {
                track_ptr: 1,
                track_len: 2,
                colors_ptr: 3,
                colors_len: 4,
            },
            solid_points: vec![PlotPoint::new(0.0, 0.0)],
            colored_segments: Vec::new(),
        });
        panel.cursor_marker_cache = Some(CachedTrackMarker {
            fingerprint: TrackMarkerFingerprint {
                track_ptr: 1,
                track_len: 2,
                sample_idx: 0,
            },
            points: [PlotPoint::new(0.0, 0.0)],
        });
        panel.sector_marker_cache = Some(CachedSectorMarkers {
            fingerprint: SectorMarkersFingerprint {
                track_ptr: 1,
                track_len: 2,
                start_indices: vec![0],
            },
            markers: Vec::new(),
        });

        panel.clear_cache();

        assert!(panel.track_line_cache.is_none());
        assert!(panel.cursor_marker_cache.is_none());
        assert!(panel.sector_marker_cache.is_none());
    }

    #[test]
    fn invalidate_color_cache_only_drops_track_line_geometry() {
        let mut panel = sample_panel();
        panel.track_line_cache = Some(CachedTrackLine {
            fingerprint: TrackLineFingerprint {
                track_ptr: 1,
                track_len: 2,
                colors_ptr: 3,
                colors_len: 4,
            },
            solid_points: vec![PlotPoint::new(0.0, 0.0)],
            colored_segments: Vec::new(),
        });
        panel.cursor_marker_cache = Some(CachedTrackMarker {
            fingerprint: TrackMarkerFingerprint {
                track_ptr: 1,
                track_len: 2,
                sample_idx: 0,
            },
            points: [PlotPoint::new(0.0, 0.0)],
        });

        panel.invalidate_color_cache();

        assert!(panel.track_line_cache.is_none());
        assert!(panel.cursor_marker_cache.is_some());
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

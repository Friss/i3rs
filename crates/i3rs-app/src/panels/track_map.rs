//! Track map panel: GPS track visualization with rainbow coloring and sector editing.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Line, MarkerShape, Plot, PlotPoint, PlotPoints, Points};
use i3rs_core::{
    Sector, SectorTime, TrackData, compute_color_map, compute_sector_times,
    find_nearest_sample_in_range,
};

use super::utils::{
    create_plotted_channel, resolve_plotted_channel_display_meta, sample_range,
    transform_channel_value,
};
use crate::state::{CHANNEL_COLORS, SharedState};

pub struct TrackMapPanel {
    pub id: u64,
    pub title: String,
    pub section_times_only: bool,
    pub show_annotations: bool,
    section_count: usize,
    trend_sector: usize,
    status: Option<String>,
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
    /// Worksheet index this track map belongs to when docked.
    pub home_worksheet: usize,
}

struct CachedSectorReport {
    sectors: Vec<Sector>,
    laps: Vec<(u64, u64)>,
    times: Vec<Vec<SectorTime>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TrackLineFingerprint {
    track_ptr: usize,
    track_len: usize,
    colors_ptr: usize,
    colors_len: usize,
    sample_range: (usize, usize),
}

struct CachedTrackLine {
    fingerprint: TrackLineFingerprint,
    solid_segments: Vec<Vec<PlotPoint>>,
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
    sectors: Vec<Sector>,
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
            section_times_only: false,
            show_annotations: true,
            section_count: 4,
            trend_sector: 0,
            status: None,
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
            home_worksheet: 0,
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
        if let Some(error) = &shared.track_definition_error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("Could not restore sections: {error}"),
            );
        }

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

        if self.section_times_only {
            self.show_sector_report(ui, shared);
            return;
        }
        self.ensure_color_map(shared);

        let track = Arc::clone(self.track_data.as_ref().unwrap());
        let cursor_time = shared.cursor_time;
        let sectors = &shared.sectors;
        let time_range = shared
            .zoom_range
            .unwrap_or((0.0, shared.data_duration.unwrap_or(f64::INFINITY)));
        let sample_range = (
            track.time.partition_point(|t| *t < time_range.0),
            track.time.partition_point(|t| *t < time_range.1),
        );

        let mut hover_idx: Option<usize> = None;
        let mut clicked_idx: Option<usize> = None;
        let editing = self.editing_sectors;

        let annotations = if self.show_annotations {
            section_annotations(&track, shared)
        } else {
            Vec::new()
        };
        let range_changed = self
            .track_line_cache
            .as_ref()
            .is_none_or(|c| c.fingerprint.sample_range != sample_range);
        let mut plot = Plot::new(format!("trackmap_{}", self.id))
            .data_aspect(1.0)
            .allow_drag(true)
            .allow_zoom(true)
            .allow_scroll(false)
            .show_axes(false)
            .show_grid(false)
            .height(
                (ui.available_height() - if sectors.is_empty() { 0.0 } else { 230.0 }).max(180.0),
            );

        if range_changed {
            plot = plot.reset();
        }
        let colors = self.cached_colors.clone();
        let response = plot.show(ui, |plot_ui| {
            Self::draw_track_line(
                plot_ui,
                &mut self.track_line_cache,
                &track,
                colors.as_deref(),
                sample_range,
            );

            Self::draw_sector_markers(plot_ui, &mut self.sector_marker_cache, &track, sectors);

            if let Some(t) = cursor_time.filter(|t| *t >= time_range.0 && *t < time_range.1) {
                Self::draw_cursor_marker(plot_ui, &mut self.cursor_marker_cache, &track, t);
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                let _hover_perf = crate::perf_metrics::scope("track hover lookup");
                let idx = find_nearest_sample_in_range(&track, coord.x, coord.y, time_range);
                hover_idx = idx;

                if plot_ui.response().clicked() {
                    // Reuse the already-computed index instead of calling find_nearest_sample again
                    clicked_idx = idx;
                }
            }
        });

        // Place labels in two screen-space columns so dense named sections
        // remain readable without changing the track's auto-fit bounds.
        let frame = *response.transform.frame();
        let painter = ui.painter().with_clip_rect(frame);
        for right in [false, true] {
            let mut labels: Vec<_> = annotations
                .iter()
                .filter_map(|(point, label)| {
                    let pos = response.transform.position_from_point(point);
                    (frame.contains(pos) && (pos.x >= frame.center().x) == right)
                        .then_some((pos, label))
                })
                .collect();
            labels.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
            let gap = 16.0_f32.min((frame.height() - 20.).max(0.) / labels.len().max(1) as f32);
            let mut ys: Vec<f32> = Vec::new();
            for (pos, _) in &labels {
                ys.push(pos.y.max(ys.last().map_or(frame.top() + 10., |y| y + gap)));
            }
            if let Some(last) = ys.last_mut() {
                *last = last.min(frame.bottom() - 10.);
            }
            for i in (0..ys.len().saturating_sub(1)).rev() {
                ys[i] = ys[i].min(ys[i + 1] - gap);
            }
            for ((point, label), y) in labels.iter().zip(ys) {
                let anchor = egui::pos2(
                    if right {
                        frame.right() - 8.
                    } else {
                        frame.left() + 8.
                    },
                    y,
                );
                let text = label
                    .replace("\nCorr Speed ", " · ")
                    .replace("\nGear ", " · G");
                let galley = painter.layout_no_wrap(
                    text,
                    egui::FontId::proportional(9.),
                    egui::Color32::LIGHT_GRAY,
                );
                let text_pos = egui::pos2(
                    if right {
                        anchor.x - galley.size().x
                    } else {
                        anchor.x
                    },
                    y - galley.size().y / 2.,
                );
                let edge = egui::pos2(
                    if right {
                        text_pos.x - 4.
                    } else {
                        text_pos.x + galley.size().x + 4.
                    },
                    y,
                );
                painter.line_segment(
                    [*point, edge],
                    egui::Stroke::new(0.5, egui::Color32::from_gray(65)),
                );
                painter.galley(text_pos, galley, egui::Color32::LIGHT_GRAY);
            }
        }

        if response.response.hovered()
            && let Some(idx) = hover_idx
        {
            shared.set_hover_cursor(track.time[idx]);
        }

        if editing && let Some(idx) = clicked_idx {
            if let Some(start) = self.pending_sector_start.take() {
                let sector_num = shared.sectors.len() + 1;
                shared.sectors.push(Sector {
                    distance: None,
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
        sample_range: (usize, usize),
    ) {
        let fingerprint = TrackLineFingerprint {
            track_ptr: track as *const TrackData as usize,
            track_len: track.x.len(),
            colors_ptr: colors.map_or(0, |colors| colors.as_ptr() as usize),
            colors_len: colors.map_or(0, |colors| colors.len()),
            sample_range,
        };

        let cached =
            cache.get_or_insert_with(|| CachedTrackLine::build(track, colors, fingerprint));
        if cached.fingerprint != fingerprint {
            *cached = CachedTrackLine::build(track, colors, fingerprint);
        }

        if cached.colored_segments.is_empty() {
            for segment in &cached.solid_segments {
                let line = Line::new("Track", PlotPoints::Borrowed(segment.as_slice()))
                    .width(2.5_f32)
                    .color(egui::Color32::from_rgb(50, 255, 200));
                plot_ui.line(line);
            }
        } else {
            for segment in &cached.colored_segments {
                let line = Line::new("", PlotPoints::Borrowed(segment.points.as_slice()))
                    .width(3.0_f32)
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

        if sample_idx >= track.x.len()
            || !track.x[sample_idx].is_finite()
            || !track.y[sample_idx].is_finite()
        {
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
            .radius(6.0_f32)
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
            sectors: sectors.to_vec(),
        };
        let cached = cache
            .get_or_insert_with(|| CachedSectorMarkers::build(track, sectors, fingerprint.clone()));
        if cached.fingerprint != fingerprint {
            *cached = CachedSectorMarkers::build(track, sectors, fingerprint);
        }

        for marker in &cached.markers {
            let points = Points::new(&marker.name, PlotPoints::Borrowed(marker.points.as_slice()))
                .shape(MarkerShape::Diamond)
                .radius(8.0_f32)
                .color(marker.color)
                .filled(true);
            plot_ui.points(points);
        }
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        ui.horizontal_wrapped(|ui| {
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
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.show_annotations, "Section labels and speed / gear");
            ui.add(egui::DragValue::new(&mut self.section_count).range(2..=20).prefix("Sections "));
            if ui.button("Split selected lap").on_hover_text("Create generic sections of equal distance; rename them below").clicked()
                && let Some(track) = &self.track_data {
                let lap = shared.selected_lap.and_then(|i| shared.laps.get(i));
                if let Some(lap) = lap {
                    shared.sectors = i3rs_core::split_lap_into_sectors(track,lap,self.section_count);
                    self.status = Some(if shared.sectors.is_empty() { "Not enough valid GPS movement in this lap" } else { "Sections created; rename them below or edit their boundaries on the map" }.into());
                } else { self.status = Some("Select a complete lap before splitting sections".into()); }
            }
            if ui.button("VIR Full sections").on_hover_text("Apply the 33 named distance sections from the VIR Full model to the selected complete lap").clicked() {
                let result = self.track_data.as_ref().ok_or("GPS track is unavailable".to_string()).and_then(|track| {
                    let lap = shared.selected_lap.and_then(|i|shared.laps.get(i)).ok_or("Select a complete VIR lap first".to_string())?;
                    i3rs_core::track::sectors_from_distance_definition(track,lap,&i3rs_core::track::vir_full_definition())
                });
                match result { Ok(sectors) => {shared.sectors=sectors; self.status=Some("VIR Full: 33 distance sections; laps within 5% stretch to 5337.5 m".into());}, Err(e)=>self.status=Some(e) }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if ui.button("Import sections…").clicked()
                    && let Some(path) = rfd::FileDialog::new().add_filter("Track sections", &["json"]).pick_file() {
                    let result = std::fs::read_to_string(path).map_err(|e| e.to_string())
                        .and_then(|json| serde_json::from_str::<i3rs_core::TrackDefinition>(&json).map_err(|e| e.to_string()))
                        .and_then(|definition| {
                            let track = self.track_data.as_ref().ok_or("GPS track is unavailable")?;
                            i3rs_core::import_track_definition(track, &definition)
                        });
                    match result {
                        Ok(sectors) => { shared.sectors = sectors; self.status = Some("Sections imported".into()); }
                        Err(error) => self.status = Some(error),
                    }
                }
                if ui.add_enabled(!shared.sectors.is_empty(),egui::Button::new("Export sections…")).clicked()
                    && let Some(track) = &self.track_data
                    && let Some(definition) = i3rs_core::export_track_definition(track,&shared.sectors,shared.file_name.clone())
                    && let Some(path) = rfd::FileDialog::new().add_filter("Track sections", &["json"]).set_file_name("track-sections.json").save_file() {
                    self.status = Some(match serde_json::to_string_pretty(&definition).map_err(|e| e.to_string())
                        .and_then(|json| std::fs::write(path,json).map_err(|e|e.to_string())) {
                        Ok(()) => "Sections exported".into(), Err(error) => error,
                    });
                }
            }
        });
        if let Some(status) = &self.status {
            ui.label(status);
        }
        if !shared.sectors.is_empty() {
            ui.collapsing("Section names", |ui| {
                ui.label("Distance-defined sections retain their model boundaries when renamed.");
                let mut remove = None;
                for (i, sector) in shared.sectors.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}.", i + 1));
                        ui.text_edit_singleline(&mut sector.name);
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    shared.sectors.remove(i);
                }
            });
        }
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

    fn show_sector_report(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        if shared.sectors.is_empty() || shared.laps.is_empty() {
            ui.label("Select a lap, then split it into sections or import named sections. Boundaries can also be picked on Track Report.");
            return;
        }
        let Some(track) = &self.track_data else {
            return;
        };
        let laps: Vec<_> = shared
            .laps
            .iter()
            .map(|l| (l.start_time.to_bits(), l.end_time.to_bits()))
            .collect();
        if self
            .cached_sector_times
            .as_ref()
            .is_none_or(|c| c.sectors != shared.sectors || c.laps != laps)
        {
            self.cached_sector_times = Some(CachedSectorReport {
                sectors: shared.sectors.clone(),
                laps,
                times: compute_sector_times(&shared.sectors, &shared.laps, track),
            });
        }
        let times = &self.cached_sector_times.as_ref().unwrap().times;
        let best: Vec<f64> = (0..shared.sectors.len())
            .map(|s| {
                times
                    .iter()
                    .filter_map(|row| row.get(s))
                    .map(|t| t.time_secs)
                    .filter(|t| t.is_finite() && *t > 0.0)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        ui.strong("Section Times");
        let max_height = if self.section_times_only {
            ui.available_height() * 0.6
        } else {
            190.0
        };
        egui::ScrollArea::both()
            .id_salt(("sections", self.id))
            .max_height(max_height)
            .show(ui, |ui| {
                egui::Grid::new(("section_matrix", self.id))
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Lap");
                        for sector in &shared.sectors {
                            ui.strong(&sector.name);
                        }
                        ui.strong("Total");
                        ui.end_row();
                        for (li, row) in times.iter().enumerate() {
                            let lap = &shared.laps[li];
                            if ui
                                .selectable_label(shared.selected_lap == Some(li), &lap.name)
                                .clicked()
                            {
                                shared.selected_lap = Some(li);
                                shared.zoom_range = Some((lap.start_time, lap.end_time));
                                if shared
                                    .cursor_time
                                    .is_none_or(|t| t < lap.start_time || t >= lap.end_time)
                                {
                                    shared.cursor_time = Some(lap.start_time);
                                }
                                shared.zoom_from_timeline = true;
                            }
                            for (si, st) in row.iter().enumerate() {
                                if !st.time_secs.is_finite() {
                                    ui.weak("—");
                                    continue;
                                }
                                let delta = shared
                                    .reference_lap
                                    .and_then(|i| times.get(i))
                                    .and_then(|r| r.get(si))
                                    .map(|r| st.time_secs - r.time_secs)
                                    .filter(|d| d.is_finite());
                                let mut label = format!("{:.3}", st.time_secs);
                                if let Some(d) = delta {
                                    label.push_str(&format!(" ({d:+.3})"));
                                }
                                let is_best = (st.time_secs - best[si]).abs() < 1e-7;
                                let color = if is_best {
                                    egui::Color32::LIGHT_GREEN
                                } else {
                                    delta.map_or(ui.visuals().text_color(), delta_color)
                                };
                                ui.colored_label(
                                    color,
                                    if is_best {
                                        format!("★ {label}")
                                    } else {
                                        label
                                    },
                                );
                            }
                            let sum: f64 = row.iter().map(|s| s.time_secs).sum();
                            ui.label(if sum.is_finite() {
                                format!("{sum:.3}")
                            } else {
                                "—".into()
                            });
                            ui.end_row();
                        }
                    });
            });
        if self.section_times_only {
            self.trend_sector = self.trend_sector.min(shared.sectors.len() - 1);
            egui::ComboBox::from_id_salt(("section_trend", self.id))
                .selected_text(&shared.sectors[self.trend_sector].name)
                .show_ui(ui, |ui| {
                    for (i, s) in shared.sectors.iter().enumerate() {
                        ui.selectable_value(&mut self.trend_sector, i, &s.name);
                    }
                });
            let mut runs = Vec::new();
            let mut run = Vec::new();
            for (i, row) in times.iter().enumerate() {
                let t = row[self.trend_sector].time_secs;
                if t.is_finite() {
                    run.push([i as f64, t]);
                } else if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
            if !run.is_empty() {
                runs.push(run);
            }
            Plot::new(("section_trend_plot", self.id))
                .height(ui.available_height().max(100.0))
                .x_axis_label("Lap index (0 = Out Lap)")
                .y_axis_label("Section time (s)")
                .show(ui, |p| {
                    for run in runs {
                        p.line(Line::new("Section time", run.clone()));
                        p.points(Points::new("", run).radius(3.0));
                    }
                });
        }
    }
}

impl CachedTrackLine {
    fn build(
        track: &TrackData,
        colors: Option<&[[u8; 4]]>,
        fingerprint: TrackLineFingerprint,
    ) -> Self {
        let (start, end) = fingerprint.sample_range;
        let mut solid_segments = Vec::new();
        let mut run = Vec::new();
        let mut colored_segments = Vec::new();
        for idx in start..end.min(track.x.len()) {
            let valid = track.x[idx].is_finite() && track.y[idx].is_finite();
            if !valid {
                if !run.is_empty() {
                    solid_segments.push(std::mem::take(&mut run));
                }
                continue;
            }
            if let Some(colors) = colors {
                if idx + 1 < end.min(track.x.len())
                    && track.x[idx + 1].is_finite()
                    && track.y[idx + 1].is_finite()
                    && let Some(color) = colors.get(idx)
                {
                    colored_segments.push(CachedColoredTrackSegment {
                        color: egui::Color32::from_rgb(color[0], color[1], color[2]),
                        points: [
                            PlotPoint::new(track.x[idx], track.y[idx]),
                            PlotPoint::new(track.x[idx + 1], track.y[idx + 1]),
                        ],
                    });
                }
            } else {
                run.push(PlotPoint::new(track.x[idx], track.y[idx]));
            }
        }
        if !run.is_empty() {
            solid_segments.push(run);
        }
        Self {
            fingerprint,
            solid_segments,
            colored_segments,
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
            .filter(|(_, sector)| {
                sector.start_index < track.x.len()
                    && track.x[sector.start_index].is_finite()
                    && track.y[sector.start_index].is_finite()
            })
            .map(|(idx, sector)| CachedSectorMarker {
                name: format!("{} start", sector.name),
                color: CHANNEL_COLORS[idx % CHANNEL_COLORS.len()],
                points: [PlotPoint::new(
                    track.x[sector.start_index],
                    track.y[sector.start_index],
                )],
            })
            .collect();

        Self {
            fingerprint,
            markers,
        }
    }
}

fn section_annotations(track: &TrackData, shared: &SharedState) -> Vec<(PlotPoint, String)> {
    let Some(lap) = shared.selected_lap.and_then(|i| shared.laps.get(i)) else {
        return Vec::new();
    };
    let Some(ld) = &shared.ld_file else {
        return Vec::new();
    };
    let gear = ld.find_channel_by_name("Gear").and_then(|c| {
        create_plotted_channel(crate::state::ChannelId::Physical(c.index), shared, 0)
    });
    shared
        .sectors
        .iter()
        .filter_map(|s| {
            let (start, end) = i3rs_core::sector_interval(s, lap, track)?;
            let mut label = s.name.clone();
            if let Some((speed, freq)) = &track.corrected_speed {
                let ranges = if end > start {
                    vec![(start, end)]
                } else {
                    vec![(start, lap.end_time), (lap.start_time, end)]
                };
                let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
                for range in ranges {
                    for i in sample_range(speed.len(), *freq, range) {
                        let value = speed[i] / 0.44704;
                        if value.is_finite() {
                            min = min.min(value);
                            max = max.max(value);
                        }
                    }
                }
                if min.is_finite() {
                    label.push_str(&format!("\nCorr Speed {min:.1}–{max:.1} mph"));
                }
            }
            for (kind, channel) in [("Gear", gear.as_ref())] {
                let Some(ch) = channel else {
                    continue;
                };
                let meta = resolve_plotted_channel_display_meta(ch, shared);
                let ranges = if end > start {
                    vec![(start, end)]
                } else {
                    vec![(start, lap.end_time), (lap.start_time, end)]
                };
                let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
                for range in ranges {
                    for i in sample_range(ch.data.len(), meta.2, range) {
                        let value = transform_channel_value(ch, ch.data[i]);
                        if value.is_finite() {
                            min = min.min(value);
                            max = max.max(value);
                        }
                    }
                }
                if min.is_finite() {
                    let dec = if kind == "Gear" { 0 } else { 1 };
                    label.push_str(&format!("\n{kind} {min:.dec$}–{max:.dec$} {}", meta.1));
                }
            }
            Some((
                PlotPoint::new(*track.x.get(s.start_index)?, *track.y.get(s.start_index)?),
                label,
            ))
        })
        .collect()
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
                sample_range: (0, 2),
            },
            solid_segments: vec![vec![PlotPoint::new(0.0, 0.0)]],
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
                sectors: vec![],
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
                sample_range: (0, 2),
            },
            solid_segments: vec![vec![PlotPoint::new(0.0, 0.0)]],
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

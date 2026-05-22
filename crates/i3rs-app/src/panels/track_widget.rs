//! Shared GPS track rendering for full track map panels and embedded gauge-row widgets.

use std::sync::Arc;

use eframe::egui;
use egui_plot::{Line, MarkerShape, Plot, PlotPoint, PlotPoints, Points};
use i3rs_core::{Sector, TrackData, compute_color_map, find_nearest_sample};

use crate::state::{CHANNEL_COLORS, SharedState};

/// Runtime state for drawing a GPS track with optional rainbow coloring and cursor marker.
pub struct TrackWidgetState {
    track_data: Option<Arc<TrackData>>,
    /// Channel index for rainbow coloring (`None` = solid teal line).
    pub color_channel_idx: Option<usize>,
    cached_colors: Option<Arc<[[u8; 4]]>>,
    cached_color_range: Option<(f64, f64)>,
    pub color_channel_name: String,
    cache_fingerprint: Option<(usize, Option<usize>)>,
    track_line_cache: Option<CachedTrackLine>,
    cursor_marker_cache: Option<CachedTrackMarker>,
    sector_marker_cache: Option<CachedSectorMarkers>,
}

/// Options controlling how a track plot is rendered and interacted with.
pub struct TrackPlotOptions {
    pub plot_id: String,
    pub allow_drag: bool,
    pub allow_zoom: bool,
    pub allow_hover_scrub: bool,
    /// When true, clicking the track sets `cursor_time` (embedded widget). Full map uses hover only.
    pub click_sets_cursor: bool,
    pub draw_sectors: bool,
    pub marker_radius: f32,
    /// Plot panel background (embedded gauges use `false` to match borderless analog gauges).
    pub show_background: bool,
    /// Draw a channel name above the plot area (embedded gauge row).
    pub show_gauge_label: bool,
}

impl Default for TrackPlotOptions {
    fn default() -> Self {
        Self {
            plot_id: "track".into(),
            allow_drag: true,
            allow_zoom: true,
            allow_hover_scrub: true,
            click_sets_cursor: false,
            draw_sectors: true,
            marker_radius: 6.0,
            show_background: true,
            show_gauge_label: false,
        }
    }
}

impl TrackPlotOptions {
    pub fn embedded(plot_id: impl Into<String>) -> Self {
        Self {
            plot_id: plot_id.into(),
            allow_drag: false,
            allow_zoom: false,
            allow_hover_scrub: false,
            click_sets_cursor: true,
            draw_sectors: false,
            marker_radius: 4.0,
            show_background: false,
            show_gauge_label: true,
        }
    }
}

pub struct TrackPlotResult {
    pub clicked_sample_idx: Option<usize>,
}

impl TrackWidgetState {
    pub fn new() -> Self {
        Self {
            track_data: None,
            color_channel_idx: None,
            cached_colors: None,
            cached_color_range: None,
            color_channel_name: String::new(),
            cache_fingerprint: None,
            track_line_cache: None,
            cursor_marker_cache: None,
            sector_marker_cache: None,
        }
    }

    pub fn set_color_channel_by_name(&mut self, name: Option<String>, shared: &SharedState) {
        self.color_channel_idx = name.as_ref().and_then(|n| {
            shared
                .ld_file
                .as_ref()
                .and_then(|ld| ld.channels.iter().position(|ch| &ch.name == n))
        });
        self.color_channel_name = name.unwrap_or_default();
        self.invalidate_color_cache();
    }

    pub fn default_color_channel_name(shared: &SharedState) -> Option<String> {
        shared.ld_file.as_ref().and_then(|ld| {
            if ld.channels.iter().any(|ch| ch.name == "Corr Speed") {
                Some("Corr Speed".into())
            } else {
                None
            }
        })
    }

    pub fn clear_cache(&mut self) {
        self.track_data = None;
        self.cached_colors = None;
        self.cached_color_range = None;
        self.color_channel_idx = None;
        self.color_channel_name.clear();
        self.cache_fingerprint = None;
        self.clear_render_caches();
    }

    pub fn invalidate_color_cache(&mut self) {
        self.cached_colors = None;
        self.cached_color_range = None;
        self.track_line_cache = None;
    }

    fn clear_render_caches(&mut self) {
        self.track_line_cache = None;
        self.cursor_marker_cache = None;
        self.sector_marker_cache = None;
    }

    pub fn ensure_track_data(&mut self, shared: &SharedState) {
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

    pub fn ensure_color_map(&mut self, shared: &SharedState) {
        if self.cached_colors.is_some() {
            return;
        }

        let Some(track) = &self.track_data else {
            return;
        };
        let Some(ch_idx) = self.color_channel_idx else {
            return;
        };
        let Some(ld) = &shared.ld_file else {
            return;
        };
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

    pub fn track_data(&self) -> Option<&Arc<TrackData>> {
        self.track_data.as_ref()
    }

    pub fn cached_color_range(&self) -> Option<(f64, f64)> {
        self.cached_color_range
    }

    /// Draw track status when GPS data is unavailable. Returns `true` if ready to plot.
    pub fn show_loading_or_empty(&self, ui: &mut egui::Ui, shared: &SharedState) -> bool {
        let Some(track) = self.track_data() else {
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
            return false;
        };

        if track.x.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("GPS track data is empty");
            });
            return false;
        }

        true
    }

    pub fn show_plot(
        &mut self,
        ui: &mut egui::Ui,
        shared: &mut SharedState,
        sectors: &[Sector],
        options: &TrackPlotOptions,
    ) -> Option<TrackPlotResult> {
        self.ensure_track_data(shared);
        if !self.show_loading_or_empty(ui, shared) {
            return None;
        }

        self.ensure_color_map(shared);

        let track = Arc::clone(self.track_data.as_ref().unwrap());
        let cursor_time = shared.cursor_time;

        let mut hover_idx: Option<usize> = None;
        let mut clicked_idx: Option<usize> = None;

        let plot = Plot::new(&options.plot_id)
            .data_aspect(1.0)
            .allow_drag(options.allow_drag)
            .allow_zoom(options.allow_zoom)
            .allow_scroll(false)
            .show_axes(false)
            .show_grid(false)
            .show_background(options.show_background);

        let colors = self.cached_colors.clone();
        let response = plot.show(ui, |plot_ui| {
            draw_track_line(
                plot_ui,
                &mut self.track_line_cache,
                &track,
                colors.as_deref(),
            );

            if options.draw_sectors {
                draw_sector_markers(
                    plot_ui,
                    &mut self.sector_marker_cache,
                    &track,
                    sectors,
                );
            }

            if let Some(t) = cursor_time {
                draw_cursor_marker(
                    plot_ui,
                    &mut self.cursor_marker_cache,
                    &track,
                    t,
                    options.marker_radius,
                );
            }

            if let Some(coord) = plot_ui.pointer_coordinate() {
                let idx = find_nearest_sample(&track, coord.x, coord.y);
                hover_idx = Some(idx);

                if plot_ui.response().clicked() {
                    clicked_idx = Some(idx);
                }
            }
        });

        if options.allow_hover_scrub
            && response.response.hovered()
            && let Some(idx) = hover_idx
        {
            shared.cursor_time = Some(track.time[idx]);
        }

        if options.click_sets_cursor && let Some(idx) = clicked_idx {
            shared.cursor_time = Some(track.time[idx]);
        }

        Some(TrackPlotResult {
            clicked_sample_idx: clicked_idx,
        })
    }

    pub fn show_plot_in_rect(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &mut SharedState,
        options: &TrackPlotOptions,
    ) -> Option<TrackPlotResult> {
        const LABEL_BAND: f32 = 16.0;
        let plot_rect = if options.show_gauge_label {
            ui.painter_at(rect).text(
                egui::pos2(rect.center().x, rect.min.y + 10.0),
                egui::Align2::CENTER_TOP,
                "Track",
                egui::FontId::proportional(12.0),
                egui::Color32::LIGHT_GRAY,
            );
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + LABEL_BAND),
                rect.max,
            )
        } else {
            rect
        };

        let mut child_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(plot_rect)
                .layout(*ui.layout()),
        );
        child_ui.set_clip_rect(plot_rect);
        self.show_plot(&mut child_ui, shared, &[], options)
    }
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

    let cached = cache.get_or_insert_with(|| CachedTrackLine::build(track, colors, fingerprint));
    if cached.fingerprint != fingerprint {
        *cached = CachedTrackLine::build(track, colors, fingerprint);
    }

    if cached.colored_segments.is_empty() {
        if !cached.solid_points.is_empty() {
            let line = Line::new("Track", PlotPoints::Borrowed(cached.solid_points.as_slice()))
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
    radius: f32,
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
        .radius(radius)
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
    let cached =
        cache.get_or_insert_with(|| CachedSectorMarkers::build(track, sectors, fingerprint.clone()));
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

pub fn draw_color_legend(ui: &mut egui::Ui, vmin: f64, vmax: f64) {
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
            .filter(|(_, sector)| sector.start_index < track.x.len())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_cache_clears_track_render_caches() {
        let mut widget = TrackWidgetState::new();
        widget.track_line_cache = Some(CachedTrackLine {
            fingerprint: TrackLineFingerprint {
                track_ptr: 1,
                track_len: 2,
                colors_ptr: 3,
                colors_len: 4,
            },
            solid_points: vec![PlotPoint::new(0.0, 0.0)],
            colored_segments: Vec::new(),
        });
        widget.cursor_marker_cache = Some(CachedTrackMarker {
            fingerprint: TrackMarkerFingerprint {
                track_ptr: 1,
                track_len: 2,
                sample_idx: 0,
            },
            points: [PlotPoint::new(0.0, 0.0)],
        });

        widget.clear_cache();

        assert!(widget.track_line_cache.is_none());
        assert!(widget.cursor_marker_cache.is_none());
    }

    #[test]
    fn invalidate_color_cache_only_drops_track_line_geometry() {
        let mut widget = TrackWidgetState::new();
        widget.track_line_cache = Some(CachedTrackLine {
            fingerprint: TrackLineFingerprint {
                track_ptr: 1,
                track_len: 2,
                colors_ptr: 3,
                colors_len: 4,
            },
            solid_points: vec![PlotPoint::new(0.0, 0.0)],
            colored_segments: Vec::new(),
        });
        widget.cursor_marker_cache = Some(CachedTrackMarker {
            fingerprint: TrackMarkerFingerprint {
                track_ptr: 1,
                track_len: 2,
                sample_idx: 0,
            },
            points: [PlotPoint::new(0.0, 0.0)],
        });

        widget.invalidate_color_cache();

        assert!(widget.track_line_cache.is_none());
        assert!(widget.cursor_marker_cache.is_some());
    }
}

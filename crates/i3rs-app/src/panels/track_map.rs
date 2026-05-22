//! Track map panel: GPS track visualization with rainbow coloring and sector editing.

use eframe::egui;
use i3rs_core::{SectorTime, compute_sector_times};

use crate::state::SharedState;

use super::track_widget::{TrackPlotOptions, TrackWidgetState, draw_color_legend};

pub struct TrackMapPanel {
    pub id: u64,
    pub title: String,
    widget: TrackWidgetState,
    editing_sectors: bool,
    pending_sector_start: Option<usize>,
    /// Cached sector time report (invalidated when sectors/laps change).
    cached_sector_times: Option<CachedSectorReport>,
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
    sector_count: usize,
    lap_count: usize,
    times: Vec<Vec<SectorTime>>,
}

impl TrackMapPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            widget: TrackWidgetState::new(),
            editing_sectors: false,
            pending_sector_start: None,
            cached_sector_times: None,
            color_filter: String::new(),
            is_popped_out: false,
            pop_out_requested: false,
            dock_requested: false,
            home_worksheet: 0,
        }
    }

    pub fn color_channel_idx(&self) -> Option<usize> {
        self.widget.color_channel_idx
    }

    pub fn set_color_channel_idx(&mut self, idx: Option<usize>) {
        self.widget.color_channel_idx = idx;
        self.widget.invalidate_color_cache();
    }

    /// Clear cached data (called when a new file is opened).
    pub fn clear_cache(&mut self) {
        self.widget.clear_cache();
        self.cached_sector_times = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        let _perf = crate::perf_metrics::scope("track-map draw");

        if self.is_popped_out && ui.input(|i| i.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dock_requested = true;
        }

        self.widget.ensure_track_data(shared);
        if !self.widget.show_loading_or_empty(ui, shared) {
            return;
        }

        self.show_toolbar(ui, shared);
        ui.separator();

        let editing = self.editing_sectors;
        let sectors = shared.sectors.clone();
        let options = TrackPlotOptions {
            plot_id: format!("trackmap_{}", self.id),
            ..TrackPlotOptions::default()
        };

        if let Some(result) = self.widget.show_plot(ui, shared, &sectors, &options)
            && editing
            && let Some(idx) = result.clicked_sample_idx
        {
            if let Some(start) = self.pending_sector_start.take() {
                let sector_num = shared.sectors.len() + 1;
                shared.sectors.push(i3rs_core::Sector {
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

    fn show_toolbar(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        ui.horizontal(|ui| {
            ui.label("Color by:");
            let current_name = if self.widget.color_channel_idx.is_some() {
                &self.widget.color_channel_name
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
                        .selectable_value(&mut self.widget.color_channel_idx, None, "None")
                        .clicked()
                    {
                        self.widget.invalidate_color_cache();
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
                                .selectable_value(
                                    &mut self.widget.color_channel_idx,
                                    Some(i),
                                    label,
                                )
                                .clicked()
                            {
                                self.widget.invalidate_color_cache();
                                self.color_filter.clear();
                            }
                        }
                    }
                });

            if let Some((vmin, vmax)) = self.widget.cached_color_range() {
                ui.separator();
                draw_color_legend(ui, vmin, vmax);
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

    fn show_sector_report(&mut self, ui: &mut egui::Ui, shared: &SharedState) {
        if shared.sectors.is_empty() || shared.laps.is_empty() {
            return;
        }

        let Some(track) = self.widget.track_data() else {
            return;
        };

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

fn delta_color(delta: f64) -> egui::Color32 {
    if delta < -0.01 {
        egui::Color32::from_rgb(100, 255, 100)
    } else if delta > 0.01 {
        egui::Color32::from_rgb(255, 100, 100)
    } else {
        egui::Color32::from_gray(200)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_cache_clears_sector_report() {
        let mut panel = TrackMapPanel::new(1, "Track");
        panel.cached_sector_times = Some(CachedSectorReport {
            sector_count: 1,
            lap_count: 1,
            times: Vec::new(),
        });
        panel.clear_cache();
        assert!(panel.cached_sector_times.is_none());
    }
}

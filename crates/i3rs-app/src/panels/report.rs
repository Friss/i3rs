//! Report panel: per-channel statistics (min/max/avg/stddev) per lap.

use eframe::egui;

use crate::state::SharedState;

pub struct ReportPanel {
    pub id: u64,
    pub title: String,
}

impl ReportPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui, shared: &mut SharedState) {
        if shared.display_channel_registry.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Plot some channels to see statistics");
            });
            return;
        }

        // Rebuild cache if channels or laps changed
        if !shared
            .report_cache
            .is_valid(&shared.display_channel_registry, shared.laps.len())
        {
            shared
                .report_cache
                .rebuild(&shared.display_channel_registry, &shared.laps);
        }

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(format!("report_grid_{}", self.id))
                    .num_columns(6)
                    .spacing([12.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // Header
                        ui.strong("Channel");
                        ui.strong("Lap");
                        ui.strong("Min");
                        ui.strong("Max");
                        ui.strong("Avg");
                        ui.strong("StdDev");
                        ui.end_row();

                        for cached in &shared.report_cache.stats {
                            let dec = cached.dec_places.max(0) as usize;
                            let session = cached.session;

                            ui.colored_label(cached.color, &cached.name);
                            ui.label("All");
                            ui.monospace(format!("{:.prec$}", session.min, prec = dec));
                            ui.monospace(format!("{:.prec$}", session.max, prec = dec));
                            ui.monospace(format!("{:.prec$}", session.avg, prec = dec));
                            ui.monospace(format!("{:.prec$}", session.stddev, prec = dec));
                            ui.end_row();

                            // Per-lap stats
                            for (lap_name, lap_stats) in &cached.per_lap {
                                let is_selected = shared
                                    .selected_lap
                                    .and_then(|idx| shared.laps.get(idx))
                                    .is_some_and(|l| l.name == *lap_name);
                                let label = lap_name.as_str();

                                ui.label(""); // empty channel column
                                if is_selected {
                                    ui.strong(label);
                                } else {
                                    ui.label(label);
                                }
                                ui.monospace(format!("{:.prec$}", lap_stats.min, prec = dec));
                                ui.monospace(format!("{:.prec$}", lap_stats.max, prec = dec));
                                ui.monospace(format!("{:.prec$}", lap_stats.avg, prec = dec));
                                ui.monospace(format!("{:.prec$}", lap_stats.stddev, prec = dec));
                                ui.end_row();
                            }
                        }
                    });
            });
    }
}

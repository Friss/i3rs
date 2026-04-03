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

    pub fn ui(&self, ui: &mut egui::Ui, shared: &SharedState) {
        if shared.display_channel_registry.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Plot some channels to see statistics");
            });
            return;
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

                        for info in &shared.display_channel_registry {
                            let freq = info.freq;
                            let dec = info.dec_places.max(0) as usize;

                            // Full session stats
                            let (min, max, avg, stddev) = compute_stats(&info.data);
                            ui.colored_label(info.color, &info.name);
                            ui.label("All");
                            ui.monospace(format!("{:.prec$}", min, prec = dec));
                            ui.monospace(format!("{:.prec$}", max, prec = dec));
                            ui.monospace(format!("{:.prec$}", avg, prec = dec));
                            ui.monospace(format!("{:.prec$}", stddev, prec = dec));
                            ui.end_row();

                            // Per-lap stats
                            for lap in &shared.laps {
                                let start_sample =
                                    (lap.start_time * freq as f64).floor() as usize;
                                let end_sample =
                                    (lap.end_time * freq as f64).ceil() as usize;
                                let start = start_sample.min(info.data.len());
                                let end = end_sample.min(info.data.len());

                                if start < end {
                                    let slice = &info.data[start..end];
                                    let (lmin, lmax, lavg, lstddev) = compute_stats(slice);

                                    let is_selected = shared.selected_lap
                                        == shared.laps.iter().position(|l| l.number == lap.number);
                                    let label = format!("Lap {}", lap.number);

                                    ui.label(""); // empty channel column
                                    if is_selected {
                                        ui.strong(&label);
                                    } else {
                                        ui.label(&label);
                                    }
                                    ui.monospace(format!("{:.prec$}", lmin, prec = dec));
                                    ui.monospace(format!("{:.prec$}", lmax, prec = dec));
                                    ui.monospace(format!("{:.prec$}", lavg, prec = dec));
                                    ui.monospace(format!("{:.prec$}", lstddev, prec = dec));
                                    ui.end_row();
                                }
                            }
                        }
                    });
            });
    }
}

fn compute_stats(data: &[f64]) -> (f64, f64, f64, f64) {
    crate::state::compute_channel_stats(data)
}

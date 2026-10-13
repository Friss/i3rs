//! Report panel: per-channel statistics (min/max/avg/stddev) per lap.

use eframe::egui;

use super::utils::{
    build_plotted_channel_info, channel_picker, create_plotted_channel, refresh_plotted_channel,
    resolve_plotted_channel_display_meta, show_plotted_channel_display_menu,
};
use crate::state::{ChannelId, PlottedChannel, ReportCache, SharedState};

pub struct ReportPanel {
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    cache: ReportCache,
    filter: String,
}

impl ReportPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            channels: Vec::new(),
            cache: ReportCache::new(),
            filter: String::new(),
        }
    }

    pub fn add_channel(&mut self, id: ChannelId, shared: &SharedState) {
        if !self.channels.iter().any(|c| c.channel_id == id)
            && let Some(pc) = create_plotted_channel(id, shared, self.channels.len())
        {
            self.channels.push(pc);
        }
    }

    pub fn clear_channels(&mut self) {
        self.channels.clear();
        self.cache = ReportCache::new();
    }

    pub fn copy_visible_channels(&mut self, shared: &SharedState) {
        for info in &shared.display_channel_registry {
            self.add_channel(info.channel_id, shared);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        if let Some(id) = shared.pending_toggle_channel.take() {
            if self.channels.iter().any(|c| c.channel_id == id) {
                self.channels.retain(|c| c.channel_id != id);
            } else {
                self.add_channel(id, shared);
            }
        }
        if shared.dragging_channel.is_some()
            && ui.input(|i| i.pointer.any_released())
            && ui.ui_contains_pointer()
            && let Some(id) = shared.dragging_channel.take()
        {
            self.add_channel(id, shared);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Add channel:");
            let mut selected = None;
            channel_picker(
                ui,
                ("report_add", self.id),
                &mut selected,
                &mut self.filter,
                shared,
            );
            if let Some(id) = selected {
                self.add_channel(id, shared);
            }
            let mut remove = None;
            for (idx, pc) in self.channels.iter_mut().enumerate() {
                refresh_plotted_channel(pc, shared);
                let name = resolve_plotted_channel_display_meta(pc, shared).0;
                let (button, clear) =
                    super::utils::segmented_channel_button(ui, &name, None, "Remove from report");
                button.context_menu(|ui| {
                    if show_plotted_channel_display_menu(ui, pc, shared) {
                        remove = Some(idx);
                    }
                });
                if clear {
                    remove = Some(idx);
                }
            }
            if let Some(idx) = remove {
                self.channels.remove(idx);
            }
        });
        ui.separator();
        if self.channels.is_empty() {
            ui.label("Add or drop channels to build this report");
            return;
        }
        let registry: Vec<_> = self
            .channels
            .iter()
            .map(|pc| build_plotted_channel_info(pc, shared))
            .collect();
        if !self.cache.is_valid(&registry, &shared.laps) {
            self.cache.rebuild(&registry, &shared.laps);
        }
        shared.plotted_channel_registry.extend(registry);

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

                        for cached in &self.cache.stats {
                            let dec = cached.dec_places.max(0) as usize;
                            let session = cached.session;

                            ui.colored_label(
                                cached.color,
                                format!("{} [{}]", cached.name, cached.unit),
                            );
                            ui.label("All");
                            ui.monospace(format_stat(session.min, dec));
                            ui.monospace(format_stat(session.max, dec));
                            ui.monospace(format_stat(session.avg, dec));
                            ui.monospace(format_stat(session.stddev, dec));
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
                                ui.monospace(format_stat(lap_stats.min, dec));
                                ui.monospace(format_stat(lap_stats.max, dec));
                                ui.monospace(format_stat(lap_stats.avg, dec));
                                ui.monospace(format_stat(lap_stats.stddev, dec));
                                ui.end_row();
                            }
                        }
                    });
            });
    }
}

fn format_stat(value: f64, decimals: usize) -> String {
    if value.is_finite() {
        format!("{value:.decimals$}")
    } else {
        "—".into()
    }
}

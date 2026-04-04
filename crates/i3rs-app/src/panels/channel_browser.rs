//! Channel browser panel: searchable channel list with drag-and-drop + lap selector.

use eframe::egui;

use crate::state::{ChannelId, SharedState};

/// Render channel browser as a standalone docked panel.
/// Channel toggles go through `shared.pending_toggle_channel`.
pub fn show_standalone(ui: &mut egui::Ui, shared: &mut SharedState) {
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(&mut shared.channel_filter);
    });

    ui.separator();

    if shared.ld_file.is_some() || !shared.math_channels.is_empty() {
        let filter_lower = shared.channel_filter.to_lowercase();
        let mut toggle_id: Option<ChannelId> = None;
        let mut drag_start: Option<ChannelId> = None;

        let has_laps = !shared.laps.is_empty();

        // Bottom region: lap selector (fixed height)
        if has_laps {
            egui::Panel::bottom("lap_selector_panel")
                .default_size(180.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    show_lap_selector(ui, shared);
                });
        }

        // Remaining space: channel list (sorted alphabetically)
        egui::ScrollArea::both()
            .id_salt("channel_list_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Physical channels
                if let Some(ld) = shared.ld_file.clone() {
                    let mut sorted_channels: Vec<_> = ld.channels.iter().collect();
                    sorted_channels
                        .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

                    for ch in &sorted_channels {
                        if !filter_lower.is_empty()
                            && !ch.name.to_lowercase().contains(&filter_lower)
                            && !ch.unit.to_lowercase().contains(&filter_lower)
                        {
                            continue;
                        }
                        let label = format!(
                            "{} [{}] {}Hz",
                            ch.name,
                            if ch.unit.is_empty() { "-" } else { &ch.unit },
                            ch.freq
                        );

                        let response = ui.selectable_label(false, &label);

                        if response.dragged() {
                            drag_start = Some(ChannelId::Physical(ch.index));
                        }

                        if response.clicked() {
                            toggle_id = Some(ChannelId::Physical(ch.index));
                        }
                    }
                }

                // Math channels
                if !shared.math_channels.is_empty() {
                    ui.separator();
                    ui.strong("Math Channels");
                    for (i, mc) in shared.math_channels.iter().enumerate() {
                        if !filter_lower.is_empty()
                            && !mc.name.to_lowercase().contains(&filter_lower)
                        {
                            continue;
                        }
                        // Skip channels with evaluation errors
                        if mc.data.is_none() {
                            let label = format!("\u{26A0} {} (error)", mc.name);
                            ui.colored_label(egui::Color32::from_rgb(200, 100, 100), &label);
                            continue;
                        }
                        let label = format!(
                            "\u{0192} {} [{}] {}Hz",
                            mc.name,
                            if mc.unit.is_empty() { "-" } else { &mc.unit },
                            mc.freq
                        );

                        let response = ui.selectable_label(false, &label);

                        if response.dragged() {
                            drag_start = Some(ChannelId::Math(i));
                        }

                        if response.clicked() {
                            toggle_id = Some(ChannelId::Math(i));
                        }
                    }
                }
            });

        if let Some(id) = drag_start {
            shared.dragging_channel = Some(id);
        }

        if let Some(id) = toggle_id {
            shared.pending_toggle_channel = Some(id);
        }
    } else {
        ui.label("No file loaded. Use File > Open.");
    }
}

fn show_lap_selector(ui: &mut egui::Ui, shared: &mut SharedState) {
    ui.add_space(6.0);
    ui.heading("Laps");

    if let Some(ldx) = &shared.ldx
        && let Some(ref fastest) = ldx.fastest_time
    {
        ui.horizontal(|ui| {
            ui.label("Fastest:");
            ui.strong(fastest);
        });
    }

    let mut new_selection = shared.selected_lap;

    egui::ScrollArea::vertical()
        .id_salt("lap_list_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let all_selected = shared.selected_lap.is_none();
            if ui.selectable_label(all_selected, "All laps").clicked() {
                new_selection = None;
            }

            for (i, lap) in shared.laps.iter().enumerate() {
                let dur = lap.duration();
                let mins = (dur / 60.0) as u32;
                let secs = dur - (mins as f64 * 60.0);
                let label = if mins > 0 {
                    format!("{} - {}:{:05.2}", lap.name, mins, secs)
                } else {
                    format!("{} - {:.2}s", lap.name, secs)
                };

                let is_selected = shared.selected_lap == Some(i);
                if ui.selectable_label(is_selected, &label).clicked() {
                    new_selection = Some(i);
                }
            }
        });

    shared.selected_lap = new_selection;
}

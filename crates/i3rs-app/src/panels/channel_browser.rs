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
                .show(ui, |ui| {
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
                    sorted_channels.sort_by_key(|a| a.name.to_lowercase());

                    for ch in &sorted_channels {
                        let (name, unit, _, _) = super::utils::resolve_channel_meta(
                            ChannelId::Physical(ch.index),
                            shared,
                        );
                        if !filter_lower.is_empty()
                            && !name.to_lowercase().contains(&filter_lower)
                            && !unit.to_lowercase().contains(&filter_lower)
                        {
                            continue;
                        }
                        let label = format!(
                            "{} [{}] {}Hz",
                            name,
                            if unit.is_empty() { "-" } else { &unit },
                            ch.freq
                        );

                        let response = ui.add_enabled(
                            shared.channel_enabled(ChannelId::Physical(ch.index)),
                            egui::Button::selectable(false, &label)
                                .sense(egui::Sense::click_and_drag()),
                        );

                        if response.dragged() {
                            drag_start = Some(ChannelId::Physical(ch.index));
                        }

                        if response.clicked() {
                            toggle_id = Some(ChannelId::Physical(ch.index));
                        }
                    }
                }

                let generated: Vec<_> = shared
                    .channel_catalog()
                    .into_iter()
                    .filter(|c| {
                        matches!(
                            c.key,
                            i3rs_core::ChannelKey::Constant(_)
                                | i3rs_core::ChannelKey::Generated(_)
                        )
                    })
                    .collect();
                if !generated.is_empty() {
                    ui.separator();
                    ui.strong("Generated / recording constants");
                }
                for channel in generated {
                    if !channel.name.to_lowercase().contains(&filter_lower) {
                        continue;
                    }
                    let reference = format!("{}::{}", channel.source, channel.name);
                    let id = shared.analysis_channel(&reference);
                    let response = ui.add_enabled(
                        shared.channel_enabled(id),
                        egui::Button::selectable(
                            false,
                            format!("{} · {}", channel.name, channel.source),
                        )
                        .sense(egui::Sense::click_and_drag()),
                    );
                    if response.dragged() {
                        drag_start = Some(id);
                    }
                    if response.clicked() {
                        toggle_id = Some(id);
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
                            let label = format!("\u{26A0} {} ({:?})", mc.name, mc.evaluation_state);
                            ui.colored_label(egui::Color32::from_rgb(200, 100, 100), &label)
                                .on_hover_text(mc.error.as_deref().unwrap_or("unavailable source"));
                            continue;
                        }
                        let label = format!(
                            "\u{0192} {} [{}] · {} · {}Hz",
                            mc.name,
                            if mc.unit.is_empty() { "-" } else { &mc.unit },
                            mc.options.source,
                            mc.freq
                        );

                        let response = ui.add(
                            egui::Button::selectable(false, &label)
                                .sense(egui::Sense::click_and_drag()),
                        );

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
            if ui.selectable_label(all_selected, "All ranges").clicked() {
                new_selection = None;
            }

            for (i, lap) in shared.laps.iter().enumerate() {
                let label =
                    if let Some(range) = shared.current_analysis().and_then(|a| a.ranges.get(i)) {
                        format!(
                            "{} [{} · {:?}] - {}",
                            lap.name,
                            range.kind.label(),
                            range.trust,
                            i3rs_core::format_duration(lap.duration())
                        )
                    } else {
                        format!(
                            "{} - {}",
                            lap.name,
                            i3rs_core::format_duration(lap.duration())
                        )
                    };

                let is_selected = shared.selected_lap == Some(i);
                if ui.selectable_label(is_selected, &label).clicked() {
                    new_selection = Some(i);
                }
            }
        });

    if shared.selected_lap != new_selection {
        shared.select_lap(new_selection);
    }
}

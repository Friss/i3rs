//! Source resolution and failures, available without opening a maths editor.
use crate::state::{ChannelId, SharedState};
use eframe::egui;

pub fn show(ctx: &egui::Context, shared: &mut SharedState) {
    if !shared.show_channel_status {
        return;
    }
    let mut open = true;
    let mut retry = false;
    egui::Window::new("Channel Status / Messages")
        .open(&mut open)
        .default_width(650.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Retry failed channels").clicked() {
                    retry = true;
                }
                if ui.button("Clear messages").clicked() {
                    shared.analysis_messages.borrow_mut().clear();
                }
            });
            egui::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    ui.strong("Aliases");
                    let aliases: std::collections::BTreeSet<_> = shared
                        .channel_settings
                        .aliases
                        .keys()
                        .chain(shared.channel_aliases.keys())
                        .collect();
                    for name in aliases {
                        let status = match shared.resolve_analysis_reference(name) {
                            i3rs_core::ChannelResolution::Resolved { name, source, .. } => {
                                format!("{source}: {name}")
                            }
                            i3rs_core::ChannelResolution::Unresolved { reason, .. } => reason,
                        };
                        ui.label(format!("{name}: {status}"));
                    }
                    ui.separator();
                    ui.strong("Corrections");
                    ui.label(if shared.is_track_data_build_pending() {
                        "Building corrected speed / distance / GPS"
                    } else {
                        "Correction outputs"
                    });
                    if let Some(error) = &shared.correction_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                    for (name, data) in [
                        ("Corr Speed", &shared.correction_speed),
                        ("Corr Dist", &shared.correction_distance),
                        ("Corr GPS Speed", &shared.correction_gps_speed),
                    ] {
                        ui.label(data.as_ref().map_or_else(
                            || format!("{name}: unavailable"),
                            |(v, f)| format!("{name}: {} samples at {f} Hz", v.len()),
                        ));
                    }
                    ui.separator();
                    ui.strong("Maths");
                    for (idx, mc) in shared.math_channels.iter().enumerate() {
                        ui.label(format!(
                            "{} [{}] · {} · {:?}{}",
                            mc.name,
                            mc.unit,
                            mc.options.source,
                            mc.evaluation_state,
                            if shared.channel_enabled(ChannelId::Math(idx)) {
                                ""
                            } else {
                                " · source unavailable"
                            }
                        ));
                        if let Some(error) = &mc.error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        if !mc.options.comment.is_empty() {
                            ui.weak(&mc.options.comment);
                        }
                    }
                    ui.separator();
                    ui.strong("Logged channel failures");
                    for (idx, error) in shared.channel_errors.borrow().iter() {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            format!(
                                "{}: {error}",
                                shared
                                    .ld_file
                                    .as_ref()
                                    .and_then(|ld| ld.channels.get(*idx))
                                    .map_or("Missing logged channel", |ch| ch.name.as_str())
                            ),
                        );
                    }
                    ui.separator();
                    ui.strong("Messages");
                    for message in shared.analysis_messages.borrow().iter().rev() {
                        let recording = if message.session_id == shared.session_id {
                            "Current recording"
                        } else {
                            "Earlier recording"
                        };
                        ui.label(format!("{recording}: {}", message.text));
                    }
                });
        });
    shared.show_channel_status = open;
    if retry {
        shared.channel_errors.borrow_mut().clear();
        shared.invalidate_correction_caches();
        shared.invalidate_math_results();
    }
}

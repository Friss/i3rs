//! Cursor readout panel: shows interpolated channel values at the cursor time.

use eframe::egui;

use crate::state::SharedState;

/// Render active-component or searchable all-channel Values. Only visible rows
/// request source decoding, keeping the all-channel view lazy.
pub fn show(ui: &mut egui::Ui, shared: &mut SharedState) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut shared.values_all_channels, false, "Active component");
        ui.selectable_value(&mut shared.values_all_channels, true, "All channels");
    });
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut shared.values_filter)
                .desired_width(ui.available_width()),
        );
        if ui.small_button("Status / Messages").clicked() {
            shared.show_channel_status = true;
        }
    });
    let selection = shared.range_selection();
    if let Some(kind) = selection.kind {
        ui.weak(format!("{} · {:?}", kind.label(), selection.trust));
    }
    let time = shared.cursor_time;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(time.is_some(), egui::Button::new("Set cursor B"))
            .clicked()
        {
            shared.measurement_cursor = time;
        }
        if ui.small_button("Clear B").clicked() {
            shared.measurement_cursor = None;
        }
    });
    if let (Some(a), Some(b)) = (time, shared.measurement_cursor) {
        ui.label(format!("A {a:.3} s · B {b:.3} s · A − B {:+.3} s", a - b));
        if let Some(axis) = &shared.distance_axis_cache
            && let (Some(da), Some(db)) = (
                i3rs_core::sample_at_time(&axis.data, axis.freq, a),
                i3rs_core::sample_at_time(&axis.data, axis.freq, b),
            )
        {
            ui.label(format!("Distance A − B: {:+.1} m", da - db));
        }
        ui.collapsing("Interval measurements", |ui| {
            for info in &shared.display_channel_registry {
                let window = i3rs_core::sample_window_for_time(
                    info.freq,
                    info.data.len(),
                    a.min(b),
                    a.max(b),
                );
                let samples: Vec<_> = info.data[window.start_sample..window.end_sample]
                    .iter()
                    .map(|v| info.transform_value(*v))
                    .collect();
                let stats = i3rs_core::compute_stats(&samples, Some(info.freq));
                let read = |t| {
                    i3rs_core::channels::sample_with_policy(
                        &info.data,
                        info.freq,
                        t,
                        info.interpolation,
                    )
                    .map(|v| info.transform_value(v))
                };
                let delta = read(a).zip(read(b)).map(|(x, y)| x - y);
                let number = |v: Option<f64>| v.map_or_else(|| "—".into(), |v| format!("{v:.3}"));
                ui.collapsing(
                    format!(
                        "{} · Δ {} {} · {} / {} finite samples",
                        info.name,
                        number(delta),
                        info.unit,
                        stats.finite_count,
                        stats.sample_count
                    ),
                    |ui| {
                        ui.label(format!(
                            "A {} · B {} · min {} · max {}",
                            number(read(a)),
                            number(read(b)),
                            number(stats.min),
                            number(stats.max)
                        ));
                        let pair = read(a).zip(read(b));
                        ui.label(format!(
                            "cursor average {} {} · cursor slope {} {}/s",
                            number(pair.map(|(x, y)| (x + y) / 2.)),
                            info.unit,
                            number(pair.filter(|_| a != b).map(|(x, y)| (x - y) / (a - b))),
                            info.unit
                        ));
                        ui.label(format!(
                            "mean {} · σ {} · RMS {} · integral {} {}·s",
                            number(stats.mean),
                            number(stats.stddev),
                            number(stats.rms),
                            number(stats.integral),
                            info.unit
                        ));
                    },
                );
            }
        });
    }
    if let (Some(a), Some(b)) = (time, shared.measurement_cursor) {
        let channels: Vec<_> = shared
            .display_channel_registry
            .iter()
            .filter_map(|info| {
                let mut pc = super::utils::create_plotted_channel(info.channel_id, shared, 0)?;
                pc.display_scale = info.display_scale;
                pc.display_offset = info.display_offset;
                pc.display_unit = Some(info.unit.clone());
                Some(pc)
            })
            .collect();
        let slots: Vec<_> = channels.iter().collect();
        let overlays = crate::comparison::overlay_populations_in_range(
            shared,
            &slots,
            &Default::default(),
            (a.min(b), a.max(b)),
        );
        ui.collapsing("Overlay interval measurements", |ui| {
            for overlay in overlays {
                ui.colored_label(overlay.color, overlay.label);
                let (oa, ob) = if a < b {
                    (overlay.range.0, overlay.range.1)
                } else {
                    (overlay.range.1, overlay.range.0)
                };
                for info in overlay.channels {
                    let w = i3rs_core::sample_window_for_time(
                        info.freq,
                        info.data.len(),
                        overlay.range.0,
                        overlay.range.1,
                    );
                    let samples: Vec<_> = info.data[w.start_sample..w.end_sample]
                        .iter()
                        .map(|v| info.transform_value(*v))
                        .collect();
                    let s = i3rs_core::compute_stats(&samples, Some(info.freq));
                    let read = |t| {
                        i3rs_core::channels::sample_with_policy(
                            &info.data,
                            info.freq,
                            t,
                            info.interpolation,
                        )
                        .map(|v| info.transform_value(v))
                    };
                    let pair = read(oa).zip(read(ob));
                    let number =
                        |v: Option<f64>| v.map_or_else(|| "—".into(), |v| format!("{v:.3}"));
                    ui.collapsing(
                        format!("{} · {} finite samples", info.name, s.finite_count),
                        |ui| {
                            ui.label(format!(
                                "A {} · B {} · Δ {} · cursor average {} {} · cursor slope {} {}/s",
                                number(read(oa)),
                                number(read(ob)),
                                number(pair.map(|(x, y)| x - y)),
                                number(pair.map(|(x, y)| (x + y) / 2.)),
                                info.unit,
                                number(pair.filter(|_| oa != ob).map(|(x, y)| (x - y) / (oa - ob))),
                                info.unit
                            ));
                            ui.label(format!(
                                "min {} · max {} · mean {} · σ {} · RMS {} · integral {} {}·s",
                                number(s.min),
                                number(s.max),
                                number(s.mean),
                                number(s.stddev),
                                number(s.rms),
                                number(s.integral),
                                info.unit
                            ));
                        },
                    );
                }
            }
        });
    }

    ui.label(time.map_or_else(
        || "Hover a graph for cursor values".into(),
        |t| format!("Time: {t:.3} s"),
    ));
    if let Some(t) = time
        && let Some(axis) = &shared.distance_axis_cache
        && let Some(distance) = i3rs_core::sample_at_time(&axis.data, axis.freq, t)
    {
        ui.label(format!("Distance: {distance:.1} m"));
    }
    ui.separator();
    let filter = shared.values_filter.to_lowercase();
    if shared.values_all_channels {
        if shared.channel_settings.enabled("MoTeC") {
            shared.request_track_data_build();
        }
        let mut channels: Vec<_> = shared
            .ld_file
            .as_ref()
            .into_iter()
            .flat_map(|ld| ld.channels.iter())
            .map(|ch| crate::state::ChannelId::Physical(ch.index))
            .chain((0..shared.math_channels.len()).map(crate::state::ChannelId::Math))
            .map(|id| (id, super::utils::resolve_channel_meta(id, shared).0))
            .filter(|(_, name)| name.to_lowercase().contains(&filter))
            .collect();
        channels.sort_by_key(|(_, name)| name.to_lowercase());
        let height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("all_values")
            .show_rows(ui, height, channels.len(), |ui, rows| {
                for row in rows {
                    let (id, name) = &channels[row];
                    if let Some(pc) = super::utils::create_plotted_channel(*id, shared, row) {
                        let info = super::utils::build_plotted_channel_info(&pc, shared);
                        value_row(ui, &info, time);
                    } else {
                        ui.label(format!("{name}: unavailable source"));
                    }
                }
            });
        for (name, unit, data) in [
            ("Corr Speed", "m/s", shared.correction_speed.as_ref()),
            ("Corr Dist", "m", shared.correction_distance.as_ref()),
            (
                "Corr GPS Speed",
                "m/s",
                shared.correction_gps_speed.as_ref(),
            ),
        ] {
            if name.to_lowercase().contains(&filter) && shared.channel_settings.enabled("MoTeC") {
                let value = data.and_then(|(samples, f)| {
                    time.and_then(|t| {
                        i3rs_core::channels::sample_with_policy(
                            samples,
                            *f,
                            t,
                            i3rs_core::Interpolation::Linear,
                        )
                    })
                });
                ui.label(value.map_or_else(
                    || format!("{name}: unavailable source"),
                    |v| format!("{name}: {v:.2} {unit}"),
                ));
            }
        }
        for constant in shared.channel_settings.constants.iter().filter(|c| {
            c.recording == shared.recording_key() && c.name.to_lowercase().contains(&filter)
        }) {
            ui.label(format!(
                "{}: {} {}",
                constant.name, constant.value, constant.unit
            ))
            .on_hover_text(&constant.comment);
        }
    } else {
        egui::ScrollArea::vertical()
            .id_salt("component_values")
            .show(ui, |ui| {
                for info in shared
                    .display_channel_registry
                    .iter()
                    .filter(|info| info.name.to_lowercase().contains(&filter))
                {
                    value_row(ui, info, time);
                }
            });
    }
}

fn value_row(ui: &mut egui::Ui, info: &crate::state::PlottedChannelInfo, time: Option<f64>) {
    let raw = time.and_then(|time| {
        i3rs_core::channels::sample_with_policy(
            &info.data,
            info.freq,
            time,
            if info.uses_discrete_values() {
                i3rs_core::channels::Interpolation::Previous
            } else {
                info.interpolation
            },
        )
    });
    let text = raw.map_or_else(
        || "---".into(),
        |raw| {
            let display = if info.uses_discrete_values() {
                raw
            } else {
                info.transform_value(raw)
            };
            info.format_value(display).unwrap_or_else(|| {
                format!(
                    "{:.precision$} {}",
                    display,
                    info.unit,
                    precision = info.dec_places.clamp(0, 12) as usize
                )
            })
        },
    );
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, info.color);
        let width = ui.available_width();
        let value_width = (width * 0.4).min(110.0);
        let name_width = (width - value_width - ui.spacing().item_spacing.x).max(0.0);
        let height = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            egui::vec2(name_width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(&info.name).truncate());
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(value_width, height),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace()).truncate());
            },
        );
    });
}

/// Read a channel value at a given time, using sample-hold semantics for discrete channels.
pub fn value_at_time(data: &[f64], freq: u16, time: f64, discrete: bool) -> f64 {
    if discrete {
        sample_hold_at_time(data, freq, time)
    } else {
        interpolate_at_time(data, freq, time)
    }
}

/// Linearly interpolate a channel value at a given time.
pub fn interpolate_at_time(data: &[f64], freq: u16, time: f64) -> f64 {
    i3rs_core::analysis::sample_at_time(data, freq, time).unwrap_or(f64::NAN)
}

/// Read the most recent sample at or before the given time.
pub fn sample_hold_at_time(data: &[f64], freq: u16, time: f64) -> f64 {
    if data.is_empty() || freq == 0 || !time.is_finite() || time < 0.0 {
        return f64::NAN;
    }

    let idx = (time * freq as f64).floor() as usize;
    if idx >= data.len() {
        return f64::NAN;
    }

    data[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_blends_between_samples() {
        let data = [0.0, 10.0];
        assert_eq!(interpolate_at_time(&data, 1, 0.5), 5.0);
    }

    #[test]
    fn sample_hold_uses_current_sample_for_discrete_channels() {
        let data = [0.0, 10.0];
        assert_eq!(sample_hold_at_time(&data, 1, 0.5), 0.0);
        assert_eq!(value_at_time(&data, 1, 0.5, true), 0.0);
    }
}

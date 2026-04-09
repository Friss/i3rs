//! Cursor readout panel: shows interpolated channel values at the cursor time.

use eframe::egui;

use crate::state::SharedState;

/// Render the cursor readout panel.
pub fn show(ui: &mut egui::Ui, shared: &SharedState) {
    let cursor_time = match shared.cursor_time {
        Some(t) => t,
        None => {
            ui.centered_and_justified(|ui| {
                ui.label("Hover over a graph to see values");
            });
            return;
        }
    };

    ui.add(
        egui::Label::new(egui::RichText::new(format!("Time: {:.3}s", cursor_time)).heading())
            .truncate(),
    );
    ui.separator();

    if shared.display_channel_registry.is_empty() {
        ui.label("No channels plotted");
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([true, false])
        .show(ui, |ui| {
            for info in &shared.display_channel_registry {
                let discrete = info.uses_discrete_values();
                let value = value_at_time(&info.data, info.freq, cursor_time, discrete);
                let display_value = if discrete {
                    value
                } else {
                    info.transform_value(value)
                };

                let value_text = if let Some(label) = info.format_value(display_value) {
                    label
                } else {
                    let dec = info.dec_places.max(0) as usize;
                    if info.unit.is_empty() {
                        format!("{:.prec$}", display_value, prec = dec)
                    } else {
                        format!("{:.prec$} {}", display_value, info.unit, prec = dec)
                    }
                };

                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, info.color);

                    let gap_width = ui.spacing().item_spacing.x;
                    let content_width = (ui.available_width() - gap_width * 2.0).max(0.0);
                    let value_width = (content_width * 0.34).clamp(24.0, 110.0).min(content_width);
                    let name_width = (content_width - value_width).max(0.0);
                    let row_height = ui.spacing().interact_size.y;

                    ui.allocate_ui_with_layout(
                        egui::vec2(name_width, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(name_width);
                            ui.add(
                                egui::Label::new(&info.name)
                                    .truncate()
                                    .halign(egui::Align::LEFT),
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(value_width, row_height),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_width(value_width);
                            ui.add(
                                egui::Label::new(egui::RichText::new(value_text).monospace())
                                    .truncate()
                                    .halign(egui::Align::RIGHT),
                            );
                        },
                    );
                });
            }
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
    if data.is_empty() || freq == 0 {
        return 0.0;
    }

    let sample_f = time * freq as f64;
    let idx = sample_f as usize;

    if idx >= data.len() {
        return *data.last().unwrap_or(&0.0);
    }

    let next_idx = idx + 1;
    if next_idx >= data.len() {
        return data[idx];
    }

    let frac = sample_f - idx as f64;
    data[idx] * (1.0 - frac) + data[next_idx] * frac
}

/// Read the most recent sample at or before the given time.
pub fn sample_hold_at_time(data: &[f64], freq: u16, time: f64) -> f64 {
    if data.is_empty() || freq == 0 {
        return 0.0;
    }

    let idx = (time * freq as f64).floor() as usize;
    if idx >= data.len() {
        return *data.last().unwrap_or(&0.0);
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

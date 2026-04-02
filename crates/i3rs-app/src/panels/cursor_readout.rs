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

    ui.heading(format!("Time: {:.3}s", cursor_time));
    ui.separator();

    if shared.display_channel_registry.is_empty() {
        ui.label("No channels plotted");
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("readout_grid")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for info in &shared.display_channel_registry {
                        let value = interpolate_at_time(&info.data, info.freq, cursor_time);

                        // Color swatch
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, info.color);

                        // Channel name
                        ui.label(&info.name);

                        // Value + unit
                        let dec = info.dec_places.max(0) as usize;
                        if info.unit.is_empty() {
                            ui.monospace(format!("{:.prec$}", value, prec = dec));
                        } else {
                            ui.monospace(format!("{:.prec$} {}", value, info.unit, prec = dec));
                        }

                        ui.end_row();
                    }
                });
        });
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

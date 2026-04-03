//! Math channel editor: define, edit, and manage computed channels.

use eframe::egui;
use i3rs_core::math_engine::ChannelData;
use std::collections::HashMap;
use std::sync::Arc;

use crate::panels::graph::GraphPanel;
use crate::state::{MathChannelDef, SharedState};

/// State for the "new channel" entry form.
pub struct MathEditorState {
    pub new_name: String,
    pub new_expression: String,
    pub new_unit: String,
}

impl MathEditorState {
    pub fn new() -> Self {
        Self {
            new_name: String::new(),
            new_expression: String::new(),
            new_unit: String::new(),
        }
    }
}

/// Show the math editor UI.
pub fn show(ui: &mut egui::Ui, shared: &mut SharedState, editor: &mut MathEditorState) {
    ui.heading("Math Channels");
    ui.separator();

    // New channel form
    ui.group(|ui| {
        ui.label("New Math Channel");
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut editor.new_name);
        });
        ui.horizontal(|ui| {
            ui.label("Unit:");
            ui.add(egui::TextEdit::singleline(&mut editor.new_unit).desired_width(60.0));
        });
        ui.label("Expression:");
        ui.add(
            egui::TextEdit::multiline(&mut editor.new_expression)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .code_editor(),
        );
        if ui
            .add_enabled(!editor.new_name.is_empty() && !editor.new_expression.is_empty(), egui::Button::new("Add Channel"))
            .clicked()
        {
            let mut mc = MathChannelDef::new(
                editor.new_name.clone(),
                editor.new_expression.clone(),
                editor.new_unit.clone(),
                2,
            );
            evaluate_math_channel(&mut mc, shared);
            shared.math_channels.push(mc);
            editor.new_name.clear();
            editor.new_expression.clear();
            editor.new_unit.clear();
        }
    });

    ui.separator();

    // Existing channels
    if shared.math_channels.is_empty() {
        ui.label("No math channels defined.");
        return;
    }

    let mut to_remove: Option<usize> = None;
    let mut to_reevaluate: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("math_channels_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, mc) in shared.math_channels.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&mc.name);
                        if let Some(ref err) = mc.error {
                            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), "\u{26A0}");
                            ui.label(err);
                        } else if mc.data.is_some() {
                            ui.colored_label(egui::Color32::from_rgb(100, 255, 100), "\u{2713}");
                            ui.label(format!("{}Hz, {} samples", mc.freq, mc.data.as_ref().map_or(0, |d| d.len())));
                        }
                    });

                    let mut expression = mc.expression.clone();
                    let changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut expression)
                                .desired_width(f32::INFINITY)
                                .code_editor(),
                        )
                        .changed();
                    if changed {
                        mc.expression = expression;
                    }

                    ui.horizontal(|ui| {
                        if ui.button("Evaluate").clicked() {
                            to_reevaluate = Some(i);
                        }
                        if ui.button("\u{1F5D1} Delete").clicked() {
                            to_remove = Some(i);
                        }
                    });
                });
            }
        });

    if let Some(idx) = to_reevaluate {
        evaluate_single_math_channel(shared, idx);
    }

    if let Some(idx) = to_remove {
        shared.math_channels.remove(idx);
    }
}

/// Build the channel data map for evaluation, decoding only channels referenced by the expression.
fn build_channel_data_map(
    shared: &SharedState,
    expression: &str,
    exclude_idx: usize,
) -> HashMap<String, ChannelData> {
    let mut channel_data: HashMap<String, ChannelData> = HashMap::new();

    // Parse expression to find which channels are referenced
    let refs: Vec<String> = match i3rs_core::parse_expression(expression) {
        Ok(expr) => i3rs_core::referenced_channels(&expr),
        Err(_) => return channel_data,
    };

    // Only decode physical channels that are referenced
    if let Some(ld) = &shared.ld_file {
        for ch in &ld.channels {
            let needed = refs.iter().any(|r| {
                r == &ch.name
                    || r.replace('_', " ") == ch.name
                    || r.replace('_', ".") == ch.name
                    || r.eq_ignore_ascii_case(&ch.name)
            });
            if needed {
                if let Some(data) = ld.read_channel_data(ch) {
                    channel_data.insert(
                        ch.name.clone(),
                        ChannelData {
                            samples: data,
                            freq: ch.freq,
                        },
                    );
                }
            }
        }
    }

    // Add other evaluated math channels that are referenced
    for (i, other) in shared.math_channels.iter().enumerate() {
        if i != exclude_idx && refs.iter().any(|r| r == &other.name) {
            if let Some(ref data) = other.data {
                channel_data.insert(
                    other.name.clone(),
                    ChannelData {
                        samples: (**data).clone(),
                        freq: other.freq,
                    },
                );
            }
        }
    }

    channel_data
}

/// Evaluate a single math channel definition (used when adding a new one).
pub fn evaluate_math_channel(mc: &mut MathChannelDef, shared: &SharedState) {
    let channel_data = build_channel_data_map(shared, &mc.expression, usize::MAX);
    eval_mc(mc, &channel_data);
}

/// Evaluate a single math channel by index within shared.math_channels.
fn evaluate_single_math_channel(shared: &mut SharedState, idx: usize) {
    let expr = shared.math_channels[idx].expression.clone();
    let channel_data = build_channel_data_map(shared, &expr, idx);
    eval_mc(&mut shared.math_channels[idx], &channel_data);
}

fn eval_mc(mc: &mut MathChannelDef, channel_data: &HashMap<String, ChannelData>) {
    match i3rs_core::evaluate_expression(&mc.expression, channel_data) {
        Ok((samples, freq)) => {
            let (min, max, avg) = GraphPanel::compute_stats(&samples);
            mc.freq = freq;
            mc.data = Some(Arc::new(samples));
            mc.error = None;
            mc.cached_min = min;
            mc.cached_max = max;
            mc.cached_avg = avg;
        }
        Err(e) => {
            mc.data = None;
            mc.error = Some(e);
        }
    }
}

/// Evaluate all math channels in order.
pub fn evaluate_all_math_channels(shared: &mut SharedState) {
    for i in 0..shared.math_channels.len() {
        let expr = shared.math_channels[i].expression.clone();
        let channel_data = build_channel_data_map(shared, &expr, i);
        eval_mc(&mut shared.math_channels[i], &channel_data);
    }
}

/// Save math channels to a JSON file.
pub fn save_math_channels(shared: &SharedState) {
    let configs: Vec<crate::workspace::MathChannelConfig> = shared
        .math_channels
        .iter()
        .map(|mc| crate::workspace::MathChannelConfig {
            name: mc.name.clone(),
            expression: mc.expression.clone(),
            unit: mc.unit.clone(),
            dec_places: mc.dec_places,
        })
        .collect();

    if let Ok(json) = serde_json::to_string_pretty(&configs)
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("Math Channels", &["json"])
            .set_file_name("math_channels.json")
            .save_file()
        && let Err(e) = std::fs::write(&path, json)
    {
        eprintln!("Failed to save math channels: {}", e);
    }
}

/// Load math channels from a JSON file and evaluate them.
pub fn load_math_channels(shared: &mut SharedState) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Math Channels", &["json"])
        .pick_file()
    {
        match std::fs::read_to_string(&path) {
            Ok(json) => {
                match serde_json::from_str::<Vec<crate::workspace::MathChannelConfig>>(&json) {
                    Ok(configs) => {
                        for config in configs {
                            let mc = MathChannelDef::new(
                                config.name,
                                config.expression,
                                config.unit,
                                config.dec_places,
                            );
                            shared.math_channels.push(mc);
                        }
                        evaluate_all_math_channels(shared);
                    }
                    Err(e) => eprintln!("Failed to parse math channels: {}", e),
                }
            }
            Err(e) => eprintln!("Failed to read file: {}", e),
        }
    }
}

//! Math channel editor: define, edit, and manage computed channels.

use eframe::egui;
use i3rs_core::math_engine::ChannelData;
use std::collections::HashMap;
use std::sync::Arc;

use crate::background_jobs::EvaluatedMathChannel;
use crate::state::{MathChannelDef, MathEvaluationState, SharedState};

/// A predefined math channel template.
struct PredefinedCalc {
    name: &'static str,
    expression: &'static str,
    unit: &'static str,
    category: &'static str,
    description: &'static str,
}

const PREDEFINED_CALCS: &[PredefinedCalc] = &[
    // Wheel slip
    PredefinedCalc {
        name: "Wheel Slip FL",
        expression: "(WheelSpeed_FL - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        category: "Traction",
        description: "Front-left wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip FR",
        expression: "(WheelSpeed_FR - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        category: "Traction",
        description: "Front-right wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip RL",
        expression: "(WheelSpeed_RL - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        category: "Traction",
        description: "Rear-left wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip RR",
        expression: "(WheelSpeed_RR - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        category: "Traction",
        description: "Rear-right wheel slip percentage",
    },
    // Chassis dynamics
    PredefinedCalc {
        name: "Pitch Angle",
        expression: "rad_to_deg(atan2(G_Force_Long, 9.81))",
        unit: "deg",
        category: "Chassis",
        description: "Vehicle pitch angle from longitudinal G",
    },
    PredefinedCalc {
        name: "Roll Angle",
        expression: "rad_to_deg(atan2(G_Force_Lat, 9.81))",
        unit: "deg",
        category: "Chassis",
        description: "Vehicle roll angle from lateral G",
    },
    PredefinedCalc {
        name: "Total G-Force",
        expression: "sqrt(pow(G_Force_Lat, 2) + pow(G_Force_Long, 2))",
        unit: "G",
        category: "Chassis",
        description: "Combined lateral and longitudinal G-force",
    },
    // Steering / oversteer
    PredefinedCalc {
        name: "Oversteer Angle",
        expression: "rad_to_deg(atan2(Yaw_Rate * 2.5, max(GPS_Speed, 1))) - Steering_Angle",
        unit: "deg",
        category: "Handling",
        description: "Oversteer angle (positive = oversteer). Assumes 2.5m wheelbase.",
    },
    PredefinedCalc {
        name: "Steering Rate",
        expression: "derivative(Steering_Angle)",
        unit: "deg/s",
        category: "Handling",
        description: "Rate of steering wheel angle change",
    },
    // Engine
    PredefinedCalc {
        name: "Throttle Smoothed",
        expression: "smooth(Throttle_Pos, 10)",
        unit: "%",
        category: "Engine",
        description: "Smoothed throttle position (10-sample window)",
    },
    PredefinedCalc {
        name: "Speed Delta",
        expression: "derivative(GPS_Speed)",
        unit: "km/h/s",
        category: "Performance",
        description: "Rate of speed change (acceleration/deceleration)",
    },
    PredefinedCalc {
        name: "Distance",
        expression: "integrate(GPS_Speed / 3.6)",
        unit: "m",
        category: "Performance",
        description: "Cumulative distance driven (from GPS speed)",
    },
];

/// State for the "new channel" entry form.
pub struct MathEditorState {
    pub new_name: String,
    pub new_expression: String,
    pub new_unit: String,
    pub show_templates: bool,
    pub show_aliases: bool,
    pub new_alias_name: String,
    pub new_alias_target: String,
}

impl MathEditorState {
    pub fn new() -> Self {
        Self {
            new_name: String::new(),
            new_expression: String::new(),
            new_unit: String::new(),
            show_templates: false,
            show_aliases: false,
            new_alias_name: String::new(),
            new_alias_target: String::new(),
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
        let name_conflict = is_duplicate_name(&editor.new_name, shared, None);
        if name_conflict {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 100),
                format!("⚠ A channel named \"{}\" already exists", editor.new_name),
            );
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !editor.new_name.is_empty()
                        && !editor.new_expression.is_empty()
                        && !name_conflict,
                    egui::Button::new("Add Channel"),
                )
                .clicked()
            {
                let math_channel = shared.create_math_channel_def(
                    editor.new_name.clone(),
                    editor.new_expression.clone(),
                    editor.new_unit.clone(),
                    2,
                );
                shared.math_channels.push(math_channel);
                queue_math_channel_evaluation(shared, shared.math_channels.len() - 1);
                shared.invalidate_derived_caches();
                editor.new_name.clear();
                editor.new_expression.clear();
                editor.new_unit.clear();
            }
            if ui
                .button(if editor.show_templates {
                    "Hide Templates"
                } else {
                    "Templates..."
                })
                .clicked()
            {
                editor.show_templates = !editor.show_templates;
            }
        });
    });

    // Predefined calculation templates
    if editor.show_templates {
        ui.separator();
        ui.group(|ui| {
            ui.label("Predefined Calculations");
            ui.label(
                egui::RichText::new(
                    "Click to populate the form above. Adjust channel names to match your data.",
                )
                .small()
                .weak(),
            );

            let mut current_category = "";
            for calc in PREDEFINED_CALCS {
                if calc.category != current_category {
                    current_category = calc.category;
                    ui.add_space(4.0);
                    ui.strong(current_category);
                }
                ui.horizontal(|ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text(calc.description)
                        .clicked()
                    {
                        editor.new_name = calc.name.to_string();
                        editor.new_expression = calc.expression.to_string();
                        editor.new_unit = calc.unit.to_string();
                    }
                    ui.label(egui::RichText::new(calc.name).strong());
                    ui.weak(format!("[{}]", calc.unit));
                });
            }
        });
    }

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
                            ui.label(format!(
                                "{}Hz, {} samples",
                                mc.freq,
                                mc.data.as_ref().map_or(0, |d| d.len())
                            ));
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
        queue_math_channel_evaluation(shared, idx);
        shared.invalidate_derived_caches();
    }

    if let Some(idx) = to_remove {
        if let Some(math_id) = shared.math_channels.get(idx).map(|mc| mc.id) {
            shared.cancel_math_channel_evaluation(math_id);
        }
        shared.math_channels.remove(idx);
        shared.invalidate_derived_caches();
    }

    // Channel aliases section
    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .selectable_label(editor.show_aliases, "Channel Aliases")
            .clicked()
        {
            editor.show_aliases = !editor.show_aliases;
        }
        if !shared.channel_aliases.is_empty() {
            ui.weak(format!("({})", shared.channel_aliases.len()));
        }
    });

    if editor.show_aliases {
        ui.group(|ui| {
            ui.label(
                egui::RichText::new(
                    "Map alternative channel names to existing channels. \
                     Aliases are used when resolving channel references in math expressions.",
                )
                .small()
                .weak(),
            );
            ui.horizontal(|ui| {
                ui.label("Alias:");
                ui.add(egui::TextEdit::singleline(&mut editor.new_alias_name).desired_width(100.0));
                ui.label("\u{2192}");
                ui.label("Target:");
                ui.add(
                    egui::TextEdit::singleline(&mut editor.new_alias_target).desired_width(100.0),
                );
                if ui
                    .add_enabled(
                        !editor.new_alias_name.is_empty() && !editor.new_alias_target.is_empty(),
                        egui::Button::new("Add"),
                    )
                    .clicked()
                {
                    shared.channel_aliases.insert(
                        editor.new_alias_name.clone(),
                        editor.new_alias_target.clone(),
                    );
                    editor.new_alias_name.clear();
                    editor.new_alias_target.clear();
                }
            });

            // List existing aliases
            let mut to_remove_alias: Option<String> = None;
            for (alias, target) in &shared.channel_aliases {
                ui.horizontal(|ui| {
                    ui.monospace(alias);
                    ui.weak("\u{2192}");
                    ui.monospace(target);
                    if ui.small_button("x").clicked() {
                        to_remove_alias = Some(alias.clone());
                    }
                });
            }
            if let Some(alias) = to_remove_alias {
                shared.channel_aliases.remove(&alias);
            }
        });
    }
}

/// Build the channel data map for evaluation, decoding only channels referenced by the expression.
fn build_channel_data_map(
    shared: &SharedState,
    expression: &str,
    exclude_math_id: u64,
) -> EvaluationInputs {
    let mut channel_data: HashMap<String, ChannelData> = HashMap::new();
    let mut waiting_on_inputs = false;

    // Parse expression to find which channels are referenced
    let refs: Vec<String> = match i3rs_core::parse_expression(expression) {
        Ok(expr) => i3rs_core::referenced_channels(&expr),
        Err(_) => {
            return EvaluationInputs {
                channel_data,
                waiting_on_inputs: false,
            };
        }
    };

    // Expand alias targets so we also load channels referenced indirectly
    let mut resolved_refs: Vec<String> = Vec::with_capacity(refs.len() * 2);
    for r in &refs {
        resolved_refs.push(r.clone());
        if let Some(target) = i3rs_core::resolve_alias_target(r, &shared.channel_aliases) {
            resolved_refs.push(target);
        }
    }

    // Only decode physical channels that are referenced
    if let Some(ld) = &shared.ld_file {
        for ch in &ld.channels {
            let needed = resolved_refs.iter().any(|r| {
                r == &ch.name
                    || r.replace('_', " ") == ch.name
                    || r.replace('_', ".") == ch.name
                    || r.eq_ignore_ascii_case(&ch.name)
            });
            if needed {
                if let Some(decoded) = shared.decoded_physical_channel_if_ready(ch.index) {
                    channel_data.insert(
                        ch.name.clone(),
                        ChannelData {
                            samples: decoded.data.to_vec(),
                            freq: ch.freq,
                        },
                    );
                } else {
                    shared.request_physical_channel_decode(ch.index);
                    waiting_on_inputs = true;
                }
            }
        }
    }

    // Add other evaluated math channels that are referenced
    for other in &shared.math_channels {
        if other.id != exclude_math_id && resolved_refs.iter().any(|r| r == &other.name) {
            if let Some(ref data) = other.data {
                channel_data.insert(
                    other.name.clone(),
                    ChannelData {
                        samples: data.to_vec(),
                        freq: other.freq,
                    },
                );
            } else if matches!(
                other.evaluation_state,
                MathEvaluationState::Queued
                    | MathEvaluationState::WaitingForInputs
                    | MathEvaluationState::Running
            ) {
                waiting_on_inputs = true;
            }
        }
    }

    EvaluationInputs {
        channel_data,
        waiting_on_inputs,
    }
}

pub struct MathEvaluationJobInput {
    pub math_id: u64,
    pub expression: String,
    pub aliases: HashMap<String, String>,
    pub channel_data: HashMap<String, ChannelData>,
}

fn mark_math_channel_status(
    mc: &mut MathChannelDef,
    evaluation_state: MathEvaluationState,
    message: Option<&str>,
) {
    mc.data = None;
    mc.freq = 0;
    mc.error = message.map(str::to_string);
    mc.evaluation_state = evaluation_state;
    mc.cached_min = 0.0;
    mc.cached_max = 0.0;
    mc.cached_avg = 0.0;
}

/// Evaluate all math channels in dependency order (topological sort).
pub fn evaluate_all_math_channels(shared: &mut SharedState) {
    for idx in topological_eval_order(shared) {
        queue_math_channel_evaluation(shared, idx);
    }
    shared.invalidate_derived_caches();
}

/// Compute a topological evaluation order for math channels based on their dependencies.
/// Falls back to original index order for channels involved in cycles.
pub fn topological_eval_order(shared: &SharedState) -> Vec<usize> {
    let n = shared.math_channels.len();
    if n == 0 {
        return Vec::new();
    }

    // Build name→index map for math channels
    let name_to_idx: HashMap<String, usize> = shared
        .math_channels
        .iter()
        .enumerate()
        .map(|(i, mc)| (mc.name.clone(), i))
        .collect();

    // Build adjacency: deps[i] = set of math channel indices that channel i depends on
    let mut deps: Vec<Vec<usize>> = Vec::with_capacity(n);
    for mc in &shared.math_channels {
        let refs = match i3rs_core::parse_expression(&mc.expression) {
            Ok(expr) => i3rs_core::referenced_channels(&expr),
            Err(_) => Vec::new(),
        };
        let dep_indices: Vec<usize> = refs
            .iter()
            .filter_map(|r| name_to_idx.get(r).copied())
            .collect();
        deps.push(dep_indices);
    }

    // Kahn's algorithm: in_degree[i] = number of dependencies channel i has.
    // Channels with 0 dependencies can be evaluated first.
    let mut in_degree = vec![0usize; n];
    for (i, dep_list) in deps.iter().enumerate() {
        in_degree[i] = dep_list.len();
    }

    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(idx) = queue.pop_front() {
        order.push(idx);
        // For every channel that depends on idx, decrease in_degree
        for (i, dep_list) in deps.iter().enumerate() {
            if dep_list.contains(&idx) {
                in_degree[i] -= 1;
                if in_degree[i] == 0 {
                    queue.push_back(i);
                }
            }
        }
    }

    // Any remaining channels (cycles) — append in original order
    if order.len() < n {
        for i in 0..n {
            if !order.contains(&i) {
                order.push(i);
            }
        }
    }

    order
}

pub fn queue_math_channel_evaluation(shared: &mut SharedState, idx: usize) {
    let Some(mc) = shared.math_channels.get_mut(idx) else {
        return;
    };
    let math_id = mc.id;
    mark_math_channel_status(
        mc,
        MathEvaluationState::Queued,
        Some("Queued for evaluation..."),
    );
    shared.request_math_channel_evaluation_by_id(math_id);
}

pub fn build_math_evaluation_job(
    shared: &mut SharedState,
    math_id: u64,
) -> Option<MathEvaluationJobInput> {
    let idx = shared.math_channel_index_by_id(math_id)?;
    let expression = shared.math_channels.get(idx)?.expression.clone();
    let inputs = build_channel_data_map(shared, &expression, math_id);
    if inputs.waiting_on_inputs {
        if let Some(mc) = shared.math_channels.get_mut(idx) {
            mark_math_channel_status(
                mc,
                MathEvaluationState::WaitingForInputs,
                Some("Waiting for source channels..."),
            );
        }
        return None;
    }

    if let Some(mc) = shared.math_channels.get_mut(idx) {
        mark_math_channel_status(mc, MathEvaluationState::Running, Some("Evaluating..."));
    }

    Some(MathEvaluationJobInput {
        math_id,
        expression,
        aliases: shared.channel_aliases.clone(),
        channel_data: inputs.channel_data,
    })
}

pub fn apply_math_evaluation_result(
    shared: &mut SharedState,
    math_id: u64,
    expression: &str,
    result: Result<EvaluatedMathChannel, String>,
) -> bool {
    let Some(idx) = shared.math_channel_index_by_id(math_id) else {
        return false;
    };
    let Some(mc) = shared.math_channels.get_mut(idx) else {
        return false;
    };
    if mc.expression != expression {
        return false;
    }

    match result {
        Ok(evaluated) => {
            mc.freq = evaluated.freq;
            mc.data = Some(Arc::from(evaluated.samples));
            mc.error = None;
            mc.evaluation_state = MathEvaluationState::Ready;
            mc.cached_min = evaluated.stats.min;
            mc.cached_max = evaluated.stats.max;
            mc.cached_avg = evaluated.stats.avg;
        }
        Err(err) => {
            mc.data = None;
            mc.freq = 0;
            mc.error = Some(err);
            mc.evaluation_state = MathEvaluationState::Error;
            mc.cached_min = 0.0;
            mc.cached_max = 0.0;
            mc.cached_avg = 0.0;
        }
    }

    true
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
        && let Some(path) = crate::platform::save_math_channels_file()
        && let Err(e) = std::fs::write(&path, json)
    {
        eprintln!("Failed to save math channels: {}", e);
    }
}

/// Load math channels from a JSON file and evaluate them.
pub fn load_math_channels(shared: &mut SharedState) {
    if let Some(path) = crate::platform::pick_math_channels_file() {
        match std::fs::read_to_string(&path) {
            Ok(json) => {
                match serde_json::from_str::<Vec<crate::workspace::MathChannelConfig>>(&json) {
                    Ok(configs) => {
                        for config in configs {
                            let mc = shared.create_math_channel_def(
                                config.name,
                                config.expression,
                                config.unit,
                                config.dec_places,
                            );
                            shared.math_channels.push(mc);
                        }
                        evaluate_all_math_channels(shared);
                        shared.invalidate_derived_caches();
                    }
                    Err(e) => eprintln!("Failed to parse math channels: {}", e),
                }
            }
            Err(e) => eprintln!("Failed to read file: {}", e),
        }
    }
}

/// Check if a channel name conflicts with existing physical or math channels.
/// `exclude_math_idx` allows excluding a specific math channel (for edits).
fn is_duplicate_name(name: &str, shared: &SharedState, exclude_math_idx: Option<usize>) -> bool {
    if name.is_empty() {
        return false;
    }
    // Check physical channels
    if let Some(ld) = &shared.ld_file
        && ld
            .channels
            .iter()
            .any(|ch| ch.name.eq_ignore_ascii_case(name))
    {
        return true;
    }
    // Check other math channels
    for (i, mc) in shared.math_channels.iter().enumerate() {
        if Some(i) == exclude_math_idx {
            continue;
        }
        if mc.name.eq_ignore_ascii_case(name) {
            return true;
        }
    }
    false
}
struct EvaluationInputs {
    channel_data: HashMap<String, ChannelData>,
    waiting_on_inputs: bool,
}

#[cfg(test)]
mod tests {
    use super::{build_math_evaluation_job, queue_math_channel_evaluation};
    use crate::state::SharedState;

    #[test]
    fn math_jobs_follow_channel_ids_after_list_reordering() {
        let mut shared = SharedState::new();
        let first = shared.create_math_channel_def("A".into(), "1".into(), String::new(), 2);
        let second = shared.create_math_channel_def("B".into(), "2".into(), String::new(), 2);
        let second_id = second.id;
        shared.math_channels.push(first);
        shared.math_channels.push(second);

        queue_math_channel_evaluation(&mut shared, 1);
        assert_eq!(
            shared.take_requested_math_channel_evaluations(),
            vec![second_id]
        );

        shared.math_channels.remove(0);

        let job = build_math_evaluation_job(&mut shared, second_id)
            .expect("remaining math channel should still be addressable by id");
        assert_eq!(job.math_id, second_id);
        assert_eq!(shared.math_channels[0].name, "B");
    }
}

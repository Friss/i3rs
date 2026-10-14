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
    quantity: &'static str,
    display_unit: Option<&'static str>,
    category: &'static str,
    description: &'static str,
}

const PREDEFINED_CALCS: &[PredefinedCalc] = &[
    // Wheel slip
    PredefinedCalc {
        name: "Wheel Slip FL",
        expression: "(WheelSpeed_FL - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        quantity: "",
        display_unit: None,
        category: "Traction",
        description: "Front-left wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip FR",
        expression: "(WheelSpeed_FR - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        quantity: "",
        display_unit: None,
        category: "Traction",
        description: "Front-right wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip RL",
        expression: "(WheelSpeed_RL - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        quantity: "",
        display_unit: None,
        category: "Traction",
        description: "Rear-left wheel slip percentage",
    },
    PredefinedCalc {
        name: "Wheel Slip RR",
        expression: "(WheelSpeed_RR - GPS_Speed) / max(GPS_Speed, 1) * 100",
        unit: "%",
        quantity: "",
        display_unit: None,
        category: "Traction",
        description: "Rear-right wheel slip percentage",
    },
    // Chassis dynamics
    PredefinedCalc {
        name: "Pitch Angle",
        expression: "rad_to_deg(atan2(G_Force_Long, 9.81))",
        unit: "deg",
        quantity: "",
        display_unit: None,
        category: "Chassis",
        description: "Vehicle pitch angle from longitudinal G",
    },
    PredefinedCalc {
        name: "Roll Angle",
        expression: "rad_to_deg(atan2(G_Force_Lat, 9.81))",
        unit: "deg",
        quantity: "",
        display_unit: None,
        category: "Chassis",
        description: "Vehicle roll angle from lateral G",
    },
    PredefinedCalc {
        name: "Total G-Force",
        expression: "sqrt(pow(G_Force_Lat, 2) + pow(G_Force_Long, 2))",
        unit: "G",
        quantity: "",
        display_unit: None,
        category: "Chassis",
        description: "Combined lateral and longitudinal G-force",
    },
    // Steering / oversteer
    PredefinedCalc {
        name: "Oversteer",
        expression: "smooth(choose('Corr Speed'[km/h] < 50, 0, sgn('G Force Lat'[m/s/s]) * (('Vehicle Wheelbase'[m] * 'G Force Lat'[m/s/s] / sqr('Corr Speed'[m/s])) - sgn(stat_mean('Steered Angle'[rad] * 'G Force Lat'[m/s/s])) * 'Steered Angle'[rad])), 0.2)",
        unit: "rad",
        quantity: "Angle",
        display_unit: Some("deg"),
        category: "Handling",
        description: "User-supplied MoTeC Oversteer definition. Requires corrected speed, lateral acceleration, steered angle and a Vehicle Wheelbase setup constant. Result: radians; 0.2 s smoothing.",
    },
    PredefinedCalc {
        name: "Steering Rate",
        expression: "derivative(Steering_Angle)",
        unit: "deg/s",
        quantity: "",
        display_unit: None,
        category: "Handling",
        description: "Rate of steering wheel angle change",
    },
    // Engine
    PredefinedCalc {
        name: "Throttle Smoothed",
        expression: "smooth(Throttle_Pos, 10)",
        unit: "%",
        quantity: "",
        display_unit: None,
        category: "Engine",
        description: "Smoothed throttle position (10-sample window)",
    },
    PredefinedCalc {
        name: "Speed Delta",
        expression: "derivative(GPS_Speed)",
        unit: "km/h/s",
        quantity: "",
        display_unit: None,
        category: "Performance",
        description: "Rate of speed change (acceleration/deceleration)",
    },
    PredefinedCalc {
        name: "Distance",
        expression: "integrate(GPS_Speed / 3.6)",
        unit: "m",
        quantity: "",
        display_unit: None,
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
    pub new_options: i3rs_core::channels::MathOptions,
    pub new_precision: i16,
    pub enum_drafts: HashMap<u64, String>,
    pub new_enums: String,
    pub setup: super::channel_setup::SetupEditor,
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
            new_options: Default::default(),
            new_precision: 2,
            enum_drafts: HashMap::new(),
            new_enums: String::new(),
            setup: super::channel_setup::SetupEditor {
                filter_window: 0.2,
                scale_factor: 1.0,
                ..Default::default()
            },
        }
    }
}

/// Show the math editor UI.
pub fn show(ui: &mut egui::Ui, shared: &mut SharedState, editor: &mut MathEditorState) {
    ui.heading("Math Channels");
    ui.separator();

    let mut definitions_changed = super::channel_setup::show(ui, shared, &mut editor.setup);
    let recording = shared.recording_key().to_string();
    let sources = shared.channel_settings.sources.clone();
    ui.collapsing("Add filter calculation", |ui| {
        ui.label("Channel name:");
        ui.text_edit_singleline(&mut editor.setup.filter_channel);
        egui::ComboBox::from_id_salt("filter_kind")
            .selected_text(
                ["Sample smoothing", "Time smoothing", "Low-pass Hz"][editor.setup.filter_kind],
            )
            .show_ui(ui, |ui| {
                for (idx, name) in ["Sample smoothing", "Time smoothing", "Low-pass Hz"]
                    .into_iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut editor.setup.filter_kind, idx, name);
                }
            });
        ui.add(
            egui::DragValue::new(&mut editor.setup.filter_window)
                .range(0.001..=10000.0)
                .speed(0.1),
        );
        if editor.setup.filter_kind != 2 {
            ui.checkbox(&mut editor.setup.filter_centered, "Centered");
        }
        if ui
            .add_enabled(
                !editor.setup.filter_channel.is_empty(),
                egui::Button::new("Use filter in new channel form"),
            )
            .clicked()
        {
            editor.new_name = format!("{} Filtered", editor.setup.filter_channel);
            let channel = format!("\"{}\"", editor.setup.filter_channel);
            let function = ["smooth_samples", "smooth_time", "lowpass"][editor.setup.filter_kind];
            let centered = if editor.setup.filter_kind == 2 {
                String::new()
            } else {
                format!(", {}", u8::from(editor.setup.filter_centered))
            };
            editor.new_expression = format!(
                "{function}({channel}, {}{centered})",
                editor.setup.filter_window
            );
            if let i3rs_core::channels::ChannelResolution::Resolved { key, .. } =
                shared.resolve_analysis_reference(&editor.setup.filter_channel)
            {
                editor.new_unit = match key {
                    i3rs_core::channels::ChannelKey::Generated(name) => {
                        if name.contains("Speed") { "m/s" } else { "m" }.into()
                    }
                    i3rs_core::channels::ChannelKey::Physical(idx) => {
                        super::utils::resolve_channel_meta(
                            crate::state::ChannelId::Physical(idx),
                            shared,
                        )
                        .1
                    }
                    i3rs_core::channels::ChannelKey::Derived(id) => shared.math_channels
                        [shared.math_channel_index_by_id(id).unwrap()]
                    .unit
                    .clone(),
                    i3rs_core::channels::ChannelKey::Constant(idx) => {
                        shared.channel_settings.constants[idx].unit.clone()
                    }
                };
            }
        }
    });

    ui.collapsing("Add scale/offset calculation", |ui| {
        ui.label("Input channel:");
        ui.text_edit_singleline(&mut editor.setup.scale_channel);
        ui.horizontal_wrapped(|ui| {
            ui.label("Scale:");
            ui.add(egui::DragValue::new(&mut editor.setup.scale_factor).speed(0.01));
            ui.label("Offset:");
            ui.add(egui::DragValue::new(&mut editor.setup.scale_offset).speed(0.1));
            ui.checkbox(
                &mut editor.setup.scale_this_recording,
                "This recording only",
            );
        });
        if ui
            .add_enabled(
                !editor.setup.scale_channel.is_empty()
                    && editor.setup.scale_factor.is_finite()
                    && editor.setup.scale_offset.is_finite(),
                egui::Button::new("Use scale/offset in new channel form"),
            )
            .clicked()
        {
            editor.new_name = format!("{} Scaled", editor.setup.scale_channel);
            editor.new_expression = format!(
                "\"{}\" * {} + {}",
                editor.setup.scale_channel, editor.setup.scale_factor, editor.setup.scale_offset
            );
            editor.new_options.recording =
                editor.setup.scale_this_recording.then(|| recording.clone());
        }
    });

    // New channel form
    ui.group(|ui| {
        ui.label("New Math Channel");
        ui.horizontal_wrapped(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut editor.new_name);
        });
        ui.label("Expression:");
        ui.add(
            egui::TextEdit::multiline(&mut editor.new_expression)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .code_editor(),
        );
        super::channel_setup::options(
            ui,
            &mut editor.new_options,
            &mut editor.new_unit,
            &mut editor.new_precision,
            &recording,
            &sources,
            &mut editor.new_enums,
        );
        let name_conflict = shared.math_channels.iter().any(|mc| {
            mc.name == editor.new_name
                && mc.options.source == editor.new_options.source
                && (mc.options.recording == editor.new_options.recording
                    || mc.options.recording.is_none()
                    || editor.new_options.recording.is_none())
        });
        if name_conflict {
            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 100),
                format!("⚠ A channel named \"{}\" already exists", editor.new_name),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !editor.new_name.is_empty()
                        && !editor.new_expression.is_empty()
                        && !name_conflict
                        && super::channel_setup::parse_enums(&editor.new_enums).is_ok(),
                    egui::Button::new("Add Channel"),
                )
                .clicked()
            {
                let mut math_channel = shared.create_math_channel_def(
                    editor.new_name.clone(),
                    editor.new_expression.clone(),
                    editor.new_unit.clone(),
                    editor.new_precision,
                );
                math_channel.options = editor.new_options.clone();
                shared.math_channels.push(math_channel);
                definitions_changed = true;
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
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text(calc.description)
                        .clicked()
                    {
                        editor.new_name = calc.name.to_string();
                        editor.new_expression = calc.expression.to_string();
                        editor.new_unit = calc.unit.to_string();
                        editor.new_precision = 2;
                        editor.new_options = i3rs_core::channels::MathOptions {
                            quantity: calc.quantity.into(),
                            display_unit: calc.display_unit.map(String::from),
                            comment: calc.description.into(),
                            ..Default::default()
                        };
                        editor.new_enums.clear();
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
    }

    let mut to_remove: Option<usize> = None;
    let mut to_reevaluate: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("math_channels_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, mc) in shared.math_channels.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        definitions_changed |= ui.text_edit_singleline(&mut mc.name).changed();
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
                        definitions_changed = true;
                    }
                    ui.push_id(mc.id, |ui| {
                        let draft = editor.enum_drafts.entry(mc.id).or_insert_with(|| {
                            super::channel_setup::enum_text(&mc.options.enum_labels)
                        });
                        definitions_changed |= super::channel_setup::options(
                            ui,
                            &mut mc.options,
                            &mut mc.unit,
                            &mut mc.dec_places,
                            &recording,
                            &sources,
                            draft,
                        );
                    });

                    ui.horizontal_wrapped(|ui| {
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

    if to_reevaluate.is_some() {
        definitions_changed = true;
    }

    if let Some(idx) = to_remove {
        if let Some(math_id) = shared.math_channels.get(idx).map(|mc| mc.id) {
            shared.cancel_math_channel_evaluation(math_id);
        }
        shared.math_channels.remove(idx);
        definitions_changed = true;
        shared.invalidate_derived_caches();
    }

    // Channel aliases section
    ui.separator();
    ui.horizontal_wrapped(|ui| {
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
            ui.horizontal_wrapped(|ui| {
                ui.label("Alias:");
                ui.add(egui::TextEdit::singleline(&mut editor.new_alias_name).desired_width(100.0));
                ui.label("\u{2192}");
                ui.label("Targets (one per line, first available wins):");
                ui.add(
                    egui::TextEdit::multiline(&mut editor.new_alias_target).desired_width(100.0),
                );
                if ui
                    .add_enabled(
                        !editor.new_alias_name.is_empty() && !editor.new_alias_target.is_empty(),
                        egui::Button::new("Add"),
                    )
                    .clicked()
                {
                    let targets = editor
                        .new_alias_target
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect();
                    shared
                        .channel_settings
                        .aliases
                        .insert(editor.new_alias_name.clone(), targets);
                    definitions_changed = true;
                    editor.new_alias_name.clear();
                    editor.new_alias_target.clear();
                }
            });

            let mut remove_ordered_alias = None;
            for (alias, targets) in &shared.channel_settings.aliases {
                ui.horizontal_wrapped(|ui| {
                    ui.monospace(alias);
                    ui.label(targets.join(" → "));
                    if ui.small_button("Delete").clicked() {
                        remove_ordered_alias = Some(alias.clone());
                    }
                    if ui.small_button("Edit").clicked() {
                        editor.new_alias_name = alias.clone();
                        editor.new_alias_target = targets.join("\n");
                    }
                });
            }
            if let Some(alias) = remove_ordered_alias {
                shared.channel_settings.aliases.remove(&alias);
                definitions_changed = true;
            }
            // List existing aliases
            let mut to_remove_alias: Option<String> = None;
            for (alias, target) in &shared.channel_aliases {
                ui.horizontal_wrapped(|ui| {
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
                definitions_changed = true;
            }
        });
    }
    if definitions_changed {
        shared.invalidate_math_results();
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
    let mut input_units = std::collections::BTreeMap::new();
    let mut input_interpolation = std::collections::BTreeMap::new();

    // Parse expression to find which channels are referenced
    let refs: Vec<String> = match i3rs_core::parse_expression(expression) {
        Ok(expr) => i3rs_core::referenced_channels(&expr),
        Err(err) => {
            return EvaluationInputs {
                channel_data,
                input_units,
                input_interpolation,
                waiting_on_inputs: false,
                error: Some(err.to_string()),
            };
        }
    };

    let mut error = None;
    for reference in refs {
        use i3rs_core::channels::{ChannelKey, ChannelResolution};
        match shared.resolve_analysis_reference(&reference) {
            ChannelResolution::Resolved {
                key: ChannelKey::Physical(idx),
                ..
            } => {
                let (_, unit, _, _) = crate::panels::utils::resolve_channel_meta(
                    crate::state::ChannelId::Physical(idx),
                    shared,
                );
                input_units.insert(reference.clone(), unit);
                let ch = &shared.ld_file.as_ref().unwrap().channels[idx];
                let policy = shared
                    .channel_settings
                    .mapping(&ch.name, shared.recording_key())
                    .map(|m| m.interpolation)
                    .unwrap_or_else(|| {
                        if super::utils::uses_discrete_values(&ch.name, &ch.enum_labels) {
                            i3rs_core::channels::Interpolation::Previous
                        } else {
                            i3rs_core::channels::Interpolation::Linear
                        }
                    });
                input_interpolation.insert(reference.clone(), policy);
                if let Some(message) = shared.channel_errors.borrow().get(&idx) {
                    error = Some(message.clone());
                    break;
                }
                if let Some(decoded) = shared.decoded_physical_channel_if_ready(idx) {
                    let freq = shared.ld_file.as_ref().unwrap().channels[idx].freq;
                    channel_data.insert(
                        reference,
                        ChannelData {
                            samples: decoded.data.to_vec(),
                            freq,
                        },
                    );
                } else {
                    shared.request_physical_channel_decode(idx);
                    waiting_on_inputs = true;
                }
            }
            ChannelResolution::Resolved {
                key: ChannelKey::Derived(id),
                ..
            } => {
                if id == exclude_math_id
                    || dependency_reaches(
                        shared,
                        id,
                        exclude_math_id,
                        &mut std::collections::HashSet::new(),
                    )
                {
                    error = Some(format!("cyclic math dependency through '{reference}'"));
                    break;
                }
                let other = &shared.math_channels[shared.math_channel_index_by_id(id).unwrap()];
                input_units.insert(reference.clone(), other.unit.clone());
                input_interpolation.insert(
                    reference.clone(),
                    if !other.options.enum_labels.is_empty()
                        && other.options.interpolation == i3rs_core::channels::Interpolation::Linear
                    {
                        i3rs_core::channels::Interpolation::Previous
                    } else {
                        other.options.interpolation
                    },
                );
                let expected_key = shared.math_evaluation_key(id);
                if other.evaluation_key.as_deref() != Some(expected_key.as_str()) {
                    shared.request_math_channel_evaluation_by_id(id);
                    waiting_on_inputs = true;
                } else if let Some(data) = &other.data {
                    channel_data.insert(
                        reference,
                        ChannelData {
                            samples: data.to_vec(),
                            freq: other.freq,
                        },
                    );
                } else if other.evaluation_state == MathEvaluationState::Error {
                    error = Some(format!(
                        "source '{}' failed: {}",
                        other.name,
                        other.error.as_deref().unwrap_or("unavailable")
                    ));
                    break;
                } else {
                    waiting_on_inputs = true;
                }
            }
            ChannelResolution::Resolved {
                key: ChannelKey::Constant(idx),
                ..
            } => {
                let constant = &shared.channel_settings.constants[idx];
                input_units.insert(reference.clone(), constant.unit.clone());
                if !constant.value.is_finite() {
                    error = Some(format!("constant '{}' must be finite", constant.name));
                    break;
                }
                let len = shared.data_duration.unwrap_or(0.0).ceil() as usize;
                channel_data.insert(
                    reference,
                    ChannelData {
                        samples: vec![constant.value; len],
                        freq: 1,
                    },
                );
            }
            ChannelResolution::Resolved {
                key: ChannelKey::Generated(name),
                ..
            } => {
                input_units.insert(
                    reference.clone(),
                    if name.contains("Speed") { "m/s" } else { "m" }.into(),
                );
                let data = match name.as_str() {
                    "Corr Speed" => shared.correction_speed.as_ref(),
                    "Corr GPS Speed" => shared.correction_gps_speed.as_ref(),
                    _ => shared.correction_distance.as_ref(),
                };
                if let Some((samples, freq)) = data {
                    channel_data.insert(
                        reference,
                        ChannelData {
                            samples: samples.to_vec(),
                            freq: *freq,
                        },
                    );
                } else if let Some(failure) = &shared.correction_error {
                    error = Some(format!("{name}: {failure}"));
                    break;
                } else if shared.corrections_settled() {
                    error = Some(format!("{name}: no usable correction source"));
                    break;
                } else {
                    shared.request_track_data_build();
                    waiting_on_inputs = true;
                }
            }
            ChannelResolution::Unresolved { reason, .. } => {
                error = Some(reason);
                break;
            }
        }
    }
    EvaluationInputs {
        channel_data,
        input_units,
        input_interpolation,
        waiting_on_inputs,
        error,
    }
}

fn dependency_reaches(
    shared: &SharedState,
    id: u64,
    needle: u64,
    visited: &mut std::collections::HashSet<u64>,
) -> bool {
    if id == needle {
        return true;
    }
    if !visited.insert(id) {
        return false;
    }
    let Some(index) = shared.math_channel_index_by_id(id) else {
        return false;
    };
    let Ok(expr) = i3rs_core::parse_expression(&shared.math_channels[index].expression) else {
        return false;
    };
    i3rs_core::referenced_channels(&expr).iter().any(|name| {
        if let i3rs_core::channels::ChannelResolution::Resolved {
            key: i3rs_core::channels::ChannelKey::Derived(dep),
            ..
        } = shared.resolve_analysis_reference(name)
        {
            dependency_reaches(shared, dep, needle, visited)
        } else {
            false
        }
    })
}

pub struct MathEvaluationJobInput {
    pub math_id: u64,
    pub expression: String,
    pub evaluation_key: String,
    pub options: i3rs_core::math_engine::EvaluationOptions,
    pub aliases: HashMap<String, String>,
    pub channel_data: HashMap<String, ChannelData>,
}

fn mark_math_channel_status(
    mc: &mut MathChannelDef,
    evaluation_state: MathEvaluationState,
    message: Option<&str>,
) {
    mc.data = None;
    mc.evaluation_key = None;
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

pub fn reevaluate_math_channels_waiting_on_inputs(shared: &mut SharedState) {
    let waiting_ids: Vec<u64> = shared
        .math_channels
        .iter()
        .filter(|mc| mc.evaluation_state == MathEvaluationState::WaitingForInputs)
        .map(|mc| mc.id)
        .collect();

    for math_id in waiting_ids {
        if let Some(idx) = shared.math_channel_index_by_id(math_id) {
            queue_math_channel_evaluation(shared, idx);
        }
    }
}

/// Compute a topological evaluation order for math channels based on their dependencies.
/// Falls back to original index order for channels involved in cycles.
pub fn topological_eval_order(shared: &SharedState) -> Vec<usize> {
    let n = shared.math_channels.len();
    if n == 0 {
        return Vec::new();
    }

    let deps: Vec<Vec<usize>> = shared
        .math_channels
        .iter()
        .map(|mc| {
            let refs = i3rs_core::parse_expression(&mc.expression)
                .map(|expr| i3rs_core::referenced_channels(&expr))
                .unwrap_or_default();
            let mut indices: Vec<_> = refs
                .iter()
                .filter_map(|r| match shared.resolve_analysis_reference(r) {
                    i3rs_core::channels::ChannelResolution::Resolved {
                        key: i3rs_core::channels::ChannelKey::Derived(id),
                        ..
                    } => shared.math_channel_index_by_id(id),
                    _ => None,
                })
                .collect();
            indices.sort_unstable();
            indices.dedup();
            indices
        })
        .collect();

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
    let evaluation_key = shared.math_evaluation_key(math_id);
    if !shared.channel_enabled(crate::state::ChannelId::Math(idx)) {
        let message = "math source is disabled or scoped to another recording";
        mark_math_channel_status(
            &mut shared.math_channels[idx],
            MathEvaluationState::Error,
            Some(message),
        );
        shared.math_channels[idx].evaluation_key = Some(evaluation_key);
        shared.message(format!("{}: {message}", shared.math_channels[idx].name));
        return None;
    }
    if let Some(display_unit) = &shared.math_channels[idx].options.display_unit
        && let Err(error) =
            i3rs_core::channels::unit_transform(&shared.math_channels[idx].unit, display_unit)
    {
        mark_math_channel_status(
            &mut shared.math_channels[idx],
            MathEvaluationState::Error,
            Some(&error),
        );
        shared.math_channels[idx].evaluation_key = Some(evaluation_key);
        shared.message(format!("{}: {error}", shared.math_channels[idx].name));
        return None;
    }
    let expression = shared.math_channels.get(idx)?.expression.clone();
    let mut options = i3rs_core::math_engine::EvaluationOptions {
        input_units: Default::default(),
        input_interpolation: Default::default(),
        output_freq: shared.math_channels[idx].options.output_freq,
        interpolation: shared.math_channels[idx].options.interpolation,
        duration: shared.data_duration,
    };
    let inputs = build_channel_data_map(shared, &expression, math_id);
    options.input_units = inputs.input_units;
    options.input_interpolation = inputs.input_interpolation;
    if let Some(error) = inputs.error {
        mark_math_channel_status(
            &mut shared.math_channels[idx],
            MathEvaluationState::Error,
            Some(&error),
        );
        shared.math_channels[idx].evaluation_key = Some(evaluation_key);
        shared.message(format!("{}: {error}", shared.math_channels[idx].name));
        return None;
    }
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
        evaluation_key,
        options,
        aliases: HashMap::new(),
        channel_data: inputs.channel_data,
    })
}

pub fn apply_math_evaluation_result(
    shared: &mut SharedState,
    math_id: u64,
    expression: &str,
    evaluation_key: &str,
    result: Result<EvaluatedMathChannel, String>,
) -> bool {
    let Some(idx) = shared.math_channel_index_by_id(math_id) else {
        return false;
    };
    if shared.math_evaluation_key(math_id) != evaluation_key {
        return false;
    }
    if let Err(error) = &result {
        shared.message(format!("{}: {error}", shared.math_channels[idx].name));
    }
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
            mc.evaluation_key = Some(evaluation_key.into());
            mc.error = None;
            mc.evaluation_state = MathEvaluationState::Ready;
            mc.cached_min = evaluated.stats.min;
            mc.cached_max = evaluated.stats.max;
            mc.cached_avg = evaluated.stats.avg;
        }
        Err(err) => {
            mc.evaluation_key = Some(evaluation_key.into());
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
            options: mc.options.clone(),
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
        shared.message(format!("Failed to save math channels: {}", e));
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
                            let mc = config.create_definition(shared);
                            shared.math_channels.push(mc);
                        }
                        evaluate_all_math_channels(shared);
                        shared.invalidate_derived_caches();
                    }
                    Err(e) => shared.message(format!("Failed to parse math channels: {}", e)),
                }
            }
            Err(e) => shared.message(format!("Failed to read file: {}", e)),
        }
    }
}

struct EvaluationInputs {
    input_interpolation: std::collections::BTreeMap<String, i3rs_core::channels::Interpolation>,
    input_units: std::collections::BTreeMap<String, String>,
    channel_data: HashMap<String, ChannelData>,
    waiting_on_inputs: bool,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        build_math_evaluation_job, queue_math_channel_evaluation,
        reevaluate_math_channels_waiting_on_inputs,
    };
    use crate::state::{MathEvaluationState, SharedState};

    #[test]
    fn oversteer_template_matches_supplied_result_and_display_properties() {
        let calc = super::PREDEFINED_CALCS
            .iter()
            .find(|calc| calc.name == "Oversteer")
            .unwrap();
        assert_eq!(calc.unit, "rad");
        assert_eq!(calc.quantity, "Angle");
        assert_eq!(calc.display_unit, Some("deg"));
        assert_eq!(
            calc.expression,
            "smooth(choose('Corr Speed'[km/h] < 50, 0, sgn('G Force Lat'[m/s/s]) * (('Vehicle Wheelbase'[m] * 'G Force Lat'[m/s/s] / sqr('Corr Speed'[m/s])) - sgn(stat_mean('Steered Angle'[rad] * 'G Force Lat'[m/s/s])) * 'Steered Angle'[rad])), 0.2)"
        );
    }

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

    #[test]
    fn reevaluate_only_requeues_math_channels_waiting_on_inputs() {
        let mut shared = SharedState::new();
        let waiting =
            shared.create_math_channel_def("Waiting".into(), "A".into(), String::new(), 2);
        let waiting_id = waiting.id;
        let mut ready =
            shared.create_math_channel_def("Ready".into(), "1".into(), String::new(), 2);
        ready.evaluation_state = MathEvaluationState::Ready;
        shared.math_channels.push(waiting);
        shared.math_channels.push(ready);
        shared.math_channels[0].evaluation_state = MathEvaluationState::WaitingForInputs;
        shared.math_channels[0].error = Some("Waiting for source channels...".into());

        reevaluate_math_channels_waiting_on_inputs(&mut shared);

        assert_eq!(
            shared.take_requested_math_channel_evaluations(),
            vec![waiting_id]
        );
        assert_eq!(
            shared.math_channels[0].evaluation_state,
            MathEvaluationState::Queued
        );
        assert_eq!(
            shared.math_channels[1].evaluation_state,
            MathEvaluationState::Ready
        );
    }
}

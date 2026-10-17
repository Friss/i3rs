//! Editors for source policies, setup constants and portable channel metadata.
use crate::state::SharedState;
use eframe::egui;
use i3rs_core::channels::{
    ChannelMapping, Interpolation, MathOptions, SetupConstant, SourcePolicy,
};

#[derive(Default)]
pub struct SetupEditor {
    pub constant_name: String,
    pub constant_value: f64,
    pub constant_unit: String,
    pub constant_comment: String,
    pub mapping: ChannelMapping,
    pub mapping_this_recording: bool,
    pub mapping_enums: String,
    pub filter_channel: String,
    pub filter_kind: usize,
    pub filter_window: f64,
    pub filter_centered: bool,
    pub scale_channel: String,
    pub scale_factor: f64,
    pub scale_offset: f64,
    pub scale_this_recording: bool,
}

pub fn interpolation_control(
    ui: &mut egui::Ui,
    value: &mut Interpolation,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let before = *value;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for policy in [
                Interpolation::Linear,
                Interpolation::Previous,
                Interpolation::Nearest,
            ] {
                ui.selectable_value(value, policy, format!("{policy:?}"));
            }
        });
    before != *value
}

pub fn parse_enums(text: &str) -> Result<std::collections::BTreeMap<i64, String>, String> {
    let mut labels = std::collections::BTreeMap::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let (key, label) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: use value=label", index + 1))?;
        let key: i64 = key
            .trim()
            .parse()
            .map_err(|_| format!("line {}: invalid integer", index + 1))?;
        if labels.insert(key, label.trim().into()).is_some() {
            return Err(format!("duplicate enum value {key}"));
        }
    }
    Ok(labels)
}

pub fn enum_text(labels: &std::collections::BTreeMap<i64, String>) -> String {
    labels
        .iter()
        .map(|(v, l)| format!("{v}={l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn options(
    ui: &mut egui::Ui,
    opts: &mut MathOptions,
    unit: &mut String,
    precision: &mut i16,
    recording: &str,
    sources: &[SourcePolicy],
    enums: &mut String,
) -> bool {
    let before = (opts.clone(), unit.clone(), *precision);
    ui.horizontal_wrapped(|ui| {
        ui.label("Source:");
        egui::ComboBox::from_id_salt("source")
            .selected_text(&opts.source)
            .show_ui(ui, |ui| {
                for source in sources.iter().filter(|s| s.name != "Logged") {
                    ui.selectable_value(&mut opts.source, source.name.clone(), &source.name);
                }
            });
        if opts.source == "Log File" && opts.recording.is_none() && recording != "no-recording" {
            opts.recording = Some(recording.into());
        }
        let mut scoped = opts.recording.is_some();
        if ui
            .add_enabled(
                recording != "no-recording" && opts.source != "Log File",
                egui::Checkbox::new(&mut scoped, "This recording only"),
            )
            .changed()
        {
            opts.recording = scoped.then(|| recording.into());
        }
    });
    if let Some(scope) = &opts.recording
        && scope != recording
    {
        ui.weak("Definition belongs to another recording.");
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("Quantity:");
        ui.add(egui::TextEdit::singleline(&mut opts.quantity).desired_width(100.0));
        ui.label("Result unit:");
        ui.add(egui::TextEdit::singleline(unit).desired_width(60.0));
        ui.label("Decimals:");
        ui.add(egui::DragValue::new(precision).range(0..=12));
    });
    ui.horizontal_wrapped(|ui| {
        let mut override_display = opts.display_unit.is_some();
        if ui
            .checkbox(&mut override_display, "Default display unit")
            .changed()
        {
            opts.display_unit = override_display.then(|| unit.clone());
        }
        if let Some(display_unit) = &mut opts.display_unit {
            ui.add(egui::TextEdit::singleline(display_unit).desired_width(80.0));
            if let Err(error) = i3rs_core::channels::unit_transform(unit, display_unit) {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        let mut fixed = opts.output_freq.is_some();
        if ui.checkbox(&mut fixed, "Fixed output rate").changed() {
            opts.output_freq = fixed.then_some(100.0);
        }
        if let Some(freq) = &mut opts.output_freq {
            ui.add(egui::DragValue::new(freq).range(1..=2000).suffix(" Hz"));
        } else {
            ui.weak("Highest input rate");
        }
        ui.label("Interpolation:");
        interpolation_control(ui, &mut opts.interpolation, "interpolation");
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Comment:");
        ui.text_edit_singleline(&mut opts.comment);
    });
    ui.collapsing("Enumeration (value=label, one per line)", |ui| {
        let changed = ui
            .add(egui::TextEdit::multiline(enums).desired_rows(3))
            .changed();
        match parse_enums(enums) {
            Ok(labels) => {
                if changed {
                    opts.enum_labels = labels;
                }
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        }
        ui.weak("Enumeration metadata is included in maths JSON import/export.");
    });
    before != (opts.clone(), unit.clone(), *precision)
}

pub fn show(ui: &mut egui::Ui, shared: &mut SharedState, editor: &mut SetupEditor) -> bool {
    let mut changed = false;
    ui.collapsing("Maths sources and precedence", |ui| {
        ui.weak(
            "Lower priority numbers win. Expressions can qualify a source: 'Logged::Engine Speed'.",
        );
        for source in &mut shared.channel_settings.sources {
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut source.enabled, &source.name).changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut source.priority).prefix("priority "))
                    .changed();
            });
        }
    });
    let recording = shared.recording_key().to_string();
    ui.collapsing("Recording setup constants", |ui| {
        ui.weak(
            "Values and units stay with this recording, including after it is renamed or moved.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut editor.constant_name);
            ui.add(egui::DragValue::new(&mut editor.constant_value).speed(0.1));
            ui.label("Unit:");
            ui.text_edit_singleline(&mut editor.constant_unit);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Comment:");
            ui.text_edit_singleline(&mut editor.constant_comment);
        });
        if ui
            .add_enabled(
                recording != "no-recording"
                    && !editor.constant_name.trim().is_empty()
                    && editor.constant_value.is_finite(),
                egui::Button::new("Save constant for this recording"),
            )
            .clicked()
        {
            let constant = SetupConstant {
                name: editor.constant_name.trim().into(),
                value: editor.constant_value,
                unit: editor.constant_unit.clone(),
                recording: recording.clone(),
                comment: editor.constant_comment.clone(),
            };
            if let Some(existing) = shared
                .channel_settings
                .constants
                .iter_mut()
                .find(|c| c.recording == recording && c.name == constant.name)
            {
                *existing = constant;
            } else {
                shared.channel_settings.constants.push(constant);
            }
            changed = true;
        }
        let mut remove = None;
        for (idx, constant) in shared
            .channel_settings
            .constants
            .iter_mut()
            .enumerate()
            .filter(|(_, c)| c.recording == recording)
        {
            ui.push_id(idx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&constant.name);
                    let mut value = constant.value;
                    if ui
                        .add(egui::DragValue::new(&mut value).speed(0.1))
                        .changed()
                    {
                        if value.is_finite() {
                            constant.value = value;
                            changed = true;
                        } else {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                "Constant values must be finite",
                            );
                        }
                    }
                    changed |= ui.text_edit_singleline(&mut constant.unit).changed();
                    changed |= ui.text_edit_singleline(&mut constant.comment).changed();
                    if ui.small_button("Delete").clicked() {
                        remove = Some(idx);
                    }
                });
            });
        }
        if let Some(idx) = remove {
            shared.channel_settings.constants.remove(idx);
            changed = true;
        }
    });
    ui.collapsing("Channel mapping, precision and scale/offset", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Logged name:");
            ui.text_edit_singleline(&mut editor.mapping.original);
            ui.label("Display name:");
            ui.text_edit_singleline(&mut editor.mapping.name);
        });
        ui.horizontal_wrapped(|ui| {
            let mut override_unit = editor.mapping.unit.is_some();
            if ui
                .checkbox(&mut override_unit, "Override source unit metadata")
                .changed()
            {
                editor.mapping.unit = override_unit.then(String::new);
            }
            if let Some(unit) = &mut editor.mapping.unit {
                ui.text_edit_singleline(unit);
            }
            let mut override_precision = editor.mapping.precision.is_some();
            if ui
                .checkbox(&mut override_precision, "Override decimals")
                .changed()
            {
                editor.mapping.precision = override_precision.then_some(2);
            }
            if let Some(p) = &mut editor.mapping.precision {
                ui.add(egui::DragValue::new(p).range(0..=12));
            }
            interpolation_control(
                ui,
                &mut editor.mapping.interpolation,
                "mapping_interpolation",
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Display scale:");
            ui.add(egui::DragValue::new(&mut editor.mapping.display_scale).speed(0.01));
            ui.label("Offset:");
            ui.add(egui::DragValue::new(&mut editor.mapping.display_offset).speed(0.1));
            let mut converted = editor.mapping.display_unit.is_some();
            if ui.checkbox(&mut converted, "Display unit").changed() {
                editor.mapping.display_unit = converted.then(String::new);
            }
            if let Some(unit) = &mut editor.mapping.display_unit {
                ui.text_edit_singleline(unit);
            }
            ui.checkbox(&mut editor.mapping_this_recording, "This recording only");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Comment:");
            ui.text_edit_singleline(&mut editor.mapping.comment);
        });
        ui.label("Enum overrides (value=label):");
        ui.text_edit_multiline(&mut editor.mapping_enums);
        let labels = parse_enums(&editor.mapping_enums);
        if let Err(error) = &labels {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if ui
            .add_enabled(
                !editor.mapping.original.is_empty()
                    && labels.is_ok()
                    && editor.mapping.display_scale.is_finite()
                    && editor.mapping.display_offset.is_finite(),
                egui::Button::new("Save mapping"),
            )
            .clicked()
        {
            let mut mapping = editor.mapping.clone();
            mapping.enum_labels = labels.unwrap();
            mapping.recording = editor.mapping_this_recording.then(|| recording.clone());
            if let Some(old) = shared
                .channel_settings
                .mappings
                .iter_mut()
                .find(|m| m.original == mapping.original && m.recording == mapping.recording)
            {
                *old = mapping;
            } else {
                shared.channel_settings.mappings.push(mapping);
            }
            changed = true;
        }
        let mut remove = None;
        for (idx, m) in shared.channel_settings.mappings.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} → {} [{}]",
                    m.original,
                    m.name,
                    if m.recording.is_some() {
                        "Recording"
                    } else {
                        "Workspace"
                    }
                ));
                if ui.small_button("Edit").clicked() {
                    editor.mapping = m.clone();
                    editor.mapping_this_recording = m.recording.is_some();
                    editor.mapping_enums = enum_text(&m.enum_labels);
                }
                if ui.small_button("Delete").clicked() {
                    remove = Some(idx);
                }
            });
        }
        if let Some(idx) = remove {
            shared.channel_settings.mappings.remove(idx);
            changed = true;
        }
    });
    changed
}

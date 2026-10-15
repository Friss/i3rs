//! Recording-aware channel resolution shared by maths, metadata and diagnostics.
use crate::state::{ChannelId, SharedState};
use i3rs_core::channels::{ChannelCandidate, ChannelKey, ChannelReference, ChannelResolution};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug)]
pub struct AnalysisMessage {
    pub session_id: u64,
    pub text: String,
}

#[derive(Clone, Default)]
pub struct AnalysisMessages(std::sync::Arc<std::sync::Mutex<VecDeque<AnalysisMessage>>>);
impl AnalysisMessages {
    pub fn borrow(&self) -> std::sync::MutexGuard<'_, VecDeque<AnalysisMessage>> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    pub fn borrow_mut(&self) -> std::sync::MutexGuard<'_, VecDeque<AnalysisMessage>> {
        self.borrow()
    }
    pub fn record(&self, session_id: u64, text: String) {
        let mut messages = self.borrow_mut();
        if messages
            .back()
            .is_some_and(|m| m.text == text && m.session_id == session_id)
        {
            return;
        }
        messages.push_back(AnalysisMessage { session_id, text });
        while messages.len() > 200 {
            messages.pop_front();
        }
    }
}

impl SharedState {
    pub fn recording_key(&self) -> &str {
        self.ld_file
            .as_ref()
            .map_or("no-recording", |ld| ld.recording_id())
    }

    pub fn message(&self, text: impl Into<String>) {
        self.analysis_messages.record(self.session_id, text.into());
    }

    pub fn channel_catalog(&self) -> Vec<ChannelCandidate> {
        let recording = self.recording_key();
        let mut catalog = Vec::new();
        if let Some(ld) = &self.ld_file {
            for ch in &ld.channels {
                let mapping = self.channel_settings.mapping(&ch.name, recording);
                catalog.push(ChannelCandidate {
                    name: mapping
                        .filter(|m| !m.name.is_empty())
                        .map_or_else(|| ch.name.clone(), |m| m.name.clone()),
                    source: "Logged".into(),
                    recording: None,
                    key: ChannelKey::Physical(ch.index),
                });
                // Keep the logged name usable when a display mapping renames it.
                if mapping.is_some_and(|m| !m.name.is_empty() && m.name != ch.name) {
                    catalog.push(ChannelCandidate {
                        name: ch.name.clone(),
                        source: "Logged".into(),
                        recording: None,
                        key: ChannelKey::Physical(ch.index),
                    });
                }
            }
        }
        let corrections = self.current_corrections();
        for (name, enabled) in [
            ("Corr Speed", corrections.speed_enabled),
            ("Corr Dist", corrections.distance_enabled),
            ("Corr GPS Speed", corrections.gps_enabled),
        ] {
            if enabled {
                catalog.push(ChannelCandidate {
                    name: name.into(),
                    source: "MoTeC".into(),
                    recording: Some(recording.into()),
                    key: ChannelKey::Generated(name.into()),
                });
            }
        }
        for mc in &self.math_channels {
            catalog.push(ChannelCandidate {
                name: mc.name.clone(),
                source: mc.options.source.clone(),
                recording: mc.options.recording.clone(),
                key: ChannelKey::Derived(mc.id),
            });
        }
        for (idx, c) in self.channel_settings.constants.iter().enumerate() {
            catalog.push(ChannelCandidate {
                name: c.name.clone(),
                source: "Log File".into(),
                recording: Some(c.recording.clone()),
                key: ChannelKey::Constant(idx),
            });
        }
        catalog
    }

    pub fn resolve_analysis_reference(&self, reference: &str) -> ChannelResolution {
        let mut settings = self.channel_settings.clone();
        for (alias, target) in &self.channel_aliases {
            settings
                .aliases
                .entry(alias.clone())
                .or_insert_with(|| vec![target.clone()]);
        }
        let (source, name) = reference
            .split_once("::")
            .map_or((None, reference), |(s, n)| (Some(s.to_string()), n));
        settings.resolve(
            &ChannelReference {
                name: name.into(),
                source,
                recording: None,
            },
            self.recording_key(),
            &self.channel_catalog(),
        )
    }

    pub fn resolved_math_index(&self, reference: &str) -> Option<usize> {
        let mut settings = self.channel_settings.clone();
        for (alias, target) in &self.channel_aliases {
            settings
                .aliases
                .entry(alias.clone())
                .or_insert_with(|| vec![target.clone()]);
        }
        let (source, name) = reference
            .split_once("::")
            .map_or((None, reference), |(s, n)| (Some(s.to_string()), n));
        let catalog: Vec<_> = self
            .channel_catalog()
            .into_iter()
            .filter(|c| matches!(c.key, ChannelKey::Derived(_)))
            .collect();
        match settings.resolve(
            &ChannelReference {
                name: name.into(),
                source,
                recording: None,
            },
            self.recording_key(),
            &catalog,
        ) {
            ChannelResolution::Resolved {
                key: ChannelKey::Derived(id),
                ..
            } => self.math_channel_index_by_id(id),
            _ => None,
        }
    }

    pub fn channel_enabled(&self, id: ChannelId) -> bool {
        match id {
            ChannelId::Physical(_) => self.channel_settings.enabled("Logged"),
            ChannelId::Math(idx) => self.math_channels.get(idx).is_some_and(|mc| {
                self.channel_settings.enabled(&mc.options.source)
                    && mc
                        .options
                        .recording
                        .as_deref()
                        .is_none_or(|r| r == self.recording_key())
            }),
        }
    }

    /// Includes recording identity, full definitions and resolution/evaluation settings.
    /// Background results with any stale part of this key are discarded.
    pub fn math_evaluation_key(&self, math_id: u64) -> String {
        let aliases: BTreeMap<_, _> = self.channel_aliases.iter().collect();
        let definitions: Vec<_> = self
            .math_channels
            .iter()
            .map(|mc| {
                (
                    mc.id,
                    &mc.name,
                    &mc.expression,
                    &mc.unit,
                    mc.dec_places,
                    &mc.options,
                )
            })
            .collect();
        serde_json::to_string(&(
            self.recording_key(),
            self.data_duration,
            math_id,
            definitions,
            aliases,
            &self.channel_settings,
            self.correction_key(),
        ))
        .expect("serializable analysis settings")
    }

    pub fn invalidate_math_results(&mut self) {
        // Clear every result before queueing; dependants cannot consume old data.
        for mc in &mut self.math_channels {
            mc.data = None;
            mc.evaluation_key = None;
        }
        crate::panels::math_editor::evaluate_all_math_channels(self);
        self.display_channel_registry.clear();
        self.invalidate_derived_caches();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_jobs::EvaluatedMathChannel;
    use crate::panels::math_editor::{apply_math_evaluation_result, build_math_evaluation_job};
    use crate::state::{MathEvaluationState, compute_channel_stats};
    use i3rs_core::channels::{Interpolation, SetupConstant};
    fn recording() -> SharedState {
        let mut shared = SharedState::new();
        shared.ld_file = Some(std::sync::Arc::new(
            i3rs_core::LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        ));
        shared.data_duration = Some(2.0);
        shared
    }
    fn add_math(shared: &mut SharedState, name: &str, expression: &str) -> u64 {
        let mc = shared.create_math_channel_def(name.into(), expression.into(), "m".into(), 3);
        let id = mc.id;
        shared.math_channels.push(mc);
        id
    }
    fn evaluate(shared: &mut SharedState, id: u64) -> Vec<f64> {
        let job = build_math_evaluation_job(shared, id).expect("job should be ready");
        let (samples, freq) = i3rs_core::math_engine::evaluate_expression_with_options(
            &job.expression,
            &job.channel_data,
            &job.aliases,
            &job.options,
        )
        .unwrap();
        let result = EvaluatedMathChannel {
            stats: compute_channel_stats(&samples),
            samples: samples.clone(),
            freq,
        };
        assert!(apply_math_evaluation_result(
            shared,
            id,
            &job.expression,
            &job.evaluation_key,
            Ok(result)
        ));
        samples
    }
    #[test]
    fn constants_remain_recording_scoped_and_keys_include_units_rates_and_definitions() {
        let mut shared = recording();
        let recording = shared.recording_key().to_string();
        shared.channel_settings.constants.push(SetupConstant {
            name: "Wheelbase".into(),
            value: 2.7,
            unit: "m".into(),
            recording,
            comment: "Measured".into(),
        });
        let id = add_math(&mut shared, "Wheelbase copy", "'Wheelbase'[m]");
        shared.math_channels[0].options.output_freq = Some(4);
        assert_eq!(evaluate(&mut shared, id), vec![2.7; 8]);
        let key = shared.math_evaluation_key(id);
        shared.math_channels[0].options.interpolation = Interpolation::Previous;
        assert_ne!(shared.math_evaluation_key(id), key);
        let job = build_math_evaluation_job(&mut shared, id).unwrap();
        shared.channel_settings.constants[0].value = 3.1;
        assert!(!apply_math_evaluation_result(
            &mut shared,
            id,
            &job.expression,
            &job.evaluation_key,
            Ok(EvaluatedMathChannel {
                samples: vec![2.7; 8],
                freq: 4,
                stats: Default::default()
            })
        ));
        // Another recording cannot resolve the first recording's setup value.
        let mut other = SharedState::new();
        let mut bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        let uploaded = i3rs_core::LdFile::from_bytes(bytes.clone()).unwrap();
        assert_eq!(uploaded.recording_id(), shared.recording_key());
        // Retain a valid fixture while changing its content identity.
        bytes.push(0);
        other.ld_file = Some(std::sync::Arc::new(
            i3rs_core::LdFile::from_bytes(bytes).unwrap(),
        ));
        assert_ne!(other.recording_key(), shared.recording_key());
        other.data_duration = Some(2.0);
        other.channel_settings = shared.channel_settings.clone();
        let other_id = add_math(&mut other, "Copy", "Wheelbase");
        assert!(build_math_evaluation_job(&mut other, other_id).is_none());
        assert_eq!(
            other.math_channels[0].evaluation_state,
            MathEvaluationState::Error
        );
        assert!(
            other.math_channels[0]
                .error
                .as_ref()
                .unwrap()
                .contains("Wheelbase")
        );
    }
    #[test]
    fn normalized_alias_loading_and_cycles_use_the_same_resolution_contract() {
        let mut shared = recording();
        shared.channel_settings.aliases.insert(
            "Velocity".into(),
            vec!["Missing".into(), "vehicle_speed".into()],
        );
        let id = add_math(&mut shared, "Velocity copy", "Velocity * 2");
        assert!(build_math_evaluation_job(&mut shared, id).is_none());
        let indices = shared.take_requested_physical_channel_decodes();
        assert_eq!(indices.len(), 1);
        let idx = indices[0];
        assert_eq!(
            shared.ld_file.as_ref().unwrap().channels[idx].name,
            "Vehicle Speed"
        );
        shared.store_decoded_physical_channel(idx, vec![10.0, 20.0], Default::default());
        assert!(build_math_evaluation_job(&mut shared, id).is_some());
        let a = add_math(&mut shared, "A", "B + B");
        let b = add_math(&mut shared, "B", "A");
        for id in [a, b] {
            assert!(build_math_evaluation_job(&mut shared, id).is_none());
        }
        assert!(
            shared.math_channels[1]
                .error
                .as_ref()
                .unwrap()
                .contains("cyclic")
        );
    }
    #[test]
    fn disabled_sources_and_decode_failures_are_diagnostic_without_retry_loops() {
        let mut shared = recording();
        let id = add_math(&mut shared, "Calc", "Vehicle_Speed");
        let idx = shared
            .ld_file
            .as_ref()
            .unwrap()
            .find_channel_by_name("Vehicle Speed")
            .unwrap()
            .index;
        shared
            .channel_errors
            .borrow_mut()
            .insert(idx, "test decode failure".into());
        assert!(build_math_evaluation_job(&mut shared, id).is_none());
        shared.request_physical_channel_decode(idx);
        assert!(shared.take_requested_physical_channel_decodes().is_empty());
        assert!(
            shared
                .analysis_messages
                .borrow()
                .back()
                .unwrap()
                .text
                .contains("decode failure")
        );
        shared
            .channel_settings
            .sources
            .iter_mut()
            .find(|s| s.name == "User")
            .unwrap()
            .enabled = false;
        assert!(build_math_evaluation_job(&mut shared, id).is_none());
        assert!(
            shared.math_channels[0]
                .error
                .as_ref()
                .unwrap()
                .contains("disabled")
        );
    }
    #[test]
    fn plotted_math_identity_survives_reordering_and_never_rebinds_after_deletion() {
        let mut shared = recording();
        let a = add_math(&mut shared, "A", "1");
        let b = add_math(&mut shared, "B", "2");
        evaluate(&mut shared, a);
        evaluate(&mut shared, b);
        let mut pc =
            crate::panels::utils::create_plotted_channel(ChannelId::Math(1), &shared, 0).unwrap();
        shared.math_channels.remove(0);
        crate::panels::utils::refresh_plotted_channel(&mut pc, &shared);
        assert_eq!(pc.channel_id, ChannelId::Math(0));
        assert_eq!(pc.data[0], 2.0);
        shared.math_channels.clear();
        add_math(&mut shared, "Other", "99");
        crate::panels::utils::refresh_plotted_channel(&mut pc, &shared);
        assert_eq!(pc.channel_id, ChannelId::Math(usize::MAX));
        assert!(pc.data.is_empty());
    }
    #[test]
    fn display_transforms_do_not_change_math_inputs_and_scope_resets_cleanly() {
        let mut shared = recording();
        let idx = shared
            .ld_file
            .as_ref()
            .unwrap()
            .find_channel_by_name("Fuel Pressure")
            .unwrap()
            .index;
        let freq = shared.ld_file.as_ref().unwrap().channels[idx].freq;
        shared.store_decoded_physical_channel(
            idx,
            vec![100.0; 2 * freq as usize],
            Default::default(),
        );
        shared
            .channel_settings
            .mappings
            .push(i3rs_core::channels::ChannelMapping {
                original: "Fuel Pressure".into(),
                name: "Fuel Test Pressure".into(),
                display_scale: 0.145037738,
                display_unit: Some("psi".into()),
                recording: Some(shared.recording_key().to_string()),
                ..Default::default()
            });
        let mut pc =
            crate::panels::utils::create_plotted_channel(ChannelId::Physical(idx), &shared, 0)
                .unwrap();
        let info = crate::panels::utils::build_plotted_channel_info(&pc, &shared);
        assert_eq!(info.name, "Fuel Test Pressure");
        assert_eq!(info.unit, "psi");
        assert!((info.transform_value(100.0) - 14.5037738).abs() < 1e-10);
        let id = add_math(&mut shared, "Raw Pressure", "'Logged::Fuel Test Pressure'");
        let values = evaluate(&mut shared, id);
        assert_eq!(values[0], 100.0);
        shared.math_channels[0].expression = "'Logged::Fuel Test Pressure'[psi]".into();
        let converted = evaluate(&mut shared, id);
        assert!((converted[0] - 14.503773773).abs() < 1e-6);
        shared.channel_settings.mappings.clear();
        crate::panels::utils::refresh_plotted_channel(&mut pc, &shared);
        assert_eq!(pc.display_scale, 1.0);
        assert!(pc.display_unit.is_none());
    }

    #[test]
    fn math_result_radians_stay_raw_while_values_and_csv_use_default_degrees() {
        use crate::panels::utils::*;
        let mut shared = recording();
        let id = add_math(&mut shared, "Oversteer", "0.1");
        shared.math_channels[0].unit = "rad".into();
        shared.math_channels[0].dec_places = 2;
        shared.math_channels[0].options.quantity = "Angle".into();
        shared.math_channels[0].options.display_unit = Some("deg".into());
        shared.math_channels[0].options.output_freq = Some(2);
        assert_eq!(evaluate(&mut shared, id), vec![0.1; 4]);
        let mut pc = create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap();
        let info = build_plotted_channel_info(&pc, &shared);
        assert_eq!(info.unit, "deg");
        assert!((info.transform_value(0.1) - 0.1_f64.to_degrees()).abs() < 1e-9);
        let path = std::env::temp_dir().join(format!("i3r2-angle-{}.csv", std::process::id()));
        i3rs_core::export_csv(&path, &[info.export_channel()], None).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            csv.starts_with("Time (s),Oversteer (deg)\n0.000000,5.73\n"),
            "{csv}"
        );
        let copy = add_math(&mut shared, "Raw angle", "'User::Oversteer'[rad]");
        shared.math_channels[1].unit = "rad".into();
        evaluate(&mut shared, id);
        assert_eq!(evaluate(&mut shared, copy), vec![0.1; 4]);
        shared.math_channels[0].options.display_unit = None;
        refresh_plotted_channel(&mut pc, &shared);
        assert_eq!(pc.display_scale, 1.0);
        assert!(pc.display_unit.is_none());
        shared.math_channels[0].options.display_unit = Some("deg".into());
        refresh_plotted_channel(&mut pc, &shared);
        assert_eq!(pc.display_unit.as_deref(), Some("deg"));
        set_plotted_channel_display_transform(&mut pc, 1.0, 0.0, None);
        shared.math_channels[0].options.display_unit = Some("°".into());
        refresh_plotted_channel(&mut pc, &shared);
        assert!(
            pc.display_unit.is_none(),
            "panel override survives default changes"
        );
        let workspace = crate::workspace::save_workspace(&[], 0, &shared);
        let restored: crate::workspace::WorkspaceFile =
            serde_json::from_str(&serde_json::to_string(&workspace).unwrap()).unwrap();
        assert_eq!(
            restored.math_channels[0].options.display_unit.as_deref(),
            Some("°")
        );
    }

    #[test]
    fn all_values_render_only_requests_visible_sources() {
        let mut shared = recording();
        shared.values_all_channels = true;
        let total = shared.ld_file.as_ref().unwrap().channels.len();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 380.0),
                )),
                ..Default::default()
            },
            |ui| {
                crate::panels::cursor_readout::show(ui, &mut shared);
            },
        );
        output.textures_delta.clear();
        let requested = shared.take_requested_physical_channel_decodes();
        assert!(!requested.is_empty());
        assert!(requested.len() < total / 2);
        let mut editor = crate::panels::math_editor::MathEditorState::new();
        shared.show_channel_status = true;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::panels::math_editor::show(ui, &mut shared, &mut editor);
            crate::panels::channel_status::show(ui.ctx(), &mut shared);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn enums_comments_sources_and_setup_values_survive_workspace_and_math_json() {
        let mut shared = recording();
        let id = add_math(&mut shared, "User Gear", "1");
        let recording = shared.recording_key().to_string();
        let options = &mut shared.math_channels[0].options;
        options.recording = Some(recording.clone());
        options.source = "Log File".into();
        options.comment = "User-labelled gears".into();
        options.quantity = "State".into();
        options.enum_labels.insert(1, "First".into());
        options.output_freq = Some(2);
        evaluate(&mut shared, id);
        let info = crate::panels::utils::build_plotted_channel_info(
            &crate::panels::utils::create_plotted_channel(ChannelId::Math(0), &shared, 0).unwrap(),
            &shared,
        );
        assert_eq!(info.format_value(1.0).as_deref(), Some("First"));
        let workspace = crate::workspace::save_workspace(&[], 0, &shared);
        let json = serde_json::to_string(&workspace).unwrap();
        let restored: crate::workspace::WorkspaceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.math_channels[0].options,
            shared.math_channels[0].options
        );
        assert_eq!(restored.channel_settings, shared.channel_settings);
        let old: crate::workspace::MathChannelConfig = serde_json::from_str(
            "{\"name\":\"Old\",\"expression\":\"1\",\"unit\":\"\",\"dec_places\":2}",
        )
        .unwrap();
        assert_eq!(old.options, Default::default());
    }
}

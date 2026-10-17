//! One recording identity owns its ranges, details, corrections and track edits.
use crate::state::SharedState;
use i3rs_core::{Lap, corrections::CorrectionSettings, ranges::RecordingAnalysis};
impl SharedState {
    /// Generated drag maths are recording-owned and run through the ordinary
    /// evaluator, so graphs, Values, reports and overlays use the same values.
    pub fn sync_drag_math(&mut self) {
        let recording = self.recording_key().to_string();
        let setup = self.current_analysis().map(|a| a.drag_setup.clone());
        let Some(setup) = setup else { return };
        let expressions = setup.expressions().unwrap_or_default();
        self.math_channels.retain(|mc| {
            !(mc.options.recording.as_deref() == Some(&recording)
                && mc.options.generator.as_deref() == Some("drag")
                && !expressions.iter().any(|(name, _, _)| *name == mc.name))
        });
        for (name, expression, unit) in expressions {
            if let Some(mc) = self.math_channels.iter_mut().find(|mc| {
                mc.options.recording.as_deref() == Some(&recording)
                    && mc.options.generator.as_deref() == Some("drag")
                    && mc.name == name
            }) {
                if mc.expression != expression || mc.unit != unit {
                    mc.expression = expression;
                    mc.unit = unit.into();
                    mc.data = None;
                    mc.evaluation_key = None;
                }
                continue;
            }
            let mut mc = self.create_math_channel_def(name.into(), expression, unit.into(), 2);
            mc.options.source = "Log File".into();
            mc.options.recording = Some(recording.clone());
            mc.options.interpolation = i3rs_core::Interpolation::Linear;
            mc.options.generator = Some("drag".into());
            mc.options.comment = "Generated drag setup".into();
            self.math_channels.push(mc);
        }
    }

    pub fn range_selection(&self) -> i3rs_core::ranges::RangeSelection {
        let range = self
            .selected_lap
            .and_then(|i| self.current_analysis()?.ranges.get(i));
        let (start, end) = self.analysis_range();
        i3rs_core::ranges::RangeSelection {
            recording: self.recording_key().into(),
            range_id: range.map(|r| r.id.clone()),
            kind: range.map(|r| r.kind),
            trust: range.map_or(i3rs_core::ranges::Trust::Unknown, |r| r.trust),
            start,
            end,
        }
    }
    pub fn current_analysis(&self) -> Option<&RecordingAnalysis> {
        self.recording_analyses.get(self.recording_key())
    }
    pub fn current_corrections(&self) -> CorrectionSettings {
        self.current_analysis()
            .map(|a| a.corrections.clone())
            .unwrap_or_default()
    }
    pub fn correction_key(&self) -> String {
        serde_json::to_string(&(
            self.recording_key(),
            self.correction_revision,
            self.current_corrections(),
            self.current_analysis().map(|a| &a.ranges),
        ))
        .expect("serializable correction settings")
    }
    pub fn competition_laps(&self) -> Vec<Lap> {
        self.current_analysis()
            .map(|a| {
                a.ranges
                    .iter()
                    .filter(|r| r.kind.competition())
                    .enumerate()
                    .map(|(i, r)| r.as_lap(i as u32))
                    .collect()
            })
            .unwrap_or_else(|| self.laps.clone())
    }
    pub fn activate_recording_analysis(&mut self, detected: &[Lap]) {
        let key = self.recording_key().to_string();
        let duration = self.data_duration.unwrap_or(0.);
        if !self.recording_analyses.contains_key(&key) {
            let mut a = RecordingAnalysis::from_laps(key.clone(), duration, detected);
            if let Some(ld) = &self.ld_file {
                a.details.driver = ld.session.driver.clone();
                a.details.venue = ld.session.venue.clone();
                a.details.vehicle = ld.session.vehicle_id.clone();
                a.details.event = ld.event.event_name.clone();
            }
            self.recording_analyses.insert(key.clone(), a);
        }
        let a = &self.recording_analyses[&key];
        self.laps = a.laps();
        self.selected_lap = a
            .selected_range
            .as_ref()
            .and_then(|id| a.ranges.iter().position(|r| &r.id == id));
        self.sectors = a.sectors.clone();
        self.pending_track_definition = a.track.clone();
    }
    pub fn snapshot_recording_analysis(
        &self,
    ) -> std::collections::BTreeMap<String, RecordingAnalysis> {
        let mut all = self.recording_analyses.clone();
        if let Some(a) = all.get_mut(self.recording_key()) {
            a.sectors = self.sectors.clone();
            a.track = self
                .track_data_if_ready()
                .and_then(|track| {
                    i3rs_core::export_track_definition(
                        &track,
                        &self.sectors,
                        a.track
                            .as_ref()
                            .map_or_else(|| "Track sections".into(), |t| t.name.clone()),
                    )
                })
                .or_else(|| self.pending_track_definition.clone())
                .or_else(|| a.track.clone());
        }
        all
    }
    pub fn remember_recording_analysis(&mut self) {
        self.recording_analyses = self.snapshot_recording_analysis();
    }

    pub fn apply_correction_result(
        &mut self,
        key: &str,
        result: Result<i3rs_core::corrections::CorrectedRecording, String>,
    ) -> bool {
        if key != self.correction_key() {
            return false;
        }
        match result {
            Ok(recording) => {
                self.correction_error = None;
                self.correction_gps_speed = recording
                    .gps_speed
                    .map(|(v, f)| (std::sync::Arc::from(v), f));
                self.correction_speed = recording.speed.map(|(v, f)| (std::sync::Arc::from(v), f));
                self.correction_distance = recording
                    .distance
                    .map(|(v, f)| (std::sync::Arc::from(v), f));
                for message in recording.messages {
                    self.message(message);
                }
                self.store_track_data(recording.track);
            }
            Err(error) => {
                self.correction_error = Some(error.clone());
                self.message(format!("Corrections: {error}"));
                self.store_track_data(None);
            }
        }
        true
    }
    pub fn apply_recording_analysis(
        &mut self,
        mut analysis: RecordingAnalysis,
    ) -> Result<(), String> {
        analysis.validate()?;
        if analysis.recording != self.recording_key() {
            return Err("Analysis sidecar belongs to another recording".into());
        }
        if (analysis.duration - self.data_duration.unwrap_or(0.)).abs() > 1e-6 {
            return Err("Analysis duration differs from this recording".into());
        }
        let corrections_changed = self.current_analysis().is_none_or(|old| {
            old.corrections != analysis.corrections || old.ranges != analysis.ranges
        });
        let drag_changed = self
            .current_analysis()
            .is_none_or(|old| old.drag_setup != analysis.drag_setup);
        analysis.ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
        analysis.mark_missed_beacons();
        let selected = self
            .selected_lap
            .and_then(|i| self.current_analysis()?.ranges.get(i))
            .map(|r| r.id.clone());
        self.laps = analysis.laps();
        self.selected_lap = analysis
            .selected_range
            .clone()
            .or(selected)
            .and_then(|id| analysis.ranges.iter().position(|r| r.id == id));
        self.pending_track_definition = analysis.track.clone();
        self.sectors = analysis.sectors.clone();
        self.recording_analyses
            .insert(analysis.recording.clone(), analysis);
        if drag_changed {
            self.sync_drag_math();
        }
        self.reference_lap = None;
        if let Some(lap) = self.selected_lap {
            self.select_lap(Some(lap));
        }
        if corrections_changed {
            self.invalidate_correction_caches();
            self.invalidate_math_results();
        } else {
            self.apply_pending_track_definition();
            if drag_changed {
                self.invalidate_math_results();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use i3rs_core::ranges::*;
    use std::sync::Arc;
    fn state() -> SharedState {
        let mut shared = SharedState::new();
        let ld = i3rs_core::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        shared.data_duration = Some(ld.duration_secs());
        let laps = i3rs_core::detect_laps(&ld);
        shared.ld_file = Some(Arc::new(ld));
        shared.activate_recording_analysis(&laps);
        shared
    }
    #[test]
    fn drag_setup_survives_sidecar_and_updates_generated_ids_without_cross_recording_leaks() {
        let mut shared = state();
        let mut a = shared.current_analysis().unwrap().clone();
        a.drag_setup.ratios = vec![2., 1.];
        shared.apply_recording_analysis(a.clone()).unwrap();
        let ids: Vec<_> = shared.math_channels.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 2);
        let saved: RecordingAnalysis =
            serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(saved.drag_setup, a.drag_setup);
        a.drag_setup.ratios[0] = 2.5;
        shared.apply_recording_analysis(a.clone()).unwrap();
        assert_eq!(
            shared
                .math_channels
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert!(shared.math_channels[0].expression.contains("2.5"));
        shared.sync_drag_math();
        assert_eq!(shared.math_channels.len(), 2);
        assert!(
            shared
                .math_channels
                .iter()
                .all(|m| m.options.recording.as_deref() == Some(shared.recording_key()))
        );
        // An overlay context can reconstruct setup maths from recording settings,
        // even if that recording has never been the main session.
        shared.math_channels.clear();
        let context =
            crate::comparison::recording_context(&shared, shared.ld_file.clone().unwrap());
        assert_eq!(context.math_channels.len(), 2);
        assert_ne!(context.math_channels[0].id, context.math_channels[1].id);
        a.drag_setup.generate_clutch_slip = false;
        shared.apply_recording_analysis(a).unwrap();
        assert!(shared.math_channels.iter().all(|m| m.name != "Clutch Slip"));
    }

    #[test]
    fn ranges_details_trust_categories_and_scope_round_trip_and_select_by_identity() {
        let mut shared = state();
        let mut a = shared.current_analysis().unwrap().clone();
        a.ranges = vec![
            AnalysisRange {
                id: "transport".into(),
                name: "Morning".into(),
                start: 0.,
                end: 1.25,
                kind: RangeKind::Transport,
                trust: Trust::Trusted,
                comment: "Warmup".into(),
                reaction_time: None,
            },
            AnalysisRange {
                id: "stage".into(),
                name: "Hill".into(),
                start: 1.25,
                end: 9.75,
                kind: RangeKind::Stage,
                trust: Trust::Trusted,
                comment: String::new(),
                reaction_time: None,
            },
        ];
        a.markers = vec![RangeMarker {
            id: "missed".into(),
            time: 5.5,
            name: "Missed beacon".into(),
            kind: MarkerKind::MissedBeacon,
            trust: Trust::Untrusted,
            distance_m: Some(100.),
        }];
        a.details.custom.insert("Tyres".into(), "Soft".into());
        a.details.driver = "Edited driver".into();
        a.selected_range = Some("stage".into());
        a.gps_generation = Some(GpsGeneration {
            gate: [[36., -79.001], [36., -78.999]],
            minimum_interval: 30.,
            kind: RangeKind::Lap,
        });
        a.sectors = vec![i3rs_core::Sector {
            name: "Renamed".into(),
            semantics: i3rs_core::track::SectionSemantics {
                id: "stable-section".into(),
                kind: i3rs_core::track::SectionKind::Corner,
                categories: ["Slow".into()].into(),
            },
            start_index: 1,
            end_index: 2,
            distance: None,
        }];
        shared.apply_recording_analysis(a).unwrap();
        let selected = shared.range_selection();
        assert_eq!(selected.kind, Some(RangeKind::Stage));
        assert_eq!(selected.trust, Trust::Untrusted);
        assert_eq!((selected.start, selected.end), (1.25, 9.75));
        let workspace = crate::workspace::save_workspace(&[], 0, &shared);
        let saved = serde_json::to_string(&workspace).unwrap();
        let workspace = serde_json::from_str(&saved).unwrap();
        let mut restored = state();
        crate::workspace::load_workspace(&workspace, &mut restored);
        assert_eq!(
            restored.current_analysis().unwrap().ranges,
            shared.current_analysis().unwrap().ranges
        );
        assert_eq!(
            restored.current_analysis().unwrap().details.custom["Tyres"],
            "Soft"
        );
        assert_eq!(restored.sectors[0].semantics.id, "stable-section");
        assert_eq!(
            restored.sectors[0].semantics.kind,
            i3rs_core::track::SectionKind::Corner
        );
        assert_eq!(
            restored.range_selection().range_id.as_deref(),
            Some("stage")
        );
        restored.remember_recording_analysis();
        let mut other = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        other.push(0);
        let other = i3rs_core::LdFile::from_bytes(other).unwrap();
        let other_laps = i3rs_core::detect_laps(&other);
        restored.ld_file = Some(Arc::new(other));
        restored.activate_recording_analysis(&other_laps);
        assert_ne!(
            restored.current_analysis().unwrap().details.driver,
            "Edited driver"
        );
        assert!(restored.sectors.is_empty());
        let original = shared.ld_file.clone().unwrap();
        restored.ld_file = Some(original);
        restored.activate_recording_analysis(&[]);
        assert_eq!(
            restored.current_analysis().unwrap().details.driver,
            "Edited driver"
        );
        assert_eq!(restored.sectors[0].semantics.id, "stable-section");
    }
    #[test]
    fn sidecars_reject_wrong_recording_and_invalid_bounds_without_changing_state() {
        let mut shared = state();
        let original = shared.current_analysis().unwrap().clone();
        let mut bad = original.clone();
        bad.recording = "other".into();
        assert!(shared.apply_recording_analysis(bad).is_err());
        let mut bad = original.clone();
        bad.ranges[0].end = f64::NAN;
        assert!(shared.apply_recording_analysis(bad).is_err());
        assert_eq!(shared.current_analysis().unwrap().ranges, original.ranges);
    }
    #[test]
    fn correction_results_reject_stale_settings_and_generated_maths_use_si_units() {
        let mut shared = state();
        let key = shared.correction_key();
        let result = i3rs_core::corrections::build_corrected_recording(
            shared.ld_file.as_ref().unwrap(),
            &shared.current_corrections(),
            &shared.competition_laps(),
        )
        .unwrap();
        shared.invalidate_correction_caches();
        assert!(!shared.apply_correction_result(&key, Ok(result)));
        assert!(shared.correction_speed.is_none());
        let key = shared.correction_key();
        let result = i3rs_core::corrections::build_corrected_recording(
            shared.ld_file.as_ref().unwrap(),
            &shared.current_corrections(),
            &shared.competition_laps(),
        )
        .unwrap();
        assert!(shared.apply_correction_result(&key, Ok(result)));
        let mc = shared.create_math_channel_def(
            "Generated speed".into(),
            "'MoTeC::Corr Speed'[km/h]".into(),
            "km/h".into(),
            2,
        );
        let id = mc.id;
        shared.math_channels.push(mc);
        let job = crate::panels::math_editor::build_math_evaluation_job(&mut shared, id).unwrap();
        assert_eq!(job.options.input_units["MoTeC::Corr Speed"], "m/s");
        let (data, _) = i3rs_core::math_engine::evaluate_expression_with_options(
            &job.expression,
            &job.channel_data,
            &job.aliases,
            &job.options,
        )
        .unwrap();
        let raw = shared.correction_speed.as_ref().unwrap().0[10];
        assert!((data[10] - raw * 3.6).abs() < 1e-9);
        let revision = shared.correction_revision;
        let mut edit = shared.current_analysis().unwrap().clone();
        edit.details.notes = "Notes only".into();
        shared.apply_recording_analysis(edit).unwrap();
        assert_eq!(shared.correction_revision, revision);
        assert!(shared.correction_speed.is_some());
    }
    #[test]
    fn changed_workspace_policies_invalidate_results_once_and_unchanged_restore_keeps_them() {
        let mut shared = state();
        let mut workspace = crate::workspace::save_workspace(&[], 0, &shared);
        let key = shared.recording_key().to_string();
        workspace
            .recording_analyses
            .get_mut(&key)
            .unwrap()
            .corrections
            .gps_enabled = false;
        let revision = shared.correction_revision;
        crate::workspace::restore_track_sections(&workspace, &mut shared);
        assert_eq!(shared.correction_revision, revision + 1);
        crate::workspace::restore_track_sections(&workspace, &mut shared);
        assert_eq!(shared.correction_revision, revision + 1);
    }
    #[test]
    fn recording_editor_renders_with_typed_selection_and_diagnostics() {
        let mut shared = state();
        shared.show_recording_editor = true;
        let mut editor = crate::panels::recording_editor::RecordingEditor::default();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 900.),
                )),
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), &mut shared),
        );
        output.textures_delta.clear();
    }
}

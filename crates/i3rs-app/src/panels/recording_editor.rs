//! Editors for recording-owned ranges, markers, details and correction policies.
use crate::state::SharedState;
use eframe::egui;
use i3rs_core::corrections::*;
use i3rs_core::ranges::*;
#[derive(Default)]
pub struct RecordingEditor {
    draft: Option<RecordingAnalysis>,
    key: String,
    status: Option<String>,
    json: String,
    counter: u64,
    custom_name: String,
    custom_value: String,
    gate: [[f64; 2]; 2],
    minimum_interval: f64,
    generation_kind: RangeKind,
}
fn kind_control(
    ui: &mut egui::Ui,
    kind: &mut RangeKind,
    id: impl std::hash::Hash + std::fmt::Debug,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(kind.label())
        .show_ui(ui, |ui| {
            for k in [
                RangeKind::Lap,
                RangeKind::OutLap,
                RangeKind::InLap,
                RangeKind::Run,
                RangeKind::Stage,
                RangeKind::Transport,
            ] {
                ui.selectable_value(kind, k, k.label());
            }
        });
}
fn trust_control(ui: &mut egui::Ui, trust: &mut Trust, id: impl std::hash::Hash + std::fmt::Debug) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{trust:?}"))
        .show_ui(ui, |ui| {
            for t in [Trust::Trusted, Trust::Untrusted, Trust::Unknown] {
                ui.selectable_value(trust, t, format!("{t:?}"));
            }
        });
}
impl RecordingEditor {
    fn reset(&mut self, shared: &SharedState) {
        self.draft = shared
            .snapshot_recording_analysis()
            .get(shared.recording_key())
            .cloned();
        self.key = shared.recording_key().into();
        self.status = None;
        self.minimum_interval = 30.;
        self.gate = [[0.; 2]; 2];
        if let Some(g) = self.draft.as_ref().and_then(|a| a.gps_generation.as_ref()) {
            self.gate = g.gate;
            self.minimum_interval = g.minimum_interval;
            self.generation_kind = g.kind;
        }
    }
    fn apply_draft(&mut self, shared: &mut SharedState) -> Result<(), String> {
        let mut analysis = self.draft.clone().ok_or("No recording analysis draft")?;
        prune_run_time_slips(&mut analysis);
        // Track edits can change independently while this draft is open.
        if let Some(current) = shared
            .snapshot_recording_analysis()
            .get(shared.recording_key())
        {
            let name = analysis.track.as_ref().map(|t| t.name.clone());
            analysis.track = current.track.clone();
            if let (Some(name), Some(track)) = (name, analysis.track.as_mut()) {
                track.name = name;
            }
            analysis.sectors = current.sectors.clone();
        }
        shared.apply_recording_analysis(analysis)?;
        self.reset(shared);
        Ok(())
    }
    pub fn show(&mut self, ctx: &egui::Context, shared: &mut SharedState) {
        if !shared.show_recording_editor {
            return;
        }
        if self.key != shared.recording_key() || self.draft.is_none() {
            self.reset(shared);
        }
        let mut open = true;
        egui::Window::new("Ranges, corrections and log details").open(&mut open).default_size([900.,700.]).show(ctx,|ui|{
            if shared.ld_file.is_none(){ui.label("Open a recording to edit its analysis settings.");return;}
            ui.horizontal(|ui|{
                if ui.button("Apply edits").clicked() {
                    self.status = Some(match self.apply_draft(shared) {
                        Ok(()) => "Edits applied; save the project, workspace or sidecar to retain them.".into(),
                        Err(error) => error,
                    });
                }
                if ui.button("Reload draft").clicked(){self.reset(shared);}
                if ui.button("Copy saved analysis JSON").clicked()&&let Some(a)=shared.snapshot_recording_analysis().get(shared.recording_key()){
                    match serde_json::to_string_pretty(a){Ok(json)=>{ctx.copy_text(json.clone());self.json=json;},Err(e)=>self.status=Some(e.to_string())}
                }
                #[cfg(not(target_arch="wasm32"))]
                if ui.button("Save sidecar…").clicked()&&let Some(a)=shared.snapshot_recording_analysis().get(shared.recording_key()){
                    let file=shared.ld_path.as_ref().map(|p|p.with_extension("i3rs.json"));
                    let mut dialog=rfd::FileDialog::new().add_filter("i3rs analysis",&["json"]);
                    if let Some(file)=&file {if let Some(parent)=file.parent(){dialog=dialog.set_directory(parent);}
if let Some(name)=file.file_name(){dialog=dialog.set_file_name(name.to_string_lossy());}}
                    if let Some(path)=dialog.save_file(){self.status=Some(match serde_json::to_string_pretty(a).map_err(|e|e.to_string()).and_then(|json|std::fs::write(path,json).map_err(|e|e.to_string())){Ok(())=>"Analysis sidecar saved".into(),Err(e)=>e});}
                }
            });
            if let Some(status)=&self.status{ui.label(status);}
            if let Some(error)=&shared.correction_error {ui.colored_label(egui::Color32::LIGHT_RED,error);}
            egui::ScrollArea::vertical().show(ui,|ui|{
                let Some(a)=self.draft.as_mut() else{return;};
                ui.collapsing("Log details and custom fields",|ui|{
                    for (label,value) in [("Driver",&mut a.details.driver),("Venue",&mut a.details.venue),("Vehicle",&mut a.details.vehicle),("Event",&mut a.details.event)]{ui.horizontal(|ui|{ui.label(label);ui.text_edit_singleline(value);});}
                    if let Some(track)=&mut a.track {text(ui,"Track name",&mut track.name);}
                    ui.label("Notes");ui.text_edit_multiline(&mut a.details.notes);
                    let mut remove=None;for (name,value) in &mut a.details.custom {ui.horizontal(|ui|{ui.label(name);ui.text_edit_singleline(value);if ui.small_button("Remove").clicked(){remove=Some(name.clone());}});}
if let Some(key)=remove{a.details.custom.remove(&key);}
                    ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.custom_name);ui.text_edit_singleline(&mut self.custom_value);if ui.button("Add custom field").clicked()&&!self.custom_name.trim().is_empty(){a.details.custom.insert(self.custom_name.trim().into(),self.custom_value.clone());}});
                });
                ui.collapsing("Drag: transmission and clutch slip", |ui| {
                    ui.label("Recording-specific transmission input/output ratios. Configure measured ratios; no vehicle ratios are assumed.");
                    text(ui, "Engine RPM channel", &mut a.drag_setup.engine);
                    text(ui, "Drive shaft RPM channel", &mut a.drag_setup.drive_shaft);
                    text(ui, "Gear channel", &mut a.drag_setup.gear);
                    ui.checkbox(&mut a.drag_setup.generate_input_shaft, "Generate Input Shaft Speed");
                    ui.checkbox(&mut a.drag_setup.generate_clutch_slip, "Generate Clutch Slip");
                    let mut remove = None;
                    for (i, ratio) in a.drag_setup.ratios.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Gear {} ratio", i + 1));
                            ui.add(egui::DragValue::new(ratio).speed(0.001).range(0.001..=100.));
                            if ui.small_button("Remove gear").clicked() { remove = Some(i); }
                        });
                    }
                    if let Some(i) = remove { a.drag_setup.ratios.remove(i); }
                    if a.drag_setup.ratios.len() < 16 && ui.button("Add gear ratio").clicked() { a.drag_setup.ratios.push(1.); }
                    if let Err(e) = a.drag_setup.expressions() { ui.weak(e); }
                    ui.label("Apply edits to update the shared maths; save the workspace, project or sidecar to retain setup.");
                });
                ui.collapsing("Drag: run time-slip details", |ui| {
                    ui.label("Create a Run range in the range editor first. Enter cumulative elapsed seconds from launch, excluding reaction time. Empty milestones use corrected-distance crossings when available.");
                    for r in a.ranges.iter().filter(|r| r.kind == RangeKind::Run) {
                        ui.push_id((&r.id, "slip"), |ui| {
                            ui.label(&r.name);
                            let slip = a.drag_runs.entry(r.id.clone()).or_default();
                            for (ft, elapsed) in i3rs_core::drag::MILESTONES_FT.iter().zip(&mut slip.elapsed) {
                                ui.horizontal(|ui| {
                                    let mut enabled = elapsed.is_some();
                                    if ui.checkbox(&mut enabled, format!("{ft} ft elapsed")).changed() { *elapsed = enabled.then_some(0.); }
                                    if let Some(t) = elapsed { ui.add(egui::DragValue::new(t).speed(0.001).suffix(" s")); }
                                });
                            }
                        });
                    }
                });
                ui.collapsing("Ranges (lap / run / stage / transport)",|ui|{
                    ui.label("Ranges are [start, end). Types and trust remain separate from their editable names.");
                    let mut remove=None;
                    for (i,r) in a.ranges.iter_mut().enumerate(){ui.push_id(&r.id,|ui|{ui.horizontal_wrapped(|ui|{ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(130.));kind_control(ui,&mut r.kind,"kind");trust_control(ui,&mut r.trust,"trust");ui.add(egui::DragValue::new(&mut r.start).speed(0.01).prefix("Start ").suffix(" s"));ui.add(egui::DragValue::new(&mut r.end).speed(0.01).prefix("End ").suffix(" s"));if ui.small_button("Select").clicked()&&let Some(index)=shared.current_analysis().and_then(|s|s.ranges.iter().position(|range|range.id==r.id)){shared.select_lap(Some(index));}
                        if ui.small_button("Remove").clicked(){remove=Some(i);}
                    });ui.horizontal(|ui|{ui.label("Comment");ui.text_edit_singleline(&mut r.comment);if r.kind==RangeKind::Run {let mut enabled=r.reaction_time.is_some();if ui.checkbox(&mut enabled,"Reaction time").changed(){r.reaction_time=enabled.then_some(0.);}
if let Some(t)=&mut r.reaction_time{ui.add(egui::DragValue::new(t).speed(0.001).suffix(" s"));}}});});}
                    if let Some(i)=remove{a.ranges.remove(i);}
                    if ui.button("Add range from selected time window").clicked(){let(start,end)=shared.analysis_range();loop{self.counter+=1;let id=format!("user-range-{}",self.counter);if !a.ranges.iter().any(|r|r.id==id){a.ranges.push(AnalysisRange{id,name:"New range".into(),start,end,kind:self.generation_kind,trust:Trust::Unknown,comment:String::new(),reaction_time:None});break;}}}
                });
                ui.collapsing("Markers and range generation",|ui|{
                    ui.label("A missed beacon flags its enclosing range as untrusted; it does not invent a crossing.");
                    let mut remove=None;
                    for(i,m)in a.markers.iter_mut().enumerate(){ui.push_id(&m.id,|ui|{ui.horizontal_wrapped(|ui|{ui.text_edit_singleline(&mut m.name);ui.add(egui::DragValue::new(&mut m.time).speed(0.001).suffix(" s"));egui::ComboBox::from_id_salt("marker-kind").selected_text(format!("{:?}",m.kind)).show_ui(ui,|ui|{for kind in [MarkerKind::Beacon,MarkerKind::Gps,MarkerKind::MissedBeacon,MarkerKind::StageStart,MarkerKind::StageEnd,MarkerKind::User]{ui.selectable_value(&mut m.kind,kind,format!("{kind:?}"));}});trust_control(ui,&mut m.trust,"trust");if ui.small_button("Remove").clicked(){remove=Some(i);}});});}
                    if let Some(i)=remove{a.markers.remove(i);}
                    ui.horizontal_wrapped(|ui|{
                        if ui.button("Add marker at cursor").clicked()&&let Some(time)=shared.cursor_time{loop{self.counter+=1;let id=format!("user-marker-{}",self.counter);if !a.markers.iter().any(|m|m.id==id){a.markers.push(RangeMarker{id,time,name:"Marker".into(),kind:MarkerKind::User,trust:Trust::Unknown,distance_m:None});break;}}}
                        if ui.button("Read recorded beacons").clicked()&&let Some(ld)=&shared.ld_file {let times=i3rs_core::recorded_beacon_times(ld);if times.is_empty(){self.status=Some("No tagged recorded beacons found".into());}else{a.markers=times.into_iter().map(|time|RangeMarker{id:format!("beacon-{:016x}",time.to_bits()),time,name:"Recorded beacon".into(),kind:MarkerKind::Beacon,trust:Trust::Unknown,distance_m:None}).collect();}}
                        kind_control(ui,&mut self.generation_kind,"generation-kind");
                        if ui.button("Replace ranges from markers").clicked(){match ranges_from_markers(&a.markers,a.duration,self.generation_kind){Ok(ranges)=>a.ranges=ranges,Err(e)=>self.status=Some(e)}}
                    });
                    ui.label("GPS gate endpoints (latitude, longitude). Reverse endpoints to reverse the crossing direction.");
                    for endpoint in &mut self.gate {ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut endpoint[0]).speed(0.00001).max_decimals(7));ui.add(egui::DragValue::new(&mut endpoint[1]).speed(0.00001).max_decimals(7));});}
                    ui.add(egui::DragValue::new(&mut self.minimum_interval).range(0.0..=3600.).prefix("Minimum crossing interval ").suffix(" s"));
                    if ui.button("Generate GPS markers").clicked(){let result=shared.track_data_if_ready().ok_or("Corrected GPS track is unavailable".to_string()).and_then(|track|track.geographic_fixes().ok_or("GPS origin is unavailable".to_string()).and_then(|fixes|gps_crossing_markers(&fixes,&track.time,self.gate,self.minimum_interval)));match result{Ok(markers)=>{self.status=Some(format!("{} directed crossings generated",markers.len()));a.markers=markers;a.gps_generation=Some(GpsGeneration{gate:self.gate,minimum_interval:self.minimum_interval,kind:self.generation_kind});},Err(e)=>self.status=Some(e)}}
                });
                ui.collapsing("Corrected speed, distance and GPS",|ui|{correction_controls(ui,&mut a.corrections);});
                prune_run_time_slips(a);
                if let Err(error)=a.validate(){ui.colored_label(egui::Color32::LIGHT_RED,error);}
            });
            ui.collapsing("Import analysis JSON",|ui|{
                #[cfg(not(target_arch="wasm32"))]
                if ui.button("Read sidecar…").clicked()&&let Some(path)=rfd::FileDialog::new().add_filter("i3rs analysis",&["json"]).pick_file(){match std::fs::read_to_string(path){Ok(json)=>self.json=json,Err(e)=>self.status=Some(e.to_string())}}
                ui.add(egui::TextEdit::multiline(&mut self.json).desired_rows(4));
                if ui.button("Import for this recording").clicked(){let result=serde_json::from_str::<RecordingAnalysis>(&self.json).map_err(|e|e.to_string()).and_then(|a|shared.apply_recording_analysis(a));self.status=Some(match result{Ok(())=>{self.reset(shared);"Recording analysis imported".into()},Err(e)=>e});}
            });
        });
        shared.show_recording_editor = open;
    }
}
fn prune_run_time_slips(analysis: &mut RecordingAnalysis) {
    analysis.drag_runs.retain(|id, _| {
        analysis
            .ranges
            .iter()
            .any(|r| &r.id == id && r.kind == RangeKind::Run)
    });
}

fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
    });
}
fn limit(ui: &mut egui::Ui, label: &str, limit: &mut Option<SpeedLimit>) {
    let mut enabled = limit.is_some();
    if ui.checkbox(&mut enabled, label).changed() {
        *limit = enabled.then_some(SpeedLimit {
            threshold_mps: 0.,
            action: ClampAction::Invalidate,
        });
    }
    if let Some(l) = limit {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut l.threshold_mps)
                    .range(0.0..=500.)
                    .suffix(" m/s"),
            );
            egui::ComboBox::from_id_salt(label)
                .selected_text(format!("{:?}", l.action))
                .show_ui(ui, |ui| {
                    for action in [
                        ClampAction::Zero,
                        ClampAction::Invalidate,
                        ClampAction::Clamp,
                    ] {
                        ui.selectable_value(&mut l.action, action, format!("{action:?}"));
                    }
                });
        });
    }
}
fn correction_controls(ui: &mut egui::Ui, s: &mut CorrectionSettings) {
    if ui.button("Restore validated defaults").clicked() {
        *s = Default::default();
    }
    ui.checkbox(&mut s.speed_enabled, "Enable corrected speed");
    ui.checkbox(
        &mut s.prefer_logged_corrected_speed,
        "Prefer logged Corr Speed",
    );
    let mut pair = s.fastest_pair.is_some();
    if ui
        .checkbox(&mut pair, "Use fastest value from both channels")
        .changed()
    {
        s.fastest_pair = pair.then_some([
            "Wheel Speed Front Left".into(),
            "Wheel Speed Front Right".into(),
        ]);
    }
    if let Some(pair) = &mut s.fastest_pair {
        text(ui, "Channel 1", &mut pair[0]);
        text(ui, "Channel 2", &mut pair[1]);
    }
    ui.label("Fallback priority (one channel per line)");
    let mut names = s.speed_priority.join("\n");
    if ui.text_edit_multiline(&mut names).changed() {
        s.speed_priority = names
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from)
            .collect();
    }
    ui.checkbox(&mut s.gps_fallback, "Otherwise use GPS speed");
    text(ui, "GPS speed channel", &mut s.gps_speed_channel);
    limit(ui, "Lower speed threshold", &mut s.lower_speed);
    limit(ui, "Upper speed threshold", &mut s.upper_speed);
    let mut wheel = s.wheel_lock.is_some();
    if ui
        .checkbox(
            &mut wheel,
            "Correct wheel locks using longitudinal acceleration",
        )
        .changed()
    {
        s.wheel_lock = wheel.then_some(WheelLockCorrection {
            acceleration_channel: "G Force Long".into(),
            lag_seconds: 0.,
            braking_threshold_mps2: -0.980665,
            speed_threshold_mps: 0.5,
        });
    }
    if let Some(w) = &mut s.wheel_lock {
        text(ui, "Acceleration channel", &mut w.acceleration_channel);
        ui.add(
            egui::DragValue::new(&mut w.lag_seconds)
                .speed(0.001)
                .prefix("Lag ")
                .suffix(" s"),
        );
        ui.add(
            egui::DragValue::new(&mut w.braking_threshold_mps2)
                .prefix("Braking below ")
                .suffix(" m/s²"),
        );
        ui.add(
            egui::DragValue::new(&mut w.speed_threshold_mps)
                .prefix("Correction threshold ")
                .suffix(" m/s"),
        );
    }
    ui.separator();
    ui.checkbox(&mut s.distance_enabled, "Enable corrected distance");
    egui::ComboBox::from_id_salt("distance-source")
        .selected_text(format!("{:?}", s.distance_source))
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut s.distance_source,
                DistanceSource::Automatic,
                "Existing automatic policy",
            );
            ui.selectable_value(
                &mut s.distance_source,
                DistanceSource::CorrectedSpeed,
                "Integrate corrected speed",
            );
            ui.selectable_value(
                &mut s.distance_source,
                DistanceSource::Channel,
                "Recorded distance channel",
            );
        });
    if s.distance_source == DistanceSource::Channel {
        text(ui, "Distance channel", &mut s.distance_channel);
    }
    ui.checkbox(
        &mut s.stretch_enabled,
        "Stretch complete competition ranges to track length",
    );
    ui.add(
        egui::DragValue::new(&mut s.stretch_tolerance)
            .range(0.0..=1.0)
            .speed(0.001)
            .prefix("Tolerance fraction "),
    );
    let mut length = s.track_length_m.is_some();
    if ui.checkbox(&mut length, "Explicit track length").changed() {
        s.track_length_m = length.then_some(5337.5);
    }
    if let Some(length) = &mut s.track_length_m {
        ui.add(
            egui::DragValue::new(length)
                .range(1.0..=1_000_000.)
                .suffix(" m"),
        );
    } else {
        ui.weak("Distance sections use their model length.");
    }
    ui.separator();
    ui.checkbox(&mut s.gps_enabled, "Enable corrected GPS");
    text(ui, "Latitude (empty = automatic)", &mut s.latitude_channel);
    text(
        ui,
        "Longitude (empty = automatic)",
        &mut s.longitude_channel,
    );
    egui::ComboBox::from_id_salt("gps-interp")
        .selected_text(format!("{:?}", s.gps_interpolation))
        .show_ui(ui, |ui| {
            for policy in [
                i3rs_core::Interpolation::Linear,
                i3rs_core::Interpolation::Previous,
                i3rs_core::Interpolation::Nearest,
            ] {
                ui.selectable_value(&mut s.gps_interpolation, policy, format!("{policy:?}"));
            }
        });
    ui.checkbox(
        &mut s.interpolate_duplicates,
        "Interpolate repeated finite GPS fixes",
    );
    let mut mode = match s.gps_shift {
        GpsShift::None => 0,
        GpsShift::Manual(_) => 1,
        GpsShift::Automatic(_) => 2,
    };
    let before = mode;
    egui::ComboBox::from_id_salt("gps-shift")
        .selected_text(["No time shift", "Manual time shift", "Automatic time shift"][mode])
        .show_ui(ui, |ui| {
            for (i, label) in ["No time shift", "Manual time shift", "Automatic time shift"]
                .iter()
                .enumerate()
            {
                ui.selectable_value(&mut mode, i, *label);
            }
        });
    if mode != before {
        s.gps_shift = match mode {
            1 => GpsShift::Manual(0.),
            2 => GpsShift::Automatic(0.8),
            _ => GpsShift::None,
        };
    }
    match &mut s.gps_shift {
        GpsShift::Manual(v) => {
            ui.add(
                egui::DragValue::new(v)
                    .range(-60.0..=60.)
                    .speed(0.001)
                    .suffix(" s advance"),
            );
        }
        GpsShift::Automatic(v) => {
            ui.add(
                egui::DragValue::new(v)
                    .range(0.0..=1.0)
                    .speed(0.01)
                    .prefix("Fix interval fraction "),
            );
        }
        _ => {}
    }
    let mut quality = s.gps_quality.is_some();
    if ui
        .checkbox(&mut quality, "Validate GPS with a quality channel")
        .changed()
    {
        s.gps_quality = quality.then_some(("GPS Sats Used".into(), 4.));
    }
    if let Some((name, min)) = &mut s.gps_quality {
        text(ui, "Quality channel", name);
        ui.add(egui::DragValue::new(min).prefix("Minimum "));
        ui.checkbox(
            &mut s.invalidate_position_with_quality,
            "Also invalidate position",
        );
    }
    let mut zero = s.gps_zero_below_mps.is_some();
    if ui
        .checkbox(&mut zero, "Zero GPS speed below threshold")
        .changed()
    {
        s.gps_zero_below_mps = zero.then_some(0.5);
    }
    if let Some(v) = &mut s.gps_zero_below_mps {
        ui.add(egui::DragValue::new(v).range(0.0..=500.).suffix(" m/s"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn editor_with_runs() -> (RecordingEditor, SharedState) {
        let ld = Arc::new(
            i3rs_core::LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        );
        let mut shared = SharedState::new();
        shared.data_duration = Some(ld.duration_secs());
        shared.ld_file = Some(ld);
        shared.activate_recording_analysis(&[]);
        let mut analysis = shared.current_analysis().unwrap().clone();
        for (id, start, end) in [("remove", 0., 5.), ("keep", 5., 10.)] {
            analysis.ranges.push(AnalysisRange {
                id: id.into(),
                name: id.into(),
                start,
                end,
                kind: RangeKind::Run,
                trust: Trust::Unknown,
                comment: String::new(),
                reaction_time: None,
            });
            analysis.drag_runs.insert(
                id.into(),
                i3rs_core::drag::RunDetails {
                    elapsed: [Some(1.), None, None, None, None],
                },
            );
        }
        shared.apply_recording_analysis(analysis).unwrap();
        let mut editor = RecordingEditor::default();
        editor.reset(&shared);
        (editor, shared)
    }
    #[test]
    fn deleting_a_run_applies_on_first_attempt_without_opening_the_slip_section() {
        let (mut editor, mut shared) = editor_with_runs();
        editor.draft.as_mut().unwrap().ranges.remove(0);
        editor.apply_draft(&mut shared).unwrap();
        let a = shared.current_analysis().unwrap();
        assert!(!a.drag_runs.contains_key("remove"));
        assert_eq!(a.drag_runs["keep"].elapsed[0], Some(1.));
        assert_eq!(a.ranges.len(), 1);
    }
    #[test]
    fn changing_a_run_kind_prunes_only_its_slip_on_first_apply() {
        let (mut editor, mut shared) = editor_with_runs();
        editor.draft.as_mut().unwrap().ranges[0].kind = RangeKind::Stage;
        editor.apply_draft(&mut shared).unwrap();
        let a = shared.current_analysis().unwrap();
        assert_eq!(a.ranges[0].kind, RangeKind::Stage);
        assert!(!a.drag_runs.contains_key("remove"));
        assert!(a.drag_runs.contains_key("keep"));
    }
    #[test]
    fn invalid_retained_slip_is_still_rejected_without_changing_saved_analysis() {
        let (mut editor, mut shared) = editor_with_runs();
        editor
            .draft
            .as_mut()
            .unwrap()
            .drag_runs
            .get_mut("keep")
            .unwrap()
            .elapsed[0] = Some(6.);
        assert!(editor.apply_draft(&mut shared).is_err());
        assert_eq!(
            shared.current_analysis().unwrap().drag_runs["keep"].elapsed[0],
            Some(1.)
        );
    }
}

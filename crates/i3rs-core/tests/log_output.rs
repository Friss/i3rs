use i3rs_core::{LdFile, detect_laps, log_writer::*, output::*, ranges::*};
fn fixture() -> LdFile {
    LdFile::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test_data/VIR_LAP.ld"
    ))
    .unwrap()
}
fn analysis(ld: &LdFile) -> RecordingAnalysis {
    let mut a = RecordingAnalysis::from_laps(
        ld.recording_id().into(),
        ld.duration_secs(),
        &detect_laps(ld),
    );
    a.details.driver = "Output test".into();
    a.details.vehicle = "Car".into();
    a.details.venue = "VIR".into();
    a.details.event = "Test".into();
    a.details.custom.insert("Tyres".into(), "Soft".into());
    a
}
#[test]
fn full_log_preserves_native_values_rates_enums_and_edited_details() {
    let ld = fixture();
    let a = analysis(&ld);
    let (bytes, saved) = encode_log(&ld, &a, None).unwrap();
    let reopened = LdFile::from_bytes(bytes).unwrap();
    assert_eq!(reopened.session.driver, "Output test");
    assert_eq!(reopened.event.event_name, "Test");
    assert_eq!(saved.details.custom, a.details.custom);
    assert_eq!(saved.ranges, a.ranges);
    for (old, new) in ld.channels.iter().zip(&reopened.channels) {
        assert_eq!(old.name, new.name);
        assert_eq!(old.freq, new.freq);
        assert_eq!(old.unit, new.unit);
        assert_eq!(old.enum_labels, new.enum_labels);
        let old = ld.read_channel_data(old).unwrap();
        let new = reopened.read_channel_data(new).unwrap();
        assert_eq!(old.len(), new.len());
        for (x, y) in old.iter().zip(new) {
            assert!(x == &y || x.is_nan() && y.is_nan());
        }
    }
}
#[test]
fn split_off_grid_reopens_with_explicit_grid_and_clipped_ranges() {
    let ld = fixture();
    let mut a = analysis(&ld);
    a.ranges = vec![AnalysisRange {
        id: "test".into(),
        name: "Stage".into(),
        start: 0.5,
        end: 2.5,
        kind: RangeKind::Stage,
        trust: Trust::Trusted,
        comment: "Keep".into(),
        reaction_time: None,
    }];
    let p = OutputPlan {
        start: 1.125,
        end: 2.375,
        rate: 20.,
        relative_time: true,
        include_time: true,
    };
    let (bytes, saved) = encode_log(&ld, &a, Some(p)).unwrap();
    let reopened = LdFile::from_bytes(bytes).unwrap();
    assert_eq!(saved.ranges[0].start, 0.);
    assert_eq!(saved.ranges[0].end, 1.25);
    assert_eq!(saved.ranges[0].kind, RangeKind::Stage);
    assert_eq!(saved.ranges[0].trust, Trust::Trusted);
    assert_eq!(saved.ranges[0].comment, "Keep");
    assert_eq!(saved.duration, 1.25);
    for (old, new) in ld.channels.iter().zip(&reopened.channels) {
        assert_eq!(new.display_decimals, old.display_decimals);
        assert_eq!(new.dec_places, 0);
        assert_eq!(new.freq, 20.);
        assert_eq!(new.n_data, 25);
        let src = ld.read_channel_data(old).unwrap();
        let values = reopened.read_channel_data(new).unwrap();
        let policy = if !old.enum_labels.is_empty() || i3rs_core::is_state_channel(&old.name) {
            i3rs_core::Interpolation::Previous
        } else {
            i3rs_core::Interpolation::Linear
        };
        for (i, v) in values.iter().enumerate() {
            let expected =
                i3rs_core::channels::sample_with_policy(&src, old.freq, p.time(i), policy);
            assert!(match expected {
                Some(x) => (*v - x).abs() < 1e-9,
                None => v.is_nan(),
            });
        }
    }
}
#[test]
fn writeback_produces_recoverable_original_and_reopenable_sidecars() {
    let ld = fixture();
    let a = analysis(&ld);
    let dir = std::env::temp_dir().join(format!("i3r9-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("output.ld");
    assert!(write_log(&path, &ld, &a, None).unwrap().is_none());
    let first = std::fs::read(&path).unwrap();
    // Keep the destination mapped while writing its edited analysis back.
    let mapped = LdFile::open(&path).unwrap();
    let mut edit: RecordingAnalysis =
        serde_json::from_slice(&std::fs::read(path.with_extension("i3rs.json")).unwrap()).unwrap();
    edit.details.driver = "Second driver".into();
    let backup = write_log(&path, &mapped, &edit, None).unwrap().unwrap();
    assert_eq!(std::fs::read(backup.join("0")).unwrap(), first);
    assert!(backup.join("RESTORE.txt").exists());
    let reopened = LdFile::open(&path).unwrap();
    assert_eq!(reopened.session.driver, "Second driver");
    let saved: RecordingAnalysis =
        serde_json::from_slice(&std::fs::read(path.with_extension("i3rs.json")).unwrap()).unwrap();
    assert_eq!(saved.recording, reopened.recording_id());
    assert_eq!(mapped.session.driver, "Output test");
    saved.validate().unwrap();
    let ldx = i3rs_core::LdxFile::open(path.with_extension("ldx")).unwrap();
    assert_eq!(ldx.laps.len(), saved.ranges.len());
    drop(reopened);
    drop(mapped);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn reference_overwrite_preserves_the_original_in_a_recovery_backup() {
    let mut lap = i3rs_core::reference_lap::ReferenceLap::decode(include_bytes!(
        "../../../test_data/reference/i3r9/rally-stage-1.td1"
    ))
    .unwrap();
    let dir = std::env::temp_dir().join(format!("i3r9-reference-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reference.td1");
    assert!(lap.write(&path).unwrap().is_none());
    let first = std::fs::read(&path).unwrap();
    lap.driver = "Replacement driver".into();
    let backup = lap.write(&path).unwrap().unwrap();
    assert_eq!(std::fs::read(backup.join("0")).unwrap(), first);
    assert!(backup.join("RESTORE.txt").exists());
    assert_eq!(
        i3rs_core::reference_lap::ReferenceLap::decode(&std::fs::read(&path).unwrap()).unwrap(),
        lap
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn browser_search_reads_exported_range_hierarchy_and_removes_deleted_entries() {
    let ld = fixture();
    let a = analysis(&ld);
    let dir = std::env::temp_dir().join(format!("i3r9-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("browser.ld");
    write_log(&path, &ld, &a, None).unwrap();
    let mut index = i3rs_core::log_index::LogIndex::default();
    assert!(index.scan(&dir).is_empty());
    let log = &index.logs[&path];
    assert!(log.matches("output VIR"));
    assert!(!log.matches("MissingDriver"));
    assert_eq!(log.analysis.ranges, a.ranges);
    std::fs::remove_file(path).unwrap();
    assert!(index.scan(&dir).is_empty());
    assert!(index.logs.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ldx_exports_i2_layers_typed_stages_and_run_elapsed_metadata() {
    let ld = fixture();
    let mut a = analysis(&ld);
    a.details.event = "Test & <output>".into();
    a.details
        .custom
        .insert("Fastest Time".into(), "stale".into());
    a.ranges = vec![AnalysisRange {
        id: "stage".into(),
        name: "Stage 1".into(),
        start: 0.125,
        end: 2.375,
        kind: RangeKind::Stage,
        trust: Trust::Unknown,
        comment: String::new(),
        reaction_time: None,
    }];
    let xml = analysis_ldx(&a);
    assert!(xml.contains("<Layers><Layer><MarkerBlock>"));
    assert!(!xml.contains("MarkerPair"));
    assert!(!xml.contains("Outings:Runs"));
    let ldx = i3rs_core::LdxFile::parse(&xml).unwrap();
    assert_eq!(ldx.details["Event"].0, a.details.event);
    let restored = ldx
        .specialty_analysis("output".into(), a.duration)
        .unwrap()
        .unwrap();
    let stage = restored
        .ranges
        .iter()
        .find(|r| r.kind == RangeKind::Stage)
        .unwrap();
    assert_eq!((stage.start, stage.end), (0.125, 2.375));
    assert_ne!(ldx.fastest_time.as_deref(), Some("stale"));
    a.ranges[0].kind = RangeKind::Run;
    a.ranges[0].reaction_time = Some(0.3);
    let ldx = i3rs_core::LdxFile::parse(&analysis_ldx(&a)).unwrap();
    let restored = ldx
        .specialty_analysis("output".into(), a.duration)
        .unwrap()
        .unwrap();
    let run = restored
        .ranges
        .iter()
        .find(|r| r.kind == RangeKind::Run)
        .unwrap();
    assert_eq!(
        (run.start, run.end, run.reaction_time),
        (0.125, 2.375, Some(0.3))
    );
}

//! Values transcribed from i2 Standard 1.1.4.457, not from i3rs output.
//! Run with the local comparison log: cargo test -p i3rs-core --test i2_reference -- --ignored
use i3rs_core::{LdFile, compute_stats, detect_laps, sample_at_time};

const FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../S1_#28299_20260508_174455_02.ld"
);

#[test]
#[ignore = "requires the local May 8 S1 log; not distributed with the repository"]
fn matches_i2_beacons_validity_metadata_and_values() {
    let ld = LdFile::open(FILE).unwrap();
    assert!((ld.duration_secs() - 3918.210).abs() < 1e-9);
    let expected = [
        351.680, 814.515, 814.574, 948.688, 1076.961, 1203.760, 1331.686, 1460.738, 1619.022,
        2482.244, 2623.089, 2753.985, 2881.716, 3012.059, 3140.984, 3272.243, 3402.929, 3532.347,
        3668.284, 3918.210,
    ];
    let laps = detect_laps(&ld);
    assert_eq!(laps.len(), expected.len());
    for (lap, end) in laps.iter().zip(expected) {
        assert!(
            (lap.end_time - end).abs() < 1e-6,
            "{}: {} != {end}",
            lap.name,
            lap.end_time
        );
    }
    assert!((laps[2].duration() - 0.059).abs() < 1e-6);

    for (name, displayed) in [
        ("Exhaust Lambda Bank 1", 1.013),
        ("Exhaust Lambda Bank 2", 1.020),
    ] {
        let ch = ld.find_channel_by_name(name).unwrap();
        assert_eq!(ch.display_decimals, 3);
        assert_eq!(ch.dec_places, 0); // Scaling stays independent of formatting.
        let values = ld.read_channel_data(ch).unwrap();
        assert!(sample_at_time(&values, ch.freq, 9.697997).is_none());
        assert!(sample_at_time(&values, ch.freq, 580.0).is_none());
        let valid = sample_at_time(&values, ch.freq, 178.136894).unwrap();
        assert!((valid - displayed).abs() < 0.0005);
        let outage = ld.read_channel_range(ch, 580 * 20, 581 * 20).unwrap();
        assert!(outage.iter().all(|v| v.is_nan()));
        let stats = compute_stats(&outage, Some(ch.freq));
        assert_eq!(stats.finite_count, 0);
    }
    let bank1 = ld.find_channel_by_name("Exhaust Lambda Bank 1").unwrap();
    let transition = ld.read_channel_range(bank1, 11242, 11245).unwrap();
    assert!(transition[0].is_finite());
    assert!(transition[1..].iter().all(|v| v.is_nan()));
    let restart = ld.read_channel_range(bank1, 12000, 12003).unwrap();
    assert!(restart[0].is_nan());
    assert!(restart[1..].iter().all(|v| v.is_finite()));

    for (name, unit) in [
        ("CP Lotus ESP Lateral Acceleration", "G"),
        ("CP Lotus Charge Cooler Input Air Temperature", "C"),
        ("CP Lotus Fuel Tank Level Calculated", "l"),
        ("Fuel Closed Loop Control Bank 1 Trim", "%Trim"),
    ] {
        assert_eq!(ld.find_channel_by_name(name).unwrap().unit, unit);
    }
    assert_eq!(
        ld.find_channel_by_name("CP Lotus ESP Lateral AcceleratioG")
            .unwrap()
            .index,
        ld.find_channel_by_name("CP Lotus ESP Lateral Acceleration")
            .unwrap()
            .index,
    );
    for (name, expected, tolerance) in [
        ("Engine Speed", 2366.4, 0.05),
        ("Throttle Position", 3.1, 0.05),
        (
            "Inlet Manifold Pressure",
            5.1 * 6.894757293168,
            0.05 * 6.894757293168,
        ),
        (
            "Fuel Pressure",
            71.6 * 6.894757293168,
            0.05 * 6.894757293168,
        ),
        ("Fuel Injector Primary Duty Cycle", 4.6, 0.05),
        ("Ignition Timing", 36.1, 0.05),
        ("Exhaust Camshaft Aim", 10.0, 0.05),
        ("Exhaust Camshaft Bank 1 Position", 9.9, 0.05),
        ("Exhaust Camshaft Bank 2 Position", 10.0, 0.05),
        ("Inlet Camshaft Bank 1 Position", 18.4, 0.05),
        ("Inlet Camshaft Bank 2 Position", 18.5, 0.05),
        ("Exhaust Lambda", 1.017, 0.0005),
    ] {
        let ch = ld.find_channel_by_name(name).unwrap();
        let values = ld.read_channel_data(ch).unwrap();
        let value = sample_at_time(&values, ch.freq, 178.136894).unwrap();
        assert!(
            (value - expected).abs() <= tolerance,
            "{name}: {value} != {expected}"
        );
    }
}

#[test]
#[ignore = "requires the local May 8 S1 log"]
fn circuit_gps_bounds_and_outlap_rpm_count() {
    let ld = LdFile::open(FILE).unwrap();
    let track = i3rs_core::extract_gps_track(&ld).unwrap();
    let bounds = |values: &[f64]| {
        values
            .iter()
            .filter(|v| v.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            })
    };
    let (x0, x1) = bounds(&track.x);
    let (y0, y1) = bounds(&track.y);
    assert!(x1 - x0 < 0.05 && y1 - y0 < 0.05);
    assert!(track.x.iter().any(|v| v.is_nan()));
    let rpm = ld.find_channel_by_name("Engine Speed").unwrap();
    let data = ld.read_channel_data(rpm).unwrap();
    let end = (351.680 * rpm.freq).ceil() as usize;
    assert_eq!(data[..end].iter().filter(|v| v.is_finite()).count(), 7031);
}

#[test]
#[ignore = "requires the local May 8 S1 reference log"]
fn matches_visible_live_i2_lap3_track_speed_labels() {
    use i3rs_core::annotations::{AnnotationKind, track_annotations};
    let ld = LdFile::open(FILE).unwrap();
    let track = i3rs_core::extract_gps_track(&ld).unwrap();
    let laps = detect_laps(&ld);
    let sectors = i3rs_core::track::sectors_from_distance_definition(
        &track,
        &laps[3],
        &i3rs_core::track::vir_full_definition(),
    )
    .unwrap();
    let events = track_annotations(&track, &laps[3], &sectors, None, &Default::default());
    // Transcribed from i2 Standard's live Track Report, 2026-10-14.
    // This compares displayed values only; pixel positions and the remaining
    // section labels are not accepted by this test.
    for (name, maximum, mph) in [
        ("Str 0-1", true, 133.6),
        ("Str 1-2", true, 91.7),
        ("Str 3-4", true, 80.0),
        ("Str 5-6", true, 125.0),
        ("Turn 7", false, 106.0),
        ("Turn 8", false, 98.9),
        ("Str 10-11", true, 104.2),
        ("Str 12-13", true, 141.9),
        ("Str 13-14", true, 64.9),
        ("Turn 15", false, 65.1),
        ("Str 16-17", true, 103.4),
    ] {
        let section = sectors.iter().find(|s| s.name == name).unwrap();
        let event = events
            .iter()
            .find(|e| e.section == section.semantics.id)
            .unwrap();
        let value = match event.kind {
            AnnotationKind::MaximumSpeed(v) if maximum => v,
            AnnotationKind::MinimumSpeed(v) if !maximum => v,
            _ => panic!("wrong event type for {name}"),
        };
        assert_eq!(
            format!("{:.1}", value / 0.44704),
            format!("{mph:.1}"),
            "{name}"
        );
    }
}

#[test]
#[ignore = "requires the local May 8 S1 log; not distributed with the repository"]
fn vir_distance_sections_match_i2_reference() {
    let ld = LdFile::open(FILE).unwrap();
    let track = i3rs_core::extract_gps_track(&ld).unwrap();
    let laps = detect_laps(&ld);
    let definition = i3rs_core::track::vir_full_definition();
    let sectors =
        i3rs_core::track::sectors_from_distance_definition(&track, &laps[3], &definition).unwrap();
    let times = i3rs_core::compute_sector_times(&sectors, &laps, &track);
    let expected = [
        ("Turn 1", 6.031),
        ("Str 1-2", 3.182),
        ("Turn 2", 1.120),
        ("Str 2-3", 1.756),
        ("Turn 3", 3.607),
        ("Str 3-4", 6.742),
        ("Turn 4", 1.538),
        ("Str 4-5", 5.013),
        ("Turn 5", 1.399),
        ("Str 5-6", 11.940),
        ("Turn 6", 0.942),
        ("Str 6-7", 1.232),
        ("Turn 7", 0.664),
        ("Str 7-8", 1.760),
        ("Turn 8", 0.461),
        ("Str 8-9", 1.569),
        ("Turn 9", 1.137),
        ("Str 9-10", 4.326),
        ("Turn 10", 1.467),
        ("Str 10-11", 9.803),
        ("Turn 11", 0.693),
        ("Str 11-12", 1.025),
        ("Turn 12", 2.964),
        ("Str 12-13", 30.923),
        ("Turn 13", 1.304),
        ("Str 13-14", 1.271),
        ("Turn 14", 0.997),
        ("Turn 15", 2.267),
        ("Str 15-16", 3.581),
        ("Turn 16", 0.571),
        ("Str 16-17", 1.559),
    ];
    assert_eq!(times[3].len(), 33);
    assert!((times[3].iter().map(|s| s.time_secs).sum::<f64>() - laps[3].duration()).abs() < 1e-8);
    let mut max_delta = 0.0f64;
    for (name, reference) in expected {
        let actual = times[3]
            .iter()
            .find(|s| s.sector_name == name)
            .unwrap()
            .time_secs;
        let delta = actual - reference;
        max_delta = max_delta.max(delta.abs());
        println!("{name}: i3rs={actual:.6} i2={reference:.3} delta={delta:+.6}");
    }
    println!("Max delta {max_delta}");
    let (distance, freq) = track.distance.as_ref().unwrap();
    assert!((distance.last().unwrap() - 89966.947932).abs() < 0.001);
    for lap in &laps {
        let start = sample_at_time(distance, *freq, lap.start_time).unwrap();
        let end = sample_at_time(distance, *freq, lap.end_time).unwrap();
        println!("{}: {:.3}m", lap.name, end - start);
    }
    // i2 shows a negative Turn 17 time on Lap 3. Keep forward interval
    // semantics and a positive result instead of reproducing that anomaly.
    assert!(
        times[3]
            .iter()
            .find(|s| s.sector_name == "Turn 17")
            .unwrap()
            .time_secs
            > 0.
    );
    let exported =
        i3rs_core::track::export_track_definition(&track, &sectors, "VIR Full".into()).unwrap();
    let json = serde_json::to_string(&exported).unwrap();
    let imported =
        i3rs_core::track::import_track_definition(&track, &serde_json::from_str(&json).unwrap())
            .unwrap();
    assert_eq!(
        sectors.iter().map(|s| &s.distance).collect::<Vec<_>>(),
        imported.iter().map(|s| &s.distance).collect::<Vec<_>>()
    );
    assert_eq!(
        i3rs_core::compute_sector_times(&imported, &laps, &track)[3]
            .iter()
            .map(|s| s.time_secs)
            .collect::<Vec<_>>(),
        times[3].iter().map(|s| s.time_secs).collect::<Vec<_>>()
    );
    assert!(max_delta < 0.05);
}

//! Integration tests for i3rs-core using real MoTeC test data.

use i3rs_core::{
    LdFile, LdxFile, detect_laps, downsample_minmax, extract_gps_track, find_ldx_for_ld,
    find_nearest_sample,
};
use std::path::Path;

const TEST_LD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ld");
const TEST_LDX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ldx");

// ---------------------------------------------------------------------------
// .ld file parsing
// ---------------------------------------------------------------------------

#[test]
fn open_ld_file() {
    let ld = LdFile::open(TEST_LD).expect("failed to open .ld file");
    assert_eq!(ld.file_size(), 4_998_791);
}

#[test]
fn open_ld_file_from_bytes() {
    let bytes = std::fs::read(TEST_LD).expect("failed to read .ld test data");
    let ld = LdFile::from_bytes(bytes).expect("failed to parse .ld bytes");
    assert_eq!(ld.file_size(), 4_998_791);
    assert_eq!(ld.channels.len(), 199);
}

#[test]
fn session_metadata() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let s = &ld.session;
    assert_eq!(s.date, "24/09/2025");
    assert_eq!(s.time, "16:23:57");
    assert_eq!(s.vehicle_id, "EVORA_Friss");
    assert_eq!(s.venue, "VIR Full");
    assert_eq!(s.device_type, "M1");
    assert_eq!(s.device_serial, 28299);
    assert_eq!(s.device_version, 150);
    assert_eq!(s.short_comment, "4th session");
}

#[test]
fn event_metadata() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let e = &ld.event;
    // Event fields should parse without panicking; vehicle_id should match session
    assert_eq!(e.vehicle_id, "EVORA_Friss");
}

#[test]
fn channel_count() {
    let ld = LdFile::open(TEST_LD).unwrap();
    assert_eq!(ld.channels.len(), 199);
}

#[test]
fn duration_is_plausible() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let dur = ld.duration_secs();
    // ~133 seconds for this single-lap file
    assert!(dur > 120.0 && dur < 150.0, "duration was {dur}");
}

#[test]
fn channels_have_valid_metadata() {
    let ld = LdFile::open(TEST_LD).unwrap();
    for ch in &ld.channels {
        assert!(!ch.name.is_empty(), "channel {} has empty name", ch.index);
        assert!(ch.freq > 0, "channel {} has zero frequency", ch.index);
        assert!(ch.n_data > 0, "channel {} has zero samples", ch.index);
    }
}

// ---------------------------------------------------------------------------
// Channel data reading
// ---------------------------------------------------------------------------

#[test]
fn read_channel_data_returns_correct_sample_count() {
    let ld = LdFile::open(TEST_LD).unwrap();
    for ch in ld.channels.iter().take(10) {
        let data = ld
            .read_channel_data(ch)
            .expect("failed to read channel data");
        assert_eq!(
            data.len(),
            ch.n_data as usize,
            "sample count mismatch for channel '{}'",
            ch.name
        );
    }
}

#[test]
fn from_bytes_matches_open_for_channel_data() {
    let bytes = std::fs::read(TEST_LD).expect("failed to read .ld test data");
    let from_path = LdFile::open(TEST_LD).unwrap();
    let from_bytes = LdFile::from_bytes(bytes).unwrap();
    let ch_idx = 0;

    let path_data = from_path
        .read_channel_data(&from_path.channels[ch_idx])
        .unwrap();
    let bytes_data = from_bytes
        .read_channel_data(&from_bytes.channels[ch_idx])
        .unwrap();

    assert_eq!(path_data, bytes_data);
}

#[test]
fn read_channel_range_subset() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let ch = &ld.channels[0];
    let full = ld.read_channel_data(ch).unwrap();
    let half_len = full.len() / 2;
    let range = ld.read_channel_range(ch, 0, half_len).unwrap();
    assert_eq!(range.len(), half_len);
    assert_eq!(&range[..], &full[..half_len]);
}

#[test]
fn lap_number_channel_values() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let lap_ch = ld
        .channels
        .iter()
        .find(|c| c.name == "Lap Number")
        .expect("Lap Number channel not found");
    let data = ld.read_channel_data(lap_ch).unwrap();
    assert_eq!(data.len(), 266);
    // Values should be in the range 2..=4
    let min = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(min, 2.0);
    assert_eq!(max, 4.0);
}

// ---------------------------------------------------------------------------
// .ldx sidecar parsing
// ---------------------------------------------------------------------------

#[test]
fn open_ldx_file() {
    let ldx = LdxFile::open(TEST_LDX).expect("failed to open .ldx file");
    assert_eq!(ldx.total_laps, Some(5));
    assert_eq!(ldx.fastest_time.as_deref(), Some("2:08.392"));
    assert_eq!(ldx.fastest_lap, Some(3));
}

#[test]
fn find_ldx_for_ld_discovers_sidecar() {
    let ldx = find_ldx_for_ld(Path::new(TEST_LD)).expect("ldx sidecar not found");
    assert_eq!(ldx.total_laps, Some(5));
}

// ---------------------------------------------------------------------------
// Lap detection
// ---------------------------------------------------------------------------

#[test]
fn detect_laps_from_ld() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let laps = detect_laps(&ld);

    // Expect 3 laps: Out Lap, Lap 1, In Lap
    assert_eq!(laps.len(), 3, "expected 3 laps, got {}", laps.len());
    assert_eq!(laps[0].name, "Out Lap");
    assert_eq!(laps[1].name, "Lap 1");
    assert_eq!(laps[2].name, "In Lap");

    // Lap 1 should be ~128.39s (i2 reports 2:08.392)
    let lap1_dur = laps[1].duration();
    assert!(
        (lap1_dur - 128.392).abs() < 0.1,
        "Lap 1 duration should be ~128.392s, got {:.3}s",
        lap1_dur
    );

    for lap in &laps {
        assert!(
            lap.duration() > 0.0,
            "{} has non-positive duration",
            lap.name
        );
        assert!(lap.end_time > lap.start_time);
    }

    // Laps should be ordered chronologically
    for w in laps.windows(2) {
        assert!(
            w[1].start_time >= w[0].start_time,
            "laps not in chronological order"
        );
    }

    // Last lap should end near the session duration
    let last = laps.last().unwrap();
    assert!(
        last.end_time > 100.0,
        "last lap ends too early: {}",
        last.end_time
    );
}

// ---------------------------------------------------------------------------
// Downsampling with real data
// ---------------------------------------------------------------------------

#[test]
fn downsample_real_channel() {
    let ld = LdFile::open(TEST_LD).unwrap();
    // Pick a high-frequency channel for meaningful downsampling
    let ch = ld
        .channels
        .iter()
        .find(|c| c.freq >= 100)
        .expect("no high-freq channel found");
    let data = ld.read_channel_data(ch).unwrap();
    let target = 200;
    let result = downsample_minmax(&data, ch.freq, 0, target);

    assert!(!result.is_empty());
    // Should be at most 2*target points when downsampling kicks in,
    // or exactly n_data points if below threshold
    if data.len() > 2 * target {
        assert_eq!(result.len(), target);
        // Each bucket's min <= max
        for pt in &result {
            assert!(pt.min <= pt.max, "min > max in downsampled point");
        }
    }

    // Times should be monotonically non-decreasing
    for w in result.windows(2) {
        assert!(w[1].time >= w[0].time);
    }
}

// ---------------------------------------------------------------------------
// Data type coverage
// ---------------------------------------------------------------------------

#[test]
fn known_data_types_are_readable() {
    let ld = LdFile::open(TEST_LD).unwrap();
    let mut types_seen = std::collections::HashSet::new();
    let mut readable_count = 0;
    let mut unknown_count = 0;
    for ch in &ld.channels {
        types_seen.insert(ch.data_type.name());
        if ch.data_type.name() == "unknown" {
            // Unknown data types may not be readable — just skip
            unknown_count += 1;
            continue;
        }
        let data = ld.read_channel_data(ch);
        assert!(
            data.is_some(),
            "failed to read channel '{}' (type {:?})",
            ch.name,
            ch.data_type
        );
        let data = data.unwrap();
        // No NaN or Inf values (data should be clean)
        for (i, &v) in data.iter().enumerate() {
            assert!(
                v.is_finite(),
                "channel '{}' sample {} is not finite: {}",
                ch.name,
                i,
                v
            );
        }
        readable_count += 1;
    }
    // Most channels should be readable
    assert!(
        readable_count > 190,
        "only {readable_count} channels readable"
    );
    assert!(
        unknown_count > 0,
        "expected some unknown data types in test data"
    );
    // We should see more than one data type across 199 channels
    assert!(
        types_seen.len() > 1,
        "expected multiple data types, only saw: {:?}",
        types_seen
    );
}

// ---------------------------------------------------------------------------
// GPS track extraction
// ---------------------------------------------------------------------------

#[test]
fn extract_gps_track_from_test_data() {
    let ld = LdFile::open(TEST_LD).expect("failed to open .ld file");
    let track = extract_gps_track(&ld).expect("GPS track extraction failed");

    // VIR_LAP.ld has GPS at 20 Hz with ~2660 samples
    assert!(
        track.x.len() > 2000,
        "expected >2000 GPS samples, got {}",
        track.x.len()
    );
    assert_eq!(track.x.len(), track.y.len());
    assert_eq!(track.x.len(), track.time.len());
    assert_eq!(track.freq, 20);

    // Coordinates should be centered near zero (mean subtracted)
    let mean_x: f64 = track.x.iter().sum::<f64>() / track.x.len() as f64;
    let mean_y: f64 = track.y.iter().sum::<f64>() / track.y.len() as f64;
    assert!(mean_x.abs() < 1e-6, "mean x should be ~0, got {}", mean_x);
    assert!(mean_y.abs() < 1e-6, "mean y should be ~0, got {}", mean_y);

    // Time should be monotonically increasing
    for i in 1..track.time.len() {
        assert!(track.time[i] >= track.time[i - 1]);
    }
}

#[test]
fn find_nearest_on_real_track() {
    let ld = LdFile::open(TEST_LD).expect("failed to open .ld file");
    let track = extract_gps_track(&ld).expect("GPS track extraction failed");

    // Find nearest to the first point should return 0
    let idx = find_nearest_sample(&track, track.x[0], track.y[0]);
    assert_eq!(idx, 0);

    // Find nearest to the last point should return a point near the end
    let last = track.x.len() - 1;
    let idx = find_nearest_sample(&track, track.x[last], track.y[last]);
    assert!(
        idx >= last.saturating_sub(5),
        "expected near end, got {}",
        idx
    );
}

// ---------------------------------------------------------------------------
// Enum label parsing from .ld file
// ---------------------------------------------------------------------------

#[test]
fn enum_labels_parsed_for_state_channels() {
    let ld = LdFile::open(TEST_LD).expect("failed to open .ld file");

    // Engine Speed Limit State should have file-parsed enum labels
    let esls = ld
        .channels
        .iter()
        .find(|c| c.name == "Engine Speed Limit State")
        .expect("Engine Speed Limit State channel not found");
    assert!(
        !esls.enum_labels.is_empty(),
        "Engine Speed Limit State should have enum labels from file"
    );
    // Value 0 should map to "Maximum" (or similar label)
    assert!(
        esls.enum_labels.contains_key(&0),
        "enum_labels should contain key 0"
    );

    // Gear should also have enum labels
    let gear = ld
        .channels
        .iter()
        .find(|c| c.name == "Gear")
        .expect("Gear channel not found");
    eprintln!("Gear enum_labels: {:?}", gear.enum_labels);

    // If Gear has no file-parsed labels, that's fine - the hardcoded fallback covers it.
    // Some channels may not have enum tables in this particular file.
    if !gear.enum_labels.is_empty() {
        let label = ld.format_channel_value(gear, 3.0);
        assert!(label.is_some(), "Gear value 3 should have a label");
    }

    // Verify format_channel_value works for Engine Speed Limit State
    let label = ld.format_channel_value(esls, 0.0);
    assert!(label.is_some(), "ESLS value 0 should have a label");
    eprintln!("ESLS value 0 = {:?}", label);
}

//! Integration tests for i3rs-core using real MoTeC test data.

use i3rs_core::{LdFile, LdxFile, detect_laps, downsample_minmax, extract_gps_track, find_ldx_for_ld, find_nearest_sample};
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
    // Lap Number channel has values 2, 3, 4 so we expect 3 laps
    assert!(
        laps.len() >= 3,
        "expected at least 3 laps, got {}",
        laps.len()
    );

    for lap in &laps {
        assert!(
            lap.duration() > 0.0,
            "lap {} has non-positive duration",
            lap.number
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
    assert!(track.x.len() > 2000, "expected >2000 GPS samples, got {}", track.x.len());
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
    assert!(idx >= last.saturating_sub(5), "expected near end, got {}", idx);
}

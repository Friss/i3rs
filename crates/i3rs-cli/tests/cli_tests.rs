//! Integration tests for the i3rs binary using real test data.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const TEST_LD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ld");
const TEST_S1_20260415_LD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test_data/S1_#28299_20260415_110834_2.ld"
);

fn run_cli(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_i3rs");
    Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute i3rs")
}

fn run_cli_in_dir(args: &[&str], cwd: &PathBuf) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_i3rs");
    Command::new(bin)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to execute i3rs")
}

fn temp_skill_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("i3rs-skill-test-{}-{nanos}", std::process::id()))
}

fn assert_agent_skill_files_exist(dir: &Path) {
    for skill_path in [
        dir.join(".agents")
            .join("skills")
            .join("i3rs")
            .join("SKILL.md"),
        dir.join(".claude")
            .join("skills")
            .join("i3rs")
            .join("SKILL.md"),
    ] {
        let skill = fs::read_to_string(&skill_path).expect("skill file should be written");
        assert!(skill.contains("name: i3rs"));
        assert!(skill.contains("i3rs <file.ld>"));
        assert!(skill.contains("install-skill"));
        assert!(skill.contains("compare-laps"));
    }
}

#[test]
fn cli_exits_with_error_when_no_args() {
    let output = run_cli(&[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage"),
        "expected usage message, got: {stderr}"
    );
}

#[test]
fn cli_help_command_prints_usage() {
    let output = run_cli(&["help"]);
    assert!(
        output.status.success(),
        "help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("<file.ld>"));
    assert!(stdout.contains("install-skill"));
    assert!(stdout.contains("compare-laps"));
}

#[test]
fn cli_install_skill_writes_skill_files_for_agent_roots() {
    let dir = temp_skill_dir();
    let dir_arg = dir.to_string_lossy().into_owned();

    let output = run_cli(&["install-skill", &dir_arg]);
    assert!(
        output.status.success(),
        "install-skill failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_agent_skill_files_exist(&dir);

    fs::remove_dir_all(dir).expect("temporary skill directory should be removable");
}

#[test]
fn cli_install_skill_defaults_to_current_working_directory() {
    let dir = temp_skill_dir();
    fs::create_dir_all(&dir).expect("temporary skill directory should be creatable");

    let output = run_cli_in_dir(&["install-skill"], &dir);
    assert!(
        output.status.success(),
        "install-skill failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_agent_skill_files_exist(&dir);

    fs::remove_dir_all(dir).expect("temporary skill directory should be removable");
}

#[test]
fn cli_summary_json_is_machine_readable() {
    let output = run_cli(&["summary", TEST_LD, "--format", "json"]);
    assert!(
        output.status.success(),
        "summary json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    assert_eq!(value["schema_version"], "i3rs-cli.analysis.v1");
    assert_eq!(value["command"], "summary");
    assert_eq!(value["channel_count"], 199);
    assert_eq!(value["lap_count"], 3);
    assert_eq!(value["session"]["vehicle_id"], "EVORA_Friss");
}

#[test]
fn cli_channels_json_supports_filtering() {
    let output = run_cli(&[
        "channels", TEST_LD, "--filter", "engine", "--format", "json",
    ]);
    assert!(
        output.status.success(),
        "channels json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let channels = value["channels"]
        .as_array()
        .expect("channels should be array");
    assert!(!channels.is_empty());
    assert!(channels.iter().any(|ch| ch["name"] == "Engine Speed"));
}

#[test]
#[ignore = "requires local S1 validation fixture that is not checked in"]
fn cli_channels_json_includes_warning_source_enum_fallback() {
    let output = run_cli(&[
        "channels",
        TEST_S1_20260415_LD,
        "--filter",
        "Warning Source",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "channels json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let channel = &value["channels"][0];
    assert_eq!(channel["name"], "Warning Source");
    assert_eq!(channel["enum_labels"]["0"], "None");
    assert_eq!(channel["enum_labels"]["1"], "Engine Oil Pressure Warning");
}

#[test]
#[ignore = "requires local S1 validation fixture that is not checked in"]
fn cli_extract_preserves_native_samples_for_enum_channels() {
    let output = run_cli(&[
        "extract",
        TEST_S1_20260415_LD,
        "--channel",
        "Warning Source",
        "--time",
        "1823:1825",
        "--resample",
        "points:2",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "extract json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let channel = &value["segments"][0]["channels"][0];
    let samples = channel["samples"]
        .as_array()
        .expect("samples should be array");

    assert_eq!(channel["effective_resample"], "native");
    assert!(
        samples.len() > 2,
        "enum channel should not be reduced to requested point count"
    );
    assert!(samples.iter().any(|sample| sample["value"] == 1.0));
    assert_eq!(value["warnings"][0]["code"], "discrete_native_samples");
}

#[test]
fn cli_laps_json_reports_detected_laps() {
    let output = run_cli(&["laps", TEST_LD, "--format", "json"]);
    assert!(
        output.status.success(),
        "laps json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let laps = value["laps"].as_array().expect("laps should be array");
    assert_eq!(laps.len(), 3);
    assert_eq!(laps[1]["name"], "Lap 1");
    assert!(laps[1]["duration"].as_f64().unwrap() > 120.0);
}

#[test]
fn cli_extract_csv_supports_lap_aligned_points() {
    let output = run_cli(&[
        "extract",
        TEST_LD,
        "--channel",
        "Engine Speed",
        "--lap",
        "Lap 1",
        "--x",
        "lap-percent",
        "--resample",
        "points:5",
        "--format",
        "csv",
    ]);
    assert!(
        output.status.success(),
        "extract csv failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "segment,lap_number,channel,x,absolute_time,value");
    assert_eq!(lines.len(), 6);
    assert!(lines[1].contains("Lap 1,1,Engine Speed,0.000000000"));
    assert!(lines[5].contains("100.000000000"));
}

#[test]
fn cli_stats_json_reports_per_lap_counts() {
    let output = run_cli(&[
        "stats",
        TEST_LD,
        "--channel",
        "Engine Speed",
        "--group",
        "lap",
        "--metrics",
        "min,max,mean,stddev",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "stats json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let results = value["results"]
        .as_array()
        .expect("results should be array");
    assert_eq!(results.len(), 3);
    assert_eq!(results[1]["lap"]["name"], "Lap 1");
    assert!(results[1]["stats"]["finite_count"].as_u64().unwrap() > 1000);
    assert!(results[1]["stats"]["metrics"]["mean"].as_f64().unwrap() > 0.0);
}

#[test]
fn cli_compare_laps_json_reports_delta_metrics() {
    let output = run_cli(&[
        "compare-laps",
        TEST_LD,
        "--channel",
        "Engine Speed",
        "--laps",
        "Lap 1,In Lap",
        "--reference",
        "Lap 1",
        "--x",
        "lap-percent",
        "--resample",
        "points:20",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "compare-laps json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let results = value["results"]
        .as_array()
        .expect("results should be array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["comparison_lap"]["name"], "In Lap");
    assert_eq!(results[0]["metrics"]["point_count"], 20);
    assert!(results[0]["metrics"]["finite_pair_count"].as_u64().unwrap() > 0);
}

#[test]
fn cli_run_executes_repeatable_analysis_spec() {
    let dir = temp_skill_dir();
    fs::create_dir_all(&dir).expect("temporary spec directory should be creatable");
    let spec_path = dir.join("analysis.json");
    fs::write(
        &spec_path,
        format!(
            r#"{{
  "file": "{}",
  "operations": [
    {{ "type": "summary" }},
    {{ "type": "extract", "channels": ["Engine Speed"], "lap": "Lap 1", "x": "lap-percent", "resample": "points:3" }},
    {{ "type": "compare-laps", "channels": ["Engine Speed"], "laps": "Lap 1,In Lap", "reference": "Lap 1", "resample": "points:3" }}
  ]
}}"#,
            TEST_LD.replace('\\', "\\\\")
        ),
    )
    .expect("spec should be writable");

    let spec_arg = spec_path.to_string_lossy().into_owned();
    let output = run_cli(&["run", &spec_arg, "--format", "json"]);
    assert!(
        output.status.success(),
        "run json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
    let results = value["results"]
        .as_array()
        .expect("results should be array");
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["command"], "summary");
    assert_eq!(results[1]["command"], "extract");
    assert_eq!(results[2]["command"], "compare-laps");

    fs::remove_dir_all(dir).expect("temporary spec directory should be removable");
}

#[test]
fn cli_exits_with_error_for_missing_file() {
    let output = run_cli(&["nonexistent.ld"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error"),
        "expected error message, got: {stderr}"
    );
}

#[test]
fn cli_parses_test_file_successfully() {
    let output = run_cli(&[TEST_LD]);
    assert!(
        output.status.success(),
        "cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    // File info
    assert!(stdout.contains("4998791 bytes"));

    // Session metadata
    assert!(stdout.contains("24/09/2025"));
    assert!(stdout.contains("16:23:57"));
    assert!(stdout.contains("EVORA_Friss"));
    assert!(stdout.contains("VIR Full"));
    assert!(stdout.contains("4th session"));
    assert!(stdout.contains("M1"));
    assert!(stdout.contains("28299"));

    // Channel summary
    assert!(stdout.contains("Total channels: 199"));

    // Spot-check a known channel name appears in output
    assert!(stdout.contains("Lap Number"));
}

#[test]
fn cli_output_contains_channel_stats() {
    let output = run_cli(&[TEST_LD]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The header row should be present
    assert!(stdout.contains("Channel Name"));
    assert!(stdout.contains("Unit"));
    assert!(stdout.contains("Hz"));
    assert!(stdout.contains("Samples"));
    assert!(stdout.contains("Min"));
    assert!(stdout.contains("Max"));
    assert!(stdout.contains("Mean"));

    // Sample rate breakdown should be present
    assert!(stdout.contains("By sample rate:"));
}

//! Integration tests for the i3rs-cli binary using real test data.

use std::process::Command;

const TEST_LD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_data/VIR_LAP.ld");

fn run_cli(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_i3rs-cli");
    Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute i3rs-cli")
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

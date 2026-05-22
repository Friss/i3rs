//! Golden regression tests for the motorcycle chassis geometry solver.
//!
//! These tests load a MotoSPEC XML chassis definition file and compare the
//! Rust solver output against a 200-row golden reference CSV exported directly
//! from MotoSPEC.  The CSV covers a wide range of suspension-pot readings and
//! lean angles, providing confidence that the ported geometry calculations
//! remain behaviourally correct.
//!
//! Fixture files live in `test_data/chassis/` at the workspace root:
//! - `2024-S1000RR.xml`               — MotoSPEC XML chassis definition
//! - `MotoSPEC-Model-Verification.csv` — 200-row golden reference

use i3rs_core::{ChassisSolver, FrameState, parse_chassis_xml};
use std::path::Path;

// ---------------------------------------------------------------------------
// Fixture paths (relative to the crate manifest, two levels up to workspace)
// ---------------------------------------------------------------------------

const XML_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test_data/chassis/2024-S1000RR.xml"
);
const CSV_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test_data/chassis/MotoSPEC-Model-Verification.csv"
);

// ---------------------------------------------------------------------------
// Tolerances — mirrored exactly from the MotoSim parity harness
// ---------------------------------------------------------------------------

const TOL_WHEELBASE: f64 = 0.2;
const TOL_RAKE: f64 = 0.02;
const TOL_TRAIL: f64 = 0.2;       // both NormalTrail and GroundTrail
const TOL_RIDE_HEIGHT: f64 = 0.2;
const TOL_SWINGARM_ANGLE: f64 = 0.02;
const TOL_ANTI_SQUAT_PCT: f64 = 0.5;
const TOL_ANTI_SQUAT_ANGLE: f64 = 0.06;
const TOL_LOAD_TRANSFER_ANGLE: f64 = 0.065;
const TOL_COG_PCT: f64 = 0.4;     // both CoGFront and CoGRear
const TOL_RR_WHEEL_FORCE: f64 = 1.0;
const TOL_RR_WHEEL_RATE: f64 = 0.1;
const TOL_FR_WHEEL_FORCE: f64 = 4.0;
const TOL_FR_WHEEL_RATE: f64 = 0.5;

// ---------------------------------------------------------------------------
// CSV row type
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct GoldenRow {
    index: usize,
    /// MoTeC front-suspension pot reading (mm)
    fr_pot: f64,
    /// MoTeC rear-suspension pot reading (mm)
    rr_pot: f64,
    lean_deg: f64,
    // --- expected outputs ---
    rake: f64,
    normal_trail: f64,
    ground_trail: f64,
    ride_height: f64,
    wheelbase: f64,
    anti_squat_pct: f64,
    anti_squat_angle: f64,
    load_transfer_angle: f64,
    swingarm_angle: f64,
    fr_wheel_force: f64,
    rr_wheel_force: f64,
    fr_wheel_rate: f64,
    rr_wheel_rate: f64,
    cog_front: f64,
    cog_rear: f64,
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Build a solver from the XML fixture — the same file the MotoSim C# tests use.
fn load_solver() -> ChassisSolver {
    let path = Path::new(XML_PATH);
    assert!(path.exists(), "XML chassis fixture not found: {XML_PATH}");
    let model = parse_chassis_xml(path).expect("Failed to parse XML chassis fixture");
    ChassisSolver::prepare(model)
}

/// Parse the golden verification CSV, returning one `GoldenRow` per data row.
fn load_golden_rows() -> Vec<GoldenRow> {
    let csv = std::fs::read_to_string(CSV_PATH)
        .unwrap_or_else(|e| panic!("Cannot read golden CSV {CSV_PATH}: {e}"));

    let mut rows = Vec::new();
    for (line_no, line) in csv.lines().enumerate() {
        if line_no == 0 || line.trim().is_empty() {
            continue; // skip header and blank lines
        }
        let parts: Vec<&str> = line.split(',').collect();
        assert!(
            parts.len() >= 18,
            "Line {}: expected 18 columns, got {}",
            line_no + 1,
            parts.len()
        );
        let p = |i: usize| -> f64 {
            parts[i].trim().parse::<f64>()
                .unwrap_or_else(|e| panic!("Line {}, col {}: parse error: {e}", line_no + 1, i))
        };
        rows.push(GoldenRow {
            index: rows.len(),
            fr_pot: p(0),
            rr_pot: p(1),
            lean_deg: p(2),
            rake: p(3),
            normal_trail: p(4),
            ground_trail: p(5),
            ride_height: p(6),
            wheelbase: p(7),
            anti_squat_pct: p(8),
            anti_squat_angle: p(9),
            load_transfer_angle: p(10),
            swingarm_angle: p(11),
            fr_wheel_force: p(12),
            rr_wheel_force: p(13),
            fr_wheel_rate: p(14),
            rr_wheel_rate: p(15),
            cog_front: p(16),
            cog_rear: p(17),
        });
    }
    rows
}

/// Assert that a single solver output is within tolerance.
fn assert_near(label: &str, row_idx: usize, expected: f64, actual: f64, tol: f64) {
    let err = (actual - expected).abs();
    assert!(
        err <= tol,
        "row {row_idx}: {label}: expected={expected:.4} actual={actual:.4} |err|={err:.4} tol={tol}"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that the fixture files can be found, a solver is built from the XML,
/// and the CSV has at least 190 data rows — matches the MotoSim fixture check.
#[test]
fn fixtures_load_xml_and_csv() {
    let solver = load_solver();
    // Sanity-solve at zero suspension / zero lean — should produce plausible geometry.
    let state = solver.solve(0.0, 0.0, 0.0);
    assert!(
        state.wheelbase_mm > 1300.0 && state.wheelbase_mm < 1600.0,
        "Design-pose wheelbase out of range: {:.1}",
        state.wheelbase_mm
    );

    let rows = load_golden_rows();
    assert!(
        rows.len() >= 190,
        "Expected at least 190 golden rows, got {}",
        rows.len()
    );
}

/// Hardcoded smoke-test at fr=72 mm, rr=22.6 mm, lean=45.6° — compares against
/// known oracle values taken directly from MotoSPEC at this operating point.
#[test]
fn smoke_triplet_fr72_rr22p6_lean45p6() {
    let solver = load_solver();
    // Note: solver takes (rr_pot, fr_pot, lean_deg)
    let s = solver.solve(22.6, 72.0, 45.6);

    assert_near("Rake",             0, 22.59,  s.rake_deg,              TOL_RAKE);
    assert_near("NormalTrail",      0, 83.7,   s.trail_mm,              TOL_TRAIL);
    assert_near("GroundTrail",      0, 90.7,   s.ground_trail_mm,       TOL_TRAIL);
    assert_near("RideHeight",       0, -74.6,  s.inst_ride_ht_mm,       TOL_RIDE_HEIGHT);
    assert_near("Wheelbase",        0, 1460.2, s.wheelbase_mm,          TOL_WHEELBASE);
    assert_near("SwingarmAngle",    0, -6.84,  s.inst_sw_angle_deg,     TOL_SWINGARM_ANGLE);
    assert_near("AntiSquatPercent", 0, 98.6,   s.anti_squat_pct,        TOL_ANTI_SQUAT_PCT);
    assert_near("FrontWheelForce",  0, 1986.1, s.fr_wheel_force_n,      TOL_FR_WHEEL_FORCE);
    assert_near("RearWheelForce",   0, 1667.3, s.rr_wheel_force_n,      TOL_RR_WHEEL_FORCE);
    assert_near("FrontWheelRate",   0, 28.34,  s.fr_wheel_rate_n_per_mm, TOL_FR_WHEEL_RATE);
    assert_near("RearWheelRate",    0, 28.48,  s.rr_wheel_rate_n_per_mm, TOL_RR_WHEEL_RATE);
}

/// All 200 golden rows — geometry group (Rake, Trail, GroundTrail, RideHeight,
/// Wheelbase, SwingarmAngle) within tolerance.
#[test]
fn all_rows_geometry_group_within_tolerance() {
    let solver = load_solver();
    let rows = load_golden_rows();
    let mut failures = Vec::new();

    for row in &rows {
        let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);
        let checks = [
            ("Rake",          row.rake,          s.rake_deg,          TOL_RAKE),
            ("NormalTrail",   row.normal_trail,  s.trail_mm,          TOL_TRAIL),
            ("GroundTrail",   row.ground_trail,  s.ground_trail_mm,   TOL_TRAIL),
            ("RideHeight",    row.ride_height,   s.inst_ride_ht_mm,   TOL_RIDE_HEIGHT),
            ("Wheelbase",     row.wheelbase,     s.wheelbase_mm,      TOL_WHEELBASE),
            ("SwingarmAngle", row.swingarm_angle, s.inst_sw_angle_deg, TOL_SWINGARM_ANGLE),
        ];
        for (label, exp, act, tol) in checks {
            let err = (act - exp).abs();
            if err > tol {
                failures.push(format!(
                    "row {}: {} expected={:.4} actual={:.4} |err|={:.4} tol={}",
                    row.index, label, exp, act, err, tol
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} geometry failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// All 200 golden rows — anti-squat / load-transfer group within tolerance.
#[test]
fn all_rows_anti_squat_lt_group_within_tolerance() {
    let solver = load_solver();
    let rows = load_golden_rows();
    let mut failures = Vec::new();

    for row in &rows {
        let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);
        let checks = [
            ("AntiSquatPercent",   row.anti_squat_pct,        s.anti_squat_pct,         TOL_ANTI_SQUAT_PCT),
            ("AntiSquatSAAngle",   row.anti_squat_angle,      s.anti_squat_angle_deg,   TOL_ANTI_SQUAT_ANGLE),
            ("LoadTransferAngle",  row.load_transfer_angle,   s.load_transfer_angle_deg, TOL_LOAD_TRANSFER_ANGLE),
        ];
        for (label, exp, act, tol) in checks {
            let err = (act - exp).abs();
            if err > tol {
                failures.push(format!(
                    "row {}: {} expected={:.4} actual={:.4} |err|={:.4} tol={}",
                    row.index, label, exp, act, err, tol
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} anti-squat/LT failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// All 200 golden rows — wheel force and rate group within tolerance.
#[test]
fn all_rows_wheel_fk_group_within_tolerance() {
    let solver = load_solver();
    let rows = load_golden_rows();
    let mut failures = Vec::new();

    for row in &rows {
        let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);
        let checks = [
            ("FrontWheelForce", row.fr_wheel_force, s.fr_wheel_force_n,       TOL_FR_WHEEL_FORCE),
            ("RearWheelForce",  row.rr_wheel_force, s.rr_wheel_force_n,       TOL_RR_WHEEL_FORCE),
            ("FrontWheelRate",  row.fr_wheel_rate,  s.fr_wheel_rate_n_per_mm, TOL_FR_WHEEL_RATE),
            ("RearWheelRate",   row.rr_wheel_rate,  s.rr_wheel_rate_n_per_mm, TOL_RR_WHEEL_RATE),
        ];
        for (label, exp, act, tol) in checks {
            let err = (act - exp).abs();
            if err > tol {
                failures.push(format!(
                    "row {}: {} expected={:.4} actual={:.4} |err|={:.4} tol={}",
                    row.index, label, exp, act, err, tol
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} wheel F/k failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// All 200 golden rows — centre-of-gravity split within tolerance.
#[test]
fn all_rows_cog_split_within_tolerance() {
    let solver = load_solver();
    let rows = load_golden_rows();
    let mut failures = Vec::new();

    for row in &rows {
        let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);

        // CoG percentages must sum to 100 %
        let sum = s.cog_percent_front + s.cog_percent_rear;
        if (sum - 100.0).abs() > 0.01 {
            failures.push(format!(
                "row {}: CoG sum = {:.3} (expected 100.0)",
                row.index, sum
            ));
        }

        let checks = [
            ("CoGPercentFront", row.cog_front, s.cog_percent_front, TOL_COG_PCT),
            ("CoGPercentRear",  row.cog_rear,  s.cog_percent_rear,  TOL_COG_PCT),
        ];
        for (label, exp, act, tol) in checks {
            let err = (act - exp).abs();
            if err > tol {
                failures.push(format!(
                    "row {}: {} expected={:.4} actual={:.4} |err|={:.4} tol={}",
                    row.index, label, exp, act, err, tol
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} CoG failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// All 200 golden rows — every mapped column within tolerance (omnibus test).
///
/// This is the primary regression gate: a single failure here means the
/// solver has drifted from MotoSPEC for at least one field at one operating
/// point.
#[test]
fn all_rows_all_fields_within_tolerance() {
    let solver = load_solver();
    let rows = load_golden_rows();
    let mut failures = Vec::new();

    for row in &rows {
        let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);
        collect_row_failures(&s, row, &mut failures);
    }

    if !failures.is_empty() {
        // Print a compact parity report before asserting so failures are
        // visible in full even when only the assert message is shown.
        eprintln!(
            "\nMotoSPEC parity failures ({} total):\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    assert!(
        failures.is_empty(),
        "{} total field failures across all rows (see stderr for details)",
        failures.len()
    );
}

/// Spot-check: the golden CSV row nearest to the standard smoke triplet
/// (fr≈72, rr≈25, lean≈43°) — row index 43 in the CSV.
#[test]
fn golden_row_44_fr72p98_rr25p42_lean43p1() {
    let solver = load_solver();
    let rows = load_golden_rows();

    // Row index 43 (0-based): fr=72.98, rr=25.42, lean=43.1
    let row = rows.iter().find(|r| r.index == 43)
        .expect("Row 43 not found in golden CSV");

    let s = solver.solve(row.rr_pot, row.fr_pot, row.lean_deg);
    let mut failures = Vec::new();
    collect_row_failures(&s, row, &mut failures);

    assert!(
        failures.is_empty(),
        "Row 43 failures:\n  {}",
        failures.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Private helper
// ---------------------------------------------------------------------------

fn collect_row_failures(s: &FrameState, row: &GoldenRow, out: &mut Vec<String>) {
    let checks: &[(&str, f64, f64, f64)] = &[
        ("Rake",              row.rake,               s.rake_deg,               TOL_RAKE),
        ("NormalTrail",       row.normal_trail,       s.trail_mm,               TOL_TRAIL),
        ("GroundTrail",       row.ground_trail,       s.ground_trail_mm,        TOL_TRAIL),
        ("RideHeight",        row.ride_height,        s.inst_ride_ht_mm,        TOL_RIDE_HEIGHT),
        ("Wheelbase",         row.wheelbase,          s.wheelbase_mm,           TOL_WHEELBASE),
        ("SwingarmAngle",     row.swingarm_angle,     s.inst_sw_angle_deg,      TOL_SWINGARM_ANGLE),
        ("AntiSquatPercent",  row.anti_squat_pct,     s.anti_squat_pct,         TOL_ANTI_SQUAT_PCT),
        ("AntiSquatSAAngle",  row.anti_squat_angle,   s.anti_squat_angle_deg,   TOL_ANTI_SQUAT_ANGLE),
        ("LoadTransferAngle", row.load_transfer_angle, s.load_transfer_angle_deg, TOL_LOAD_TRANSFER_ANGLE),
        ("FrontWheelForce",   row.fr_wheel_force,     s.fr_wheel_force_n,       TOL_FR_WHEEL_FORCE),
        ("RearWheelForce",    row.rr_wheel_force,     s.rr_wheel_force_n,       TOL_RR_WHEEL_FORCE),
        ("FrontWheelRate",    row.fr_wheel_rate,      s.fr_wheel_rate_n_per_mm, TOL_FR_WHEEL_RATE),
        ("RearWheelRate",     row.rr_wheel_rate,      s.rr_wheel_rate_n_per_mm, TOL_RR_WHEEL_RATE),
        ("CoGPercentFront",   row.cog_front,          s.cog_percent_front,      TOL_COG_PCT),
        ("CoGPercentRear",    row.cog_rear,           s.cog_percent_rear,       TOL_COG_PCT),
    ];
    for &(label, exp, act, tol) in checks {
        let err = (act - exp).abs();
        if err > tol {
            out.push(format!(
                "row {}: {} expected={:.4} actual={:.4} |err|={:.4} tol={}",
                row.index, label, exp, act, err, tol
            ));
        }
    }
}

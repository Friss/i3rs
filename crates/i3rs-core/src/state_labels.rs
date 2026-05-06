//! Text labels for MoTeC M1 ECU state/enum channels.
//!
//! Channels like Gear, Brake State, and Engine Speed Limit State store
//! numeric values that correspond to human-readable labels. This module
//! provides the mapping from (channel_name, numeric_value) → text label.

use std::collections::HashMap;

/// Returns a text label for a known state channel value, or `None` if
/// the channel is not a recognized state channel.
///
/// For recognized channels with an unmapped numeric value, returns
/// the raw integer as a string (e.g. "34").
pub fn format_state_value(channel_name: &str, value: f64) -> Option<String> {
    let v = value.round() as i64;
    let name = channel_name.trim();

    let label = match_channel(name, v)?;
    Some(label)
}

/// Returns `true` if the channel name is a known state/enum channel.
pub fn is_state_channel(channel_name: &str) -> bool {
    let name = channel_name.trim();
    const STATE_CHANNELS: &[&str] = &[
        "gear",
        "gear estimate",
        "brake state",
        "clutch state",
        "engine state",
        "engine speed limit state",
        "torque limit state",
        "warning source",
        "launch state",
        "cruise state",
        "traction state",
        "idle state",
        "torque control state",
        "lap state",
    ];
    STATE_CHANNELS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// Returns known labels for channels where some files do not include an enum
/// table reference in the channel metadata.
pub fn state_channel_labels(channel_name: &str) -> Option<HashMap<i64, String>> {
    let name = channel_name.trim();

    if name.eq_ignore_ascii_case("warning source") {
        return Some(labels(&[
            (0, "None"),
            (1, "Engine Oil Pressure Warning"),
            (2, "Knock Warning"),
            (3, "Engine Crankcase Pressure Warning"),
            (4, "Fuel Direct Injection Bank 1 Warning"),
            (5, "Fuel Direct Injection Bank 2 Warning"),
            (6, "Fuel Pressure Warning"),
            (7, "Alternative Fuel Pressure Warning"),
            (8, "Fuel Injector Primary Duty Cycle Warning"),
            (9, "Fuel Injector Secondary Duty Cycle Warning"),
            (10, "Boost Pressure Warning"),
            (11, "Engine Oil Level Warning"),
            (12, "Coolant Temperature Warning"),
            (13, "Coolant Temperature 2 Warning"),
            (14, "Inlet Air Temperature Warning"),
            (15, "Engine Oil Temperature Warning"),
            (16, "Transmission Temperature Warning"),
            (17, "Transmission Pressure Warning"),
            (18, "Exhaust Lambda Warning"),
            (19, "Exhaust Temperature Warning"),
            (20, "Coolant Pressure Warning"),
            (21, "Engine Speed Warning"),
            (22, "Gear Slip Warning"),
            (23, "Transmission Torque Convertor Slip Warning"),
            (24, "Nitrous Bottle Pressure Warning"),
            (25, "ECU Battery Diagnostic"),
            (26, "Inlet Manifold Pressure Sensor Diagnostic"),
            (27, "Engine Oil Pressure Sensor Diagnostic"),
            (28, "Coolant Temperature Sensor Diagnostic"),
            (29, "Coolant Temperature 2 Sensor Diagnostic"),
            (30, "Fuel Pressure Direct Bank 1 Sensor Diagnostic"),
            (31, "Fuel Pressure Direct Bank 2 Sensor Diagnostic"),
            (32, "Fuel Pressure Sensor Diagnostic"),
            (33, "Brake Vacuum Pressure Sensor Diagnostic"),
            (34, "Inlet Air Temperature Sensor Diagnostic"),
            (35, "Gear Position Sensor Diagnostic"),
            (36, "Gear Paddle Diagnostic"),
            (37, "Gear Lever Diagnostic"),
            (38, "Engine Crankcase Pressure Sensor Diagnostic"),
            (39, "Engine Oil Temperature Sensor Diagnostic"),
            (40, "Exhaust Lambda Bank 1 Collector Diagnostic"),
            (41, "Exhaust Lambda Bank 2 Collector Diagnostic"),
            (42, "Exhaust Temperature Diagnostic"),
            (43, "Throttle Pedal Sensor Diagnostic"),
            (44, "Throttle Position Sensor Diagnostic"),
            (45, "Throttle Servo Bank 1 Position Sensor Diagnostic"),
            (46, "Throttle Servo Bank 2 Position Sensor Diagnostic"),
            (47, "Boost Pressure Sensor Diagnostic"),
            (48, "Fuel Composition Sensor Diagnostic"),
            (49, "Fuel Temperature Sensor Diagnostic"),
            (50, "Ambient Pressure Sensor Diagnostic"),
            (51, "Fuel Closed Loop Diagnostic"),
            (52, "Throttle Servo Bank 1 Diagnostic"),
            (53, "Throttle Servo Bank 2 Diagnostic"),
            (54, "Boost Control Diagnostic"),
            (55, "Coolant Pressure Sensor Diagnostic"),
            (56, "Inlet Camshaft Bank 1 Position Diagnostic"),
            (57, "Inlet Camshaft Bank 2 Position Diagnostic"),
            (58, "Exhaust Camshaft Bank 1 Position Diagnostic"),
            (59, "Exhaust Camshaft Bank 2 Position Diagnostic"),
            (60, "Fuel Cylinder 1 Primary Pin Diagnostic"),
            (61, "Fuel Cylinder 2 Primary Pin Diagnostic"),
            (62, "Fuel Cylinder 3 Primary Pin Diagnostic"),
            (63, "Fuel Cylinder 4 Primary Pin Diagnostic"),
            (64, "Fuel Cylinder 5 Primary Pin Diagnostic"),
            (65, "Fuel Cylinder 6 Primary Pin Diagnostic"),
            (66, "Fuel Cylinder 1 Secondary Pin Diagnostic"),
            (67, "Fuel Cylinder 2 Secondary Pin Diagnostic"),
            (68, "Fuel Cylinder 3 Secondary Pin Diagnostic"),
            (69, "Fuel Cylinder 4 Secondary Pin Diagnostic"),
            (70, "Fuel Cylinder 5 Secondary Pin Diagnostic"),
            (71, "Fuel Cylinder 6 Secondary Pin Diagnostic"),
            (72, "CAN Bus 1 Diagnostic"),
            (73, "CAN Bus 2 Diagnostic"),
            (74, "CAN Bus 3 Diagnostic"),
            (75, "RS232 Diagnostic"),
            (76, "Engine Speed Reference Diagnostic"),
            (77, "GPS Diagnostic"),
            (78, "Engine Speed Reference State"),
            (79, "Vehicle Acceleration Lateral Sensor Diagnostic"),
        ]));
    }

    None
}

fn labels(entries: &[(i64, &str)]) -> HashMap<i64, String> {
    entries
        .iter()
        .map(|(value, label)| (*value, (*label).to_string()))
        .collect()
}

fn match_channel(name: &str, v: i64) -> Option<String> {
    if name.eq_ignore_ascii_case("gear") || name.eq_ignore_ascii_case("gear estimate") {
        return Some(match v {
            -1 => "Reverse".into(),
            0 => "Neutral".into(),
            1 => "First".into(),
            2 => "Second".into(),
            3 => "Third".into(),
            4 => "Fourth".into(),
            5 => "Fifth".into(),
            6 => "Sixth".into(),
            7 => "Seventh".into(),
            8 => "Reverse".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("brake state") {
        return Some(match v {
            0 => "Unknown".into(),
            1 => "Off".into(),
            2 => "On".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("clutch state") {
        return Some(match v {
            0 => "Released".into(),
            1 => "Transitioning".into(),
            2 => "Pressed".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("engine state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Cranking".into(),
            2 => "Running".into(),
            3 => "Stall".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("engine speed limit state") {
        return Some(match v {
            0 => "Maximum".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("torque limit state") {
        return Some(match v {
            0 => "Maximum".into(),
            1 => "Driver Demand".into(),
            2 => "Engine Protection".into(),
            3 => "External".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("warning source") {
        let labels = state_channel_labels(name)?;
        return Some(labels.get(&v).cloned().unwrap_or_else(|| format!("{}", v)));
    }

    if name.eq_ignore_ascii_case("launch state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Armed".into(),
            2 => "Active".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("cruise state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Active".into(),
            2 => "Overridden".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("traction state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Active".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("idle state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Active".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("torque control state") {
        return Some(match v {
            0 => "Off".into(),
            1 => "Active".into(),
            other => format!("{}", other),
        });
    }

    if name.eq_ignore_ascii_case("lap state") {
        return Some(match v {
            0 => "Out Lap".into(),
            1 => "Flying".into(),
            2 => "In Lap".into(),
            3 => "Pit".into(),
            other => format!("{}", other),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gear_labels() {
        assert_eq!(format_state_value("Gear", 3.0), Some("Third".into()));
        assert_eq!(format_state_value("Gear", 0.0), Some("Neutral".into()));
        assert_eq!(format_state_value("Gear", 5.0), Some("Fifth".into()));
        assert_eq!(format_state_value("Gear", -1.0), Some("Reverse".into()));
    }

    #[test]
    fn test_brake_state() {
        assert_eq!(format_state_value("Brake State", 1.0), Some("Off".into()));
        assert_eq!(format_state_value("Brake State", 2.0), Some("On".into()));
    }

    #[test]
    fn test_engine_speed_limit_state() {
        assert_eq!(
            format_state_value("Engine Speed Limit State", 0.0),
            Some("Maximum".into())
        );
        assert_eq!(
            format_state_value("Engine Speed Limit State", 34.0),
            Some("34".into())
        );
    }

    #[test]
    fn test_warning_source() {
        assert_eq!(
            format_state_value("Warning Source", 0.0),
            Some("None".into())
        );
        assert_eq!(
            format_state_value("Warning Source", 1.0),
            Some("Engine Oil Pressure Warning".into())
        );
        assert_eq!(
            format_state_value("Warning Source", 14.0),
            Some("Inlet Air Temperature Warning".into())
        );
        assert_eq!(
            format_state_value("Warning Source", 99.0),
            Some("99".into())
        );

        let labels = state_channel_labels("Warning Source").expect("warning source labels");
        assert_eq!(
            labels.get(&1).map(String::as_str),
            Some("Engine Oil Pressure Warning")
        );
    }

    #[test]
    fn test_unknown_channel() {
        assert_eq!(format_state_value("Engine Speed", 4000.0), None);
    }

    #[test]
    fn test_is_state_channel() {
        assert!(is_state_channel("Gear"));
        assert!(is_state_channel("Brake State"));
        assert!(!is_state_channel("Engine Speed"));
        assert!(!is_state_channel("Throttle Position"));
    }
}

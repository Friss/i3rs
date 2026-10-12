//! Speed-derived distance shared by graphs and distance-defined track sections.
use crate::{LdFile, sample_at_time};

pub const SPEED_FALLBACKS: [&str; 7] = [
    "Wheel Speed Front Left",
    "Wheel Speed Front Right",
    "Vehicle Speed",
    "GPS Speed",
    "Ground Speed",
    "Speed",
    "Drive Speed",
];

/// Use the faster valid front wheel at each time, after converting both to m/s.
/// A missing sample in one wheel does not discard the other wheel's valid speed.
pub fn fastest_speed(a: (&[f64], u16, &str), b: (&[f64], u16, &str)) -> Option<(Vec<f64>, u16)> {
    let (sa, sb) = (speed_unit_to_mps(a.2)?, speed_unit_to_mps(b.2)?);
    let freq = a.1.max(b.1);
    if a.1 == 0 || b.1 == 0 {
        return None;
    }
    let duration = (a.0.len() as f64 / a.1 as f64).max(b.0.len() as f64 / b.1 as f64);
    let data = (0..(duration * freq as f64).ceil() as usize)
        .map(|i| {
            let t = i as f64 / freq as f64;
            let av = sample_at_time(a.0, a.1, t).map(|v| v * sa);
            let bv = sample_at_time(b.0, b.1, t).map(|v| v * sb);
            match (av, bv) {
                (Some(a), Some(b)) => a.max(b),
                (Some(v), None) | (None, Some(v)) => v,
                _ => f64::NAN,
            }
        })
        .collect();
    Some((data, freq))
}

/// Default Circuit speed source: an explicit corrected channel, fastest front
/// wheels, then a single available wheel/vehicle/GPS speed. Units are m/s.
pub fn corrected_speed_from_ld(ld: &LdFile) -> Option<(Vec<f64>, u16)> {
    let load = |name| {
        let c = ld.find_channel_by_name(name)?;
        Some((ld.read_channel_data(c)?, c.freq, c.unit.clone()))
    };
    let convert = |(d, f, u): (Vec<f64>, u16, String)| {
        let s = speed_unit_to_mps(&u)?;
        (f > 0).then(|| (d.into_iter().map(|v| v * s).collect(), f))
    };
    if let Some(v) = load("Corr Speed").and_then(convert) {
        return Some(v);
    }
    if let (Some(a), Some(b)) = (
        load("Wheel Speed Front Left"),
        load("Wheel Speed Front Right"),
    ) && let Some(v) = fastest_speed((&a.0, a.1, &a.2), (&b.0, b.1, &b.2))
    {
        return Some(v);
    }
    SPEED_FALLBACKS
        .into_iter()
        .find_map(|name| load(name).and_then(convert))
}

pub fn integrate_speed_series(data: &[f64], freq: u16, unit: &str) -> Option<Vec<f64>> {
    if freq == 0 {
        return None;
    }

    let to_mps = speed_unit_to_mps(unit)?;
    if data.is_empty() {
        return Some(Vec::new());
    }

    let dt = 1.0 / freq as f64;
    let mut result = Vec::with_capacity(data.len());
    let mut sum = 0.0;
    result.push(0.0);

    for i in 0..data.len() - 1 {
        let v0 = data[i] * to_mps;
        let v1 = data[i + 1] * to_mps;
        // Hold distance through a missing interval: inventing motion across an
        // outage is misleading, but a NaN must not poison the rest of the lap.
        if v0.is_finite() && v1.is_finite() {
            sum += (v0.max(0.0) + v1.max(0.0)) * 0.5 * dt;
        }
        result.push(sum);
    }

    Some(result)
}

pub fn speed_unit_to_mps(unit: &str) -> Option<f64> {
    let normalized = unit.to_ascii_lowercase().replace(' ', "");
    if normalized.contains("km/h") || normalized.contains("kmh") || normalized.contains("kph") {
        Some(1.0 / 3.6)
    } else if normalized.contains("mph") {
        Some(0.44704)
    } else if normalized.contains("m/s") || normalized.contains("mps") {
        Some(1.0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fastest_wheel_converts_units_and_preserves_one_valid_source() {
        let (speed, freq) = fastest_speed(
            (&[36., f64::NAN, 18.], 1, "km/h"),
            (&[5., 8., f64::NAN], 1, "m/s"),
        )
        .unwrap();
        assert_eq!(freq, 1);
        assert_eq!(speed, [10., 8., 5.]);
        assert_eq!(
            integrate_speed_series(&speed, 1, "m/s").unwrap(),
            [0., 9., 15.5]
        );
        assert!(fastest_speed((&[1.], 0, "km/h"), (&[1.], 1, "mph")).is_none());
        assert!(fastest_speed((&[1.], 1, "rpm"), (&[1.], 1, "mph")).is_none());
    }
}

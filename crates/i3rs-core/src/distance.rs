//! Speed-derived distance shared by graphs and distance-defined track sections.
use crate::{ChannelMeta, LdFile, sample_at_time};

pub const MPS_PER_MPH: f64 = 0.44704;

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

/// The channel(s) corrected speed is read from.
pub enum SpeedSource<'a> {
    Single(&'a ChannelMeta),
    /// The faster of the two front wheels at each time.
    FrontWheels(&'a ChannelMeta, &'a ChannelMeta),
}

/// Default Circuit speed source: an explicit corrected channel, fastest front
/// wheels, then a single available wheel/vehicle/GPS speed. Every selected
/// channel has decodable samples, a nonzero rate and a speed unit. All sessions (primary and
/// overlays, sync and async decode) select through here so they agree.
pub fn corrected_speed_source(ld: &LdFile) -> Option<SpeedSource<'_>> {
    let usable = |name| {
        ld.find_channel_by_name(name).filter(|c| {
            c.freq > 0
                && c.n_data > 0
                && c.data_type.bytes_per_sample().is_some()
                && speed_unit_to_mps(&c.unit).is_some()
        })
    };
    if let Some(c) = usable("Corr Speed") {
        return Some(SpeedSource::Single(c));
    }
    if let (Some(a), Some(b)) = (
        usable("Wheel Speed Front Left"),
        usable("Wheel Speed Front Right"),
    ) {
        return Some(SpeedSource::FrontWheels(a, b));
    }
    SPEED_FALLBACKS
        .into_iter()
        .find_map(usable)
        .map(SpeedSource::Single)
}

/// Corrected speed from [`corrected_speed_source`], in m/s.
pub fn corrected_speed_from_ld(ld: &LdFile) -> Option<(Vec<f64>, u16)> {
    match corrected_speed_source(ld)? {
        SpeedSource::Single(c) => {
            let scale = speed_unit_to_mps(&c.unit)?;
            let data = ld.read_channel_data(c)?;
            Some((data.into_iter().map(|v| v * scale).collect(), c.freq))
        }
        SpeedSource::FrontWheels(a, b) => fastest_speed(
            (&ld.read_channel_data(a)?, a.freq, &a.unit),
            (&ld.read_channel_data(b)?, b.freq, &b.unit),
        ),
    }
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
        Some(MPS_PER_MPH)
    } else if normalized.contains("m/s") || normalized.contains("mps") {
        Some(1.0)
    } else {
        None
    }
}

pub fn distance_unit_to_meters(unit: &str) -> Option<f64> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "m" | "meter" | "meters" | "metre" | "metres" => Some(1.),
        "km" => Some(1000.),
        "mi" | "mile" | "miles" => Some(1609.344),
        "ft" | "feet" => Some(0.3048),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speed_source_skips_undecodable_and_empty_candidates_before_loading() {
        let mut ld = LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        let base = ld.channels[0].clone();
        ld.channels = [
            "Corr Speed",
            "Wheel Speed Front Left",
            "Wheel Speed Front Right",
            "Vehicle Speed",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut channel = base.clone();
            channel.index = index;
            channel.name = name.into();
            channel.freq = 10;
            channel.unit = "m/s".into();
            channel
        })
        .collect();
        ld.channels[0].data_type = crate::DataType::Unknown(0, 0);
        assert!(
            matches!(corrected_speed_source(&ld), Some(SpeedSource::FrontWheels(a, b)) if (a.index, b.index) == (1, 2))
        );
        ld.channels[1].data_type = crate::DataType::Unknown(0, 0);
        assert!(
            matches!(corrected_speed_source(&ld), Some(SpeedSource::Single(c)) if c.index == 2)
        );
        let (data, freq) = corrected_speed_from_ld(&ld).unwrap();
        assert_eq!(freq, 10);
        assert_eq!(
            data.len(),
            ld.read_channel_data(&ld.channels[2]).unwrap().len()
        );
        ld.channels[2].n_data = 0;
        assert!(
            matches!(corrected_speed_source(&ld), Some(SpeedSource::Single(c)) if c.index == 3)
        );
        ld.channels[3].freq = 0;
        assert!(corrected_speed_source(&ld).is_none());
    }

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

//! Explicit correction policies; defaults retain the established i3rs calculations.
use crate::channels::{Interpolation, sample_with_policy};
use crate::distance::*;
use crate::{Lap, LdFile, TrackData};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClampAction {
    Zero,
    #[default]
    Invalidate,
    Clamp,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedLimit {
    pub threshold_mps: f64,
    pub action: ClampAction,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WheelLockCorrection {
    pub acceleration_channel: String,
    pub lag_seconds: f64,
    pub braking_threshold_mps2: f64,
    pub speed_threshold_mps: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceSource {
    #[default]
    Automatic,
    CorrectedSpeed,
    Channel,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum GpsShift {
    #[default]
    None,
    Manual(f64),
    Automatic(f64),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorrectionSettings {
    pub speed_enabled: bool,
    pub prefer_logged_corrected_speed: bool,
    pub fastest_pair: Option<[String; 2]>,
    pub speed_priority: Vec<String>,
    pub gps_speed_channel: String,
    pub gps_fallback: bool,
    pub lower_speed: Option<SpeedLimit>,
    pub upper_speed: Option<SpeedLimit>,
    pub wheel_lock: Option<WheelLockCorrection>,
    pub distance_enabled: bool,
    pub distance_source: DistanceSource,
    pub distance_channel: String,
    pub stretch_enabled: bool,
    pub stretch_tolerance: f64,
    pub track_length_m: Option<f64>,
    pub gps_enabled: bool,
    pub latitude_channel: String,
    pub longitude_channel: String,
    pub gps_interpolation: Interpolation,
    pub interpolate_duplicates: bool,
    pub gps_shift: GpsShift,
    pub gps_quality: Option<(String, f64)>,
    pub gps_zero_below_mps: Option<f64>,
    pub invalidate_position_with_quality: bool,
}
impl Default for CorrectionSettings {
    fn default() -> Self {
        Self {
            speed_enabled: true,
            prefer_logged_corrected_speed: true,
            fastest_pair: Some([
                "Wheel Speed Front Left".into(),
                "Wheel Speed Front Right".into(),
            ]),
            speed_priority: SPEED_FALLBACKS.iter().map(|s| s.to_string()).collect(),
            gps_speed_channel: "GPS Speed".into(),
            gps_fallback: true,
            lower_speed: None,
            upper_speed: None,
            wheel_lock: None,
            distance_enabled: true,
            distance_source: DistanceSource::Automatic,
            distance_channel: "Lap Distance".into(),
            stretch_enabled: true,
            stretch_tolerance: crate::track::DISTANCE_STRETCH_TOLERANCE,
            track_length_m: None,
            gps_enabled: true,
            latitude_channel: String::new(),
            longitude_channel: String::new(),
            gps_interpolation: Interpolation::Linear,
            interpolate_duplicates: false,
            gps_shift: GpsShift::None,
            gps_quality: None,
            gps_zero_below_mps: None,
            invalidate_position_with_quality: true,
        }
    }
}
impl CorrectionSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.stretch_tolerance.is_finite()
            || !(0.0..=1.0).contains(&self.stretch_tolerance)
            || self
                .track_length_m
                .is_some_and(|v| !v.is_finite() || v <= 0. || v > 1_000_000.)
        {
            return Err("Invalid track length or stretch tolerance".into());
        }
        if self
            .lower_speed
            .as_ref()
            .into_iter()
            .chain(self.upper_speed.as_ref())
            .any(|v| !v.threshold_mps.is_finite() || v.threshold_mps < 0.)
            || self
                .lower_speed
                .as_ref()
                .zip(self.upper_speed.as_ref())
                .is_some_and(|(a, b)| a.threshold_mps > b.threshold_mps)
        {
            return Err("Invalid speed clamp limits".into());
        }
        if let Some(w) = &self.wheel_lock
            && (w.acceleration_channel.trim().is_empty()
                || ![
                    w.lag_seconds,
                    w.braking_threshold_mps2,
                    w.speed_threshold_mps,
                ]
                .iter()
                .all(|v| v.is_finite())
                || w.speed_threshold_mps < 0.)
        {
            return Err("Invalid wheel-lock correction".into());
        }
        match self.gps_shift {
            GpsShift::Manual(v) if !v.is_finite() || v.abs() > 60. => {
                return Err("GPS time shift must be within 60 seconds".into());
            }
            GpsShift::Automatic(v) if !v.is_finite() || !(0.0..=1.0).contains(&v) => {
                return Err("GPS automatic fraction must be between 0 and 1".into());
            }
            _ => {}
        }
        if self
            .gps_quality
            .as_ref()
            .is_some_and(|(name, v)| name.trim().is_empty() || !v.is_finite())
            || self
                .gps_zero_below_mps
                .is_some_and(|v| !v.is_finite() || v < 0.)
        {
            return Err("Invalid GPS speed validity settings".into());
        }
        Ok(())
    }
}
pub struct CorrectedRecording {
    pub gps_speed: Option<(Vec<f64>, u16)>,
    pub speed: Option<(Vec<f64>, u16)>,
    pub distance: Option<(Vec<f64>, u16)>,
    pub track: Option<TrackData>,
    pub messages: Vec<String>,
}

pub fn configured_speed_source<'a>(
    ld: &'a LdFile,
    s: &CorrectionSettings,
) -> Option<SpeedSource<'a>> {
    if !s.speed_enabled {
        return None;
    }
    let usable = |name: &str| {
        ld.find_channel_by_name(name).filter(|c| {
            c.freq > 0
                && c.n_data > 0
                && c.data_type.bytes_per_sample().is_some()
                && speed_unit_to_mps(&c.unit).is_some()
        })
    };
    if s.prefer_logged_corrected_speed
        && let Some(c) = usable("Corr Speed")
    {
        return Some(SpeedSource::Single(c));
    }
    if let Some(pair) = &s.fastest_pair
        && let (Some(a), Some(b)) = (usable(&pair[0]), usable(&pair[1]))
    {
        return Some(SpeedSource::FrontWheels(a, b));
    }
    s.speed_priority
        .iter()
        .filter(|n| s.gps_fallback || *n != &s.gps_speed_channel)
        .find_map(|n| usable(n))
        .or_else(|| {
            s.gps_fallback
                .then(|| usable(&s.gps_speed_channel))
                .flatten()
        })
        .map(SpeedSource::Single)
}
fn read_speed(ld: &LdFile, source: SpeedSource<'_>) -> Option<(Vec<f64>, u16)> {
    match source {
        SpeedSource::Single(c) => Some((
            ld.read_channel_data(c)?
                .iter()
                .map(|v| v * speed_unit_to_mps(&c.unit).unwrap())
                .collect(),
            c.freq,
        )),
        SpeedSource::FrontWheels(a, b) => fastest_speed(
            (&ld.read_channel_data(a)?, a.freq, &a.unit),
            (&ld.read_channel_data(b)?, b.freq, &b.unit),
        ),
    }
}
pub fn clamp_speed(data: &mut [f64], lower: Option<&SpeedLimit>, upper: Option<&SpeedLimit>) {
    for v in data {
        if !v.is_finite() {
            continue;
        }
        for (limit, low) in [(lower, true), (upper, false)] {
            if let Some(l) = limit
                && if low {
                    *v < l.threshold_mps
                } else {
                    *v > l.threshold_mps
                }
            {
                *v = match l.action {
                    ClampAction::Zero => 0.,
                    ClampAction::Invalidate => f64::NAN,
                    ClampAction::Clamp => l.threshold_mps,
                };
                break;
            }
        }
    }
}
/// During braking use integrated acceleration only when wheel speed drops below
/// its prediction by the configured threshold. Restart at missing data.
pub fn correct_wheel_locks(
    speed: &mut [f64],
    freq: u16,
    accel: &[f64],
    accel_freq: u16,
    w: &WheelLockCorrection,
) {
    if freq == 0 {
        return;
    }
    let mut previous: Option<f64> = None;
    for (i, v) in speed.iter_mut().enumerate() {
        let a = sample_with_policy(
            accel,
            accel_freq,
            i as f64 / freq as f64 + w.lag_seconds,
            Interpolation::Linear,
        );
        if let (Some(prev), Some(a)) = (previous, a)
            && v.is_finite()
            && a < w.braking_threshold_mps2
        {
            let predicted = (prev + a / freq as f64).max(0.);
            if predicted - *v > w.speed_threshold_mps {
                *v = predicted;
            }
        }
        previous = (v.is_finite() && a.is_some()).then_some(*v);
    }
}
/// Stretch supplied competition ranges; the caller selects them by type.
/// Preserve cumulative continuity outside those ranges.
pub fn stretch_distance(data: &mut [f64], freq: u16, laps: &[Lap], length: f64, tolerance: f64) {
    if freq == 0 {
        return;
    }
    let original = data.to_vec();
    let mut changes: Vec<(f64, f64, f64, f64)> = vec![];
    for lap in laps {
        if let (Some(a), Some(b)) = (
            crate::sample_at_time(&original, freq, lap.start_time),
            crate::sample_at_time(&original, freq, lap.end_time),
        ) {
            let measured = b - a;
            if measured > 0. && ((measured - length) / length).abs() <= tolerance {
                changes.push((lap.start_time, lap.end_time, a, length / measured));
            }
        }
    }
    for (i, v) in data.iter_mut().enumerate() {
        let t = i as f64 / freq as f64;
        let mut offset = 0.;
        for &(start, end, a, scale) in &changes {
            if t >= end {
                let b = crate::sample_at_time(&original, freq, end).unwrap();
                offset += (b - a) * (scale - 1.);
            } else if t >= start {
                *v = a + (*v - a) * scale;
                break;
            }
        }
        *v += offset;
    }
}
fn duplicate_interpolation(a: &mut [f64], b: &mut [f64]) {
    let mut start = 0;
    while start < a.len() {
        if !a[start].is_finite() || !b[start].is_finite() {
            start += 1;
            continue;
        }
        let mut next = start + 1;
        while next < a.len() && a[next] == a[start] && b[next] == b[start] {
            next += 1;
        }
        if next < a.len() && a[next].is_finite() && b[next].is_finite() {
            for i in start + 1..next {
                let f = (i - start) as f64 / (next - start) as f64;
                a[i] = a[start] + f * (a[next] - a[start]);
                b[i] = b[start] + f * (b[next] - b[start]);
            }
        }
        start = next;
    }
}
fn automatic_shift(lat: &[f64], lon: &[f64], freq: u16, fraction: f64) -> f64 {
    let changed: Vec<_> = (1..lat.len().min(lon.len()))
        .filter(|&i| {
            [lat[i], lon[i], lat[i - 1], lon[i - 1]]
                .iter()
                .all(|v| v.is_finite())
                && (lat[i] != lat[i - 1] || lon[i] != lon[i - 1])
        })
        .collect();
    let mut periods: Vec<_> = changed
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 / freq as f64)
        .collect();
    periods.sort_by(f64::total_cmp);
    periods
        .get(periods.len() / 2)
        .copied()
        .unwrap_or(1. / freq as f64)
        * fraction
}

/// Correct positions on the latitude grid. Positive shifts advance delayed data.
/// Automatic shift is a fraction of the median finite fix-change interval.
pub fn gps_track_with_settings(
    lat: &[f64],
    lat_freq: u16,
    lon: &[f64],
    lon_freq: u16,
    s: &CorrectionSettings,
    quality: Option<(&[f64], u16, f64)>,
) -> Result<(Option<TrackData>, f64), String> {
    s.validate()?;
    if lat_freq == 0 || lon_freq == 0 {
        return Err("GPS channels need nonzero sample rates".into());
    }
    let mut lat = lat.to_vec();
    let mut aligned: Vec<_> = (0..lat.len())
        .map(|i| {
            sample_with_policy(
                lon,
                lon_freq,
                i as f64 / lat_freq as f64,
                s.gps_interpolation,
            )
            .unwrap_or(f64::NAN)
        })
        .collect();
    for (i, (x, y)) in lat.iter_mut().zip(aligned.iter_mut()).enumerate() {
        if !x.is_finite()
            || !y.is_finite()
            || x.abs() > 90.
            || y.abs() > 180.
            || (*x == 0. && *y == 0.)
            || s.invalidate_position_with_quality
                && quality.is_some_and(|(q, f, min)| {
                    sample_with_policy(q, f, i as f64 / lat_freq as f64, Interpolation::Previous)
                        .is_none_or(|v| v < min)
                })
        {
            *x = f64::NAN;
            *y = f64::NAN;
        }
    }
    let shift = match s.gps_shift {
        GpsShift::None => 0.,
        GpsShift::Manual(v) => v,
        GpsShift::Automatic(f) => automatic_shift(&lat, &aligned, lat_freq, f),
    };
    if s.interpolate_duplicates {
        duplicate_interpolation(&mut lat, &mut aligned);
    }
    if shift != 0. {
        let shifted = |data: &[f64]| {
            (0..data.len())
                .map(|i| {
                    sample_with_policy(
                        data,
                        lat_freq,
                        i as f64 / lat_freq as f64 + shift,
                        s.gps_interpolation,
                    )
                    .unwrap_or(f64::NAN)
                })
                .collect::<Vec<_>>()
        };
        lat = shifted(&lat);
        aligned = shifted(&aligned);
    }
    Ok((
        crate::track::gps_track_from_samples(&lat, lat_freq, &aligned, lat_freq),
        shift,
    ))
}

pub fn correct_gps_speed(
    samples: &[f64],
    freq: u16,
    s: &CorrectionSettings,
    quality: Option<(&[f64], u16, f64)>,
    shift: f64,
) -> Vec<f64> {
    if freq == 0 {
        return vec![f64::NAN; samples.len()];
    }
    let mut data = samples.to_vec();
    if s.interpolate_duplicates {
        let mut other = data.clone();
        duplicate_interpolation(&mut data, &mut other);
    }
    if shift != 0. {
        data = (0..data.len())
            .map(|i| {
                sample_with_policy(
                    &data,
                    freq,
                    i as f64 / freq as f64 + shift,
                    s.gps_interpolation,
                )
                .unwrap_or(f64::NAN)
            })
            .collect();
    }
    for (i, v) in data.iter_mut().enumerate() {
        let t = i as f64 / freq as f64 + shift;
        if quality.is_some_and(|(q, f, min)| {
            sample_with_policy(q, f, t, Interpolation::Previous).is_none_or(|v| v < min)
        }) {
            *v = f64::NAN;
        }
        if s.gps_zero_below_mps.is_some_and(|min| *v < min) {
            *v = 0.;
        }
    }
    data
}

pub fn build_corrected_recording(
    ld: &LdFile,
    s: &CorrectionSettings,
    laps: &[Lap],
) -> Result<CorrectedRecording, String> {
    s.validate()?;
    let mut messages = vec![];
    let mut speed = configured_speed_source(ld, s).and_then(|source| read_speed(ld, source));
    if s.speed_enabled && speed.is_none() {
        messages.push("No usable corrected-speed source".into());
    }
    let quality = if let Some((name, min)) = &s.gps_quality {
        let c = ld
            .find_channel_by_name(name)
            .ok_or_else(|| format!("GPS validity source '{name}' is unavailable"))?;
        Some((
            ld.read_channel_data(c)
                .ok_or("GPS validity decode failed")?,
            c.freq,
            *min,
        ))
    } else {
        None
    };
    let mut track = None;
    let mut gps_shift = match s.gps_shift {
        GpsShift::Manual(v) => v,
        _ => 0.,
    };
    if s.gps_enabled {
        let channels = if s.latitude_channel.is_empty() && s.longitude_channel.is_empty() {
            crate::track::gps_channels(ld)
        } else {
            ld.find_channel_by_name(&s.latitude_channel)
                .zip(ld.find_channel_by_name(&s.longitude_channel))
        };
        if let Some((a, b)) = channels {
            let lat = ld.read_channel_data(a).ok_or("Latitude decode failed")?;
            let lon = ld.read_channel_data(b).ok_or("Longitude decode failed")?;
            let quality_samples = quality.as_ref().map(|(v, f, min)| (v.as_slice(), *f, *min));
            (track, gps_shift) =
                gps_track_with_settings(&lat, a.freq, &lon, b.freq, s, quality_samples)?;
        } else {
            messages.push("GPS latitude/longitude sources are unavailable".into());
        }
    }
    let gps_speed = if s.gps_enabled {
        ld.find_channel_by_name(&s.gps_speed_channel)
            .filter(|c| c.freq > 0)
            .and_then(|c| {
                let scale = speed_unit_to_mps(&c.unit)?;
                let raw = ld.read_channel_data(c)?;
                let si: Vec<_> = raw.iter().map(|v| v * scale).collect();
                let q = quality.as_ref().map(|(v, f, min)| (v.as_slice(), *f, *min));
                Some((correct_gps_speed(&si, c.freq, s, q, gps_shift), c.freq))
            })
    } else {
        None
    };
    if let Some((data, freq)) = &mut speed {
        if let Some(w) = &s.wheel_lock {
            let c = ld
                .find_channel_by_name(&w.acceleration_channel)
                .ok_or("Longitudinal acceleration source is unavailable")?;
            let (scale, offset) = crate::channels::unit_transform(&c.unit, "m/s/s")?;
            let accel: Vec<_> = ld
                .read_channel_data(c)
                .ok_or("Acceleration decode failed")?
                .iter()
                .map(|v| v * scale + offset)
                .collect();
            correct_wheel_locks(data, *freq, &accel, c.freq, w);
        }
        if let Some(SpeedSource::Single(c)) = configured_speed_source(ld, s)
            && ld
                .find_channel_by_name(&s.gps_speed_channel)
                .is_some_and(|gps| gps.index == c.index)
            && let Some((gps, f)) = &gps_speed
            && f == freq
        {
            *data = gps.clone();
        }
        clamp_speed(data, s.lower_speed.as_ref(), s.upper_speed.as_ref());
    }
    let mut distance = if !s.distance_enabled {
        None
    } else if s.distance_source == DistanceSource::Channel {
        let c = ld
            .find_channel_by_name(&s.distance_channel)
            .ok_or("Distance channel is unavailable")?;
        if c.freq == 0 {
            return Err("Distance channel has zero sample rate".into());
        }
        let scale =
            distance_unit_to_meters(&c.unit).ok_or("Distance channel has incompatible units")?;
        let raw = ld.read_channel_data(c).ok_or("Distance decode failed")?;
        let mut out = vec![];
        let mut offset = 0.;
        let mut previous: Option<f64> = None;
        for v in raw {
            let v = v * scale;
            if v.is_finite() {
                if let Some(prev) = previous
                    && v < prev
                {
                    offset += prev;
                }
                previous = Some(v);
                out.push(v + offset);
            } else {
                out.push(out.last().copied().unwrap_or(0.));
            }
        }
        Some((out, c.freq))
    } else {
        speed
            .as_ref()
            .and_then(|(v, f)| integrate_speed_series(v, *f, "m/s").map(|d| (d, *f)))
    };
    if s.stretch_enabled
        && let (Some(length), Some((data, f))) = (s.track_length_m, &mut distance)
    {
        stretch_distance(data, *f, laps, length, s.stretch_tolerance);
    }
    if let Some(track) = &mut track {
        track.distance = distance.clone();
        track.corrected_speed = speed.clone();
        track.distance_stretch_tolerance = if s.stretch_enabled {
            Some(s.stretch_tolerance)
        } else {
            None
        };
    }
    Ok(CorrectedRecording {
        gps_speed,
        speed,
        distance,
        track,
        messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn log() -> LdFile {
        LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap()
    }
    #[test]
    fn default_build_preserves_validated_speed_distance_and_gps() {
        let ld = log();
        let legacy = crate::extract_gps_track(&ld).unwrap();
        let result =
            build_corrected_recording(&ld, &Default::default(), &crate::detect_laps(&ld)).unwrap();
        let equal = |a: &[f64], b: &[f64]| {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert!(
                    a.to_bits() == b.to_bits() || a.is_nan() && b.is_nan(),
                    "{a} != {b}"
                );
            }
        };
        equal(
            &legacy.corrected_speed.as_ref().unwrap().0,
            &result.speed.unwrap().0,
        );
        equal(
            &legacy.distance.as_ref().unwrap().0,
            &result.distance.unwrap().0,
        );
        let track = result.track.unwrap();
        equal(&legacy.x, &track.x);
        equal(&legacy.y, &track.y);
        assert_eq!(legacy.time, track.time);
    }
    #[test]
    fn priority_pair_disable_and_missing_sources_are_predictable() {
        let ld = log();
        let mut s = CorrectionSettings {
            prefer_logged_corrected_speed: false,
            fastest_pair: None,
            speed_priority: vec!["Missing".into(), "GPS Speed".into()],
            ..Default::default()
        };
        assert!(
            matches!(configured_speed_source(&ld,&s),Some(SpeedSource::Single(c)) if c.name=="GPS Speed")
        );
        s.gps_fallback = false;
        assert!(configured_speed_source(&ld, &s).is_none());
        s.speed_enabled = false;
        assert!(configured_speed_source(&ld, &s).is_none());
        s.distance_enabled = false;
        s.gps_enabled = false;
        let built = build_corrected_recording(&ld, &s, &[]).unwrap();
        assert!(built.speed.is_none() && built.distance.is_none() && built.track.is_none());
    }
    #[test]
    fn clamps_and_braking_correction_preserve_missing_segments() {
        let mut v = [-1., 5., 25., f64::NAN];
        clamp_speed(
            &mut v,
            Some(&SpeedLimit {
                threshold_mps: 0.,
                action: ClampAction::Zero,
            }),
            Some(&SpeedLimit {
                threshold_mps: 20.,
                action: ClampAction::Clamp,
            }),
        );
        assert_eq!(v[..3], [0., 5., 20.]);
        assert!(v[3].is_nan());
        let w = WheelLockCorrection {
            acceleration_channel: "G".into(),
            lag_seconds: 0.,
            braking_threshold_mps2: -0.1,
            speed_threshold_mps: 1.,
        };
        let mut v = [20., 5., 4., f64::NAN, 3., 2.];
        correct_wheel_locks(&mut v, 1, &[-2.; 6], 1, &w);
        assert_eq!(v[..3], [20., 18., 16.]);
        assert!(v[3].is_nan());
        assert_eq!(v[4..], [3., 2.]);
        let mut v = [20., 5., 4.];
        correct_wheel_locks(&mut v, 1, &[-2., f64::NAN, -2.], 1, &w);
        assert_eq!(v, [20., 5., 4.]);
    }
    #[test]
    fn off_grid_distance_stretch_is_continuous_and_respects_tolerance() {
        let lap = Lap {
            number: 1,
            name: "Lap 1".into(),
            start_time: 0.5,
            end_time: 4.5,
        };
        let mut d = (0..7).map(|i| i as f64 * 10.).collect::<Vec<_>>();
        stretch_distance(&mut d, 1, std::slice::from_ref(&lap), 42., 0.05);
        assert_eq!(d[0], 0.);
        assert!((d[1] - 10.25).abs() < 1e-9);
        assert!((d[5] - 52.).abs() < 1e-9);
        assert_eq!(d[6], 62.);
        let mut unchanged = (0..7).map(|i| i as f64 * 10.).collect::<Vec<_>>();
        stretch_distance(&mut unchanged, 1, &[lap], 45., 0.05);
        assert_eq!(
            unchanged,
            (0..7).map(|i| i as f64 * 10.).collect::<Vec<_>>()
        );
    }
    #[test]
    fn gps_duplicate_interpolation_time_shift_and_validity_keep_outages() {
        let lat = [36., 36., 36.002, 0., 36.004, 36.004, 36.006];
        let lon = [-79., -79., -79.002, 0., -79.004, -79.004, -79.006];
        let mut s = CorrectionSettings {
            interpolate_duplicates: true,
            ..Default::default()
        };
        let (track, shift) = gps_track_with_settings(&lat, 1, &lon, 1, &s, None).unwrap();
        assert_eq!(shift, 0.);
        let fixes = track.unwrap().geographic_fixes().unwrap();
        assert!((fixes[1][0] - 36.001).abs() < 1e-10);
        assert!(fixes[3][0].is_nan());
        s.gps_shift = GpsShift::Manual(1.);
        let (track, shift) = gps_track_with_settings(
            &lat,
            1,
            &lon,
            1,
            &s,
            Some((&[5., 5., 5., 5., 0., 5., 5.], 1, 4.)),
        )
        .unwrap();
        assert_eq!(shift, 1.);
        let fixes = track.unwrap().geographic_fixes().unwrap();
        assert!((fixes[0][0] - 36.001).abs() < 1e-10);
        assert!(fixes[2][0].is_nan());
        assert!(fixes[3][0].is_nan());
        assert!(fixes[6][0].is_nan());
        s.gps_shift = GpsShift::Automatic(0.8);
        let (_, shift) = gps_track_with_settings(
            &[36., 36., 36.001, 36.001, 36.002, 36.002],
            1,
            &[-79.; 6],
            1,
            &s,
            None,
        )
        .unwrap();
        assert!((shift - 1.6).abs() < 1e-9);
        assert!(gps_track_with_settings(&lat, 0, &lon, 1, &s, None).is_err());
    }
}

#[cfg(test)]
mod gps_speed_tests {
    use super::*;
    #[test]
    fn independent_gps_speed_validity_shift_and_zeroing_preserve_missing_values() {
        let s = CorrectionSettings {
            gps_zero_below_mps: Some(1.),
            ..Default::default()
        };
        let result = correct_gps_speed(
            &[0.5, 2., 4., f64::NAN],
            1,
            &s,
            Some((&[5., 3., 5., 5.], 1, 4.)),
            0.,
        );
        assert_eq!(result[0], 0.);
        assert!(result[1].is_nan());
        assert_eq!(result[2], 4.);
        assert!(result[3].is_nan());
        let shifted = correct_gps_speed(&[0.5, 2., 4., 6.], 1, &s, None, 0.5);
        assert_eq!(&shifted[..3], &[1.25, 3., 5.]);
    }
}

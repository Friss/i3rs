//! Acceleration events derived from continuous speed regions, in SI units.
//! Hysteresis confirms a direction/reversal; event times remain interpolated at
//! the requested speed points, rather than at the confirmation samples.
use crate::report::{ReportSeries, intersect_report_windows, report_windows};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccelerationSettings {
    pub points_mps: Vec<f64>,
    pub hysteresis_mps: f64,
    pub accel_hold_secs: f64,
    pub decel_hold_secs: f64,
    pub start_padding_secs: f64,
    pub end_padding_secs: f64,
}
impl Default for AccelerationSettings {
    fn default() -> Self {
        Self {
            points_mps: vec![60. / 3.6, 100. / 3.6, 140. / 3.6, 180. / 3.6, 220. / 3.6],
            hysteresis_mps: 2. / 3.6,
            accel_hold_secs: 0.5,
            decel_hold_secs: 0.5,
            start_padding_secs: 5.,
            end_padding_secs: 5.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccelerationDirection {
    Acceleration,
    Deceleration,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccelerationEvent {
    pub region: usize,
    pub direction: AccelerationDirection,
    pub from_mps: f64,
    pub to_mps: f64,
    pub start: f64,
    pub end: f64,
    pub elapsed_secs: f64,
    pub distance_metres: f64,
    pub average_g: f64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccelerationReport {
    pub selection: Vec<(f64, f64)>,
    pub search_windows: Vec<(f64, f64)>,
    pub events: Vec<AccelerationEvent>,
}

/// `allowed` is an optional trust mask. Padding never extends outside this mask.
/// Gaps, invalid speeds and recording limits break regions; no extrapolation.
/// Every speed pair is emitted once per confirmed region, provided it overlaps
/// the unpadded selection. Distance is the piecewise-linear speed integral.
pub fn acceleration_report(
    series: &ReportSeries<'_>,
    selection: &[(f64, f64)],
    allowed: Option<&[(f64, f64)]>,
    settings: &AccelerationSettings,
) -> Result<AccelerationReport, String> {
    if settings.points_mps.len() < 2
        || settings.points_mps.len() > 64
        || settings
            .points_mps
            .iter()
            .any(|p| !p.is_finite() || *p < 0.)
        || settings.points_mps.windows(2).any(|w| w[0] >= w[1])
    {
        return Err("Use 2–64 strictly increasing, finite, nonnegative speed points".into());
    }
    if [
        settings.hysteresis_mps,
        settings.accel_hold_secs,
        settings.decel_hold_secs,
        settings.start_padding_secs,
        settings.end_padding_secs,
    ]
    .iter()
    .any(|v| !v.is_finite() || *v < 0.)
    {
        return Err("Hysteresis, hold times and offsets must be finite and nonnegative".into());
    }
    let selection = report_windows(selection);
    let mut result = AccelerationReport {
        selection: selection.clone(),
        ..Default::default()
    };
    if series.freq == 0 || series.data.len() < 2 {
        return Ok(result);
    }
    let last = (series.data.len() - 1) as f64 / series.freq as f64;
    let padded: Vec<_> = selection
        .iter()
        .map(|(a, b)| {
            (
                (a - settings.start_padding_secs).max(0.),
                (b + settings.end_padding_secs).min(last),
            )
        })
        .collect();
    result.search_windows = if let Some(mask) = allowed {
        intersect_report_windows(&padded, mask)
    } else {
        report_windows(&padded)
    };
    for &(a, b) in &result.search_windows {
        let first = (a * series.freq as f64).floor() as usize;
        let end = ((b * series.freq as f64).ceil() as usize).min(series.data.len() - 1);
        let mut points = Vec::new();
        for i in first..end {
            let t0 = i as f64 / series.freq as f64;
            let t1 = (i + 1) as f64 / series.freq as f64;
            let v0 = series.data[i] * series.scale + series.offset;
            let v1 = series.data[i + 1] * series.scale + series.offset;
            if !v0.is_finite() || !v1.is_finite() || v0 < 0. || v1 < 0. {
                emit_regions(&points, &selection, settings, &mut result.events);
                points.clear();
                continue;
            }
            let lo = a.max(t0);
            let hi = b.min(t1);
            if hi <= lo {
                continue;
            }
            let at = |t| v0 + (v1 - v0) * (t - t0) / (t1 - t0);
            if points.is_empty() {
                points.push((lo, at(lo)));
            }
            points.push((hi, at(hi)));
        }
        emit_regions(&points, &selection, settings, &mut result.events);
    }
    Ok(result)
}
fn emit_regions(
    points: &[(f64, f64)],
    selection: &[(f64, f64)],
    settings: &AccelerationSettings,
    events: &mut Vec<AccelerationEvent>,
) {
    if points.len() < 2 {
        return;
    }
    let mut low = 0;
    let mut high = 0;
    let mut start = 0;
    let mut extreme = 0;
    let mut direction = 0i8;
    for i in 1..points.len() {
        let v = points[i].1;
        if direction == 0 {
            if v < points[low].1 {
                low = i;
            }
            if v > points[high].1 {
                high = i;
            }
            if v > points[low].1 && v - points[low].1 >= settings.hysteresis_mps {
                direction = 1;
                start = low;
                extreme = i;
            } else if v < points[high].1 && points[high].1 - v >= settings.hysteresis_mps {
                direction = -1;
                start = high;
                extreme = i;
            }
        } else if (v - points[extreme].1) * direction as f64 >= 0. {
            extreme = i;
        } else if (points[extreme].1 - v) * direction as f64 >= settings.hysteresis_mps {
            emit_pairs(
                &points[start..=extreme],
                direction,
                selection,
                settings,
                events,
            );
            start = extreme;
            extreme = i;
            direction = -direction;
        }
    }
    if direction != 0 {
        emit_pairs(
            &points[start..=extreme],
            direction,
            selection,
            settings,
            events,
        );
    }
}
fn emit_pairs(
    points: &[(f64, f64)],
    direction: i8,
    selection: &[(f64, f64)],
    settings: &AccelerationSettings,
    events: &mut Vec<AccelerationEvent>,
) {
    let hold = if direction > 0 {
        settings.accel_hold_secs
    } else {
        settings.decel_hold_secs
    };
    if points.len() < 2 || points.last().unwrap().0 - points[0].0 < hold {
        return;
    }
    let region = events.last().map_or(0, |e| e.region + 1);
    let mut integral = vec![0.; points.len()];
    for i in 1..points.len() {
        integral[i] = integral[i - 1]
            + (points[i - 1].1 * 0.5 + points[i].1 * 0.5) * (points[i].0 - points[i - 1].0);
    }
    let crossings: Vec<_> = settings
        .points_mps
        .iter()
        .map(|speed| {
            points.windows(2).enumerate().find_map(|(i, w)| {
                let ((a, x), (b, y)) = (w[0], w[1]);
                if (y - x) * direction as f64 > 0.
                    && (speed - x) * direction as f64 >= 0.
                    && (y - speed) * direction as f64 >= 0.
                {
                    let t = a + (b - a) * (speed - x) / (y - x);
                    Some((t, integral[i] + (x * 0.5 + speed * 0.5) * (t - a)))
                } else {
                    None
                }
            })
        })
        .collect();
    for i in 0..settings.points_mps.len() - 1 {
        for j in i + 1..settings.points_mps.len() {
            let (from, to) = if direction > 0 {
                (settings.points_mps[i], settings.points_mps[j])
            } else {
                (settings.points_mps[j], settings.points_mps[i])
            };
            let (from_cross, to_cross) = if direction > 0 {
                (crossings[i], crossings[j])
            } else {
                (crossings[j], crossings[i])
            };
            let Some(((a, da), (b, db))) = from_cross.zip(to_cross) else {
                continue;
            };
            if b <= a || !selection.iter().any(|(lo, hi)| a < *hi && b > *lo) {
                continue;
            }
            let distance = db - da;
            let average_g = (to - from) / (b - a) / 9.80665;
            if !distance.is_finite() || !average_g.is_finite() {
                continue;
            }
            events.push(AccelerationEvent {
                region,
                direction: if direction > 0 {
                    AccelerationDirection::Acceleration
                } else {
                    AccelerationDirection::Deceleration
                },
                from_mps: from,
                to_mps: to,
                start: a,
                end: b,
                elapsed_secs: b - a,
                distance_metres: distance,
                average_g,
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn run(
        data: &[f64],
        selection: &[(f64, f64)],
        mask: Option<&[(f64, f64)]>,
        settings: &AccelerationSettings,
    ) -> AccelerationReport {
        acceleration_report(
            &ReportSeries {
                data,
                freq: 1,
                scale: 1.,
                offset: 0.,
                interpolation: crate::Interpolation::Linear,
            },
            selection,
            mask,
            settings,
        )
        .unwrap()
    }
    fn settings() -> AccelerationSettings {
        AccelerationSettings {
            points_mps: vec![5., 15., 25.],
            hysteresis_mps: 2.,
            accel_hold_secs: 0.,
            decel_hold_secs: 0.,
            start_padding_secs: 10.,
            end_padding_secs: 10.,
        }
    }
    #[test]
    fn exact_crossings_all_pairs_integrals_and_deceleration() {
        let r = run(
            &[0., 10., 20., 30., 20., 10., 0.],
            &[(1., 5.)],
            None,
            &settings(),
        );
        assert_eq!(r.events.len(), 6);
        let e = &r.events[0];
        assert_eq!(
            (e.start, e.end, e.elapsed_secs, e.distance_metres),
            (0.5, 1.5, 1., 10.)
        );
        assert!((e.average_g - 10. / 9.80665).abs() < 1e-12);
        assert_eq!(r.events[3].direction, AccelerationDirection::Deceleration);
        assert_eq!((r.events[3].from_mps, r.events[3].to_mps), (15., 5.));
        assert!(r.events[3].average_g < 0.);
    }
    #[test]
    fn padding_hold_hysteresis_and_trust_boundaries() {
        let data = [0., 10., 9., 20., 30., 20., 10., 0.];
        let mut s = settings();
        assert_eq!(run(&data, &[(1., 2.)], None, &s).events.len(), 2);
        s.start_padding_secs = 0.;
        s.end_padding_secs = 0.;
        assert!(run(&data, &[(1., 2.)], None, &s).events.is_empty());
        s = settings();
        s.accel_hold_secs = 5.;
        s.decel_hold_secs = 5.;
        assert!(run(&data, &[(1., 6.)], None, &s).events.is_empty());
        s = settings();
        s.hysteresis_mps = 40.;
        assert!(run(&data, &[(1., 6.)], None, &s).events.is_empty());
        s = settings();
        assert!(
            run(&data, &[(1., 2.)], Some(&[(1., 2.)]), &s)
                .events
                .is_empty()
        );
    }
    #[test]
    fn missing_samples_break_events_no_extrapolation_or_duplicate_padding() {
        let s = settings();
        assert!(
            run(&[0., 10., f64::NAN, 30.], &[(0., 4.)], None, &s)
                .events
                .is_empty()
        );
        let r = run(&[0., 10., 20., 30.], &[(0., 1.), (2., 3.)], None, &s);
        assert_eq!(r.events.len(), 3);
        assert_eq!(r.search_windows, vec![(0., 3.)]);
        let mut bad = s.clone();
        bad.points_mps = vec![1., 1.];
        assert!(
            acceleration_report(
                &ReportSeries {
                    data: &[0., 30.],
                    freq: 1,
                    scale: 1.,
                    offset: 0.,
                    interpolation: crate::Interpolation::Linear
                },
                &[(0., 1.)],
                None,
                &bad
            )
            .is_err()
        );
    }
    #[test]
    fn units_sample_rates_and_off_grid_padded_boundaries_preserve_exact_metrics() {
        let s = AccelerationSettings {
            start_padding_secs: 0.25,
            end_padding_secs: 0.25,
            ..settings()
        };
        let kmh: Vec<_> = (0..=30).map(|i| i as f64 * 3.6).collect();
        let series = ReportSeries {
            data: &kmh,
            freq: 10,
            scale: 1. / 3.6,
            offset: 0.,
            interpolation: crate::Interpolation::Linear,
        };
        let report = acceleration_report(&series, &[(0.75, 1.25)], None, &s).unwrap();
        assert_eq!(report.events.len(), 1);
        let event = &report.events[0];
        assert!((event.start - 0.5).abs() < 1e-12);
        assert!((event.end - 1.5).abs() < 1e-12);
        assert!((event.distance_metres - 10.).abs() < 1e-12);
        assert!(
            acceleration_report(
                &series,
                &[(0.75, 1.25)],
                Some(&[(0.5, 0.9), (1.1, 1.5)]),
                &s
            )
            .unwrap()
            .events
            .is_empty()
        );
    }
}

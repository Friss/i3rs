//! Numerical contracts for component populations and specialized analysis.
//! Windows are half open; missing pairs and rejected gates never enter denominators.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Regression {
    pub count: usize,
    pub slope: f64,
    pub intercept: f64,
    /// Undefined for a constant dependent variable.
    pub r_squared: Option<f64>,
}

/// Numerically stable least squares over finite pairs; a constant X has no fit.
pub fn linear_regression(pairs: impl IntoIterator<Item = (f64, f64)>) -> Option<Regression> {
    let (mut n, mut mx, mut my, mut xx, mut xy, mut yy) = (0usize, 0., 0., 0., 0., 0.);
    for (x, y) in pairs {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        n += 1;
        let (dx, dy) = (x - mx, y - my);
        mx += dx / n as f64;
        my += dy / n as f64;
        xx += dx * (x - mx);
        xy += dx * (y - my);
        yy += dy * (y - my);
    }
    if n < 2 || xx <= 0. {
        return None;
    }
    let slope = xy / xx;
    Some(Regression {
        count: n,
        slope,
        intercept: my - slope * mx,
        r_squared: (yy > 0.).then(|| (xy * xy / (xx * yy)).clamp(0., 1.)),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntervalAverage {
    pub start: f64,
    pub end: f64,
    pub count: usize,
    pub mean: Option<f64>,
}

/// Average native samples with an optional positive-value gate, aligned by time.
pub fn interval_averages(
    data: &[f64],
    hz: u16,
    intervals: &[(f64, f64)],
    gate: Option<(&[f64], u16)>,
) -> Vec<IntervalAverage> {
    intervals
        .iter()
        .map(|&(start, end)| {
            let window = crate::sample_window_for_time(hz, data.len(), start, end);
            let (mut count, mut mean) = (0usize, 0.);
            for (i, &value) in data
                .iter()
                .enumerate()
                .take(window.end_sample)
                .skip(window.start_sample)
            {
                let included = gate.is_none_or(|(g, f)| {
                    crate::channels::sample_with_policy(
                        g,
                        f,
                        i as f64 / hz as f64,
                        crate::Interpolation::Previous,
                    )
                    .is_some_and(crate::component_gate_includes)
                });
                if included && value.is_finite() {
                    count += 1;
                    mean += (value - mean) / count as f64;
                }
            }
            IntervalAverage {
                start,
                end,
                count,
                mean: (count > 0).then_some(mean),
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GearFilter {
    pub enabled: bool,
    pub rising_rate: f64,
    pub falling_rate: f64,
    pub exclusion_seconds: f64,
    pub rpm_filter_seconds: f64,
}
impl Default for GearFilter {
    fn default() -> Self {
        Self {
            enabled: false,
            rising_rate: 2500.,
            falling_rate: -2500.,
            exclusion_seconds: 0.5,
            rpm_filter_seconds: 0.25,
        }
    }
}

/// Centered least-squares RPM slope over the configured time window, then a
/// non-retriggering exclusion period. The odd window includes its center sample.
/// Missing values invalidate the entire slope window; gaps are never bridged.
pub fn gear_filter_mask(rpm: &[f64], hz: u16, settings: GearFilter) -> Vec<bool> {
    if !settings.enabled {
        return vec![true; rpm.len()];
    }
    if hz == 0 {
        return vec![false; rpm.len()];
    }
    let width =
        ((settings.rpm_filter_seconds.max(0.) * hz as f64).round() as usize).clamp(1, 100001) | 1;
    let half = width / 2;
    let mut until = f64::NEG_INFINITY;
    let mut mask = vec![false; rpm.len()];
    for i in 0..rpm.len() {
        let rate = if width == 1 {
            if i == 0 {
                Some(0.)
            } else {
                Some((rpm[i] - rpm[i - 1]) * hz as f64)
            }
        } else if i >= half && i + half < rpm.len() {
            let segment = &rpm[i - half..=i + half];
            if segment.iter().all(|v| v.is_finite()) {
                let mut numerator = 0.;
                let mut denominator = 0.;
                for (j, &v) in segment.iter().enumerate() {
                    let t = (j as f64 - half as f64) / hz as f64;
                    numerator += t * v;
                    denominator += t * t;
                }
                Some(numerator / denominator)
            } else {
                None
            }
        } else {
            None
        };
        let time = i as f64 / hz as f64;
        if !rpm[i].is_finite() {
            continue;
        }
        let Some(rate) = rate.filter(|r| r.is_finite()) else {
            continue;
        };
        if time >= until && (rate > settings.rising_rate || rate < settings.falling_rate) {
            until = time + settings.exclusion_seconds.max(0.);
        }
        mask[i] = time >= until;
    }
    mask
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SuspensionSummary {
    pub finite_count: usize,
    pub bump_count: usize,
    pub rebound_count: usize,
    pub stationary_count: usize,
    pub bump_mean: Option<f64>,
    pub rebound_mean: Option<f64>,
}

/// Backward velocity; positive means bump unless inverted. Missing position
/// produces missing velocity on both sides of the gap. Units follow position/s.
pub fn suspension_velocity(position: &[f64], hz: u16, invert: bool) -> Vec<f64> {
    let mut velocity = vec![f64::NAN; position.len()];
    if hz == 0 {
        return velocity;
    }
    let sign = if invert { -1. } else { 1. };
    for i in 1..position.len() {
        if position[i].is_finite() && position[i - 1].is_finite() {
            velocity[i] = (position[i] - position[i - 1]) * hz as f64 * sign;
        }
    }
    velocity
}

pub fn suspension_summary(velocity: &[f64], deadband: f64) -> SuspensionSummary {
    let (mut bump, mut rebound, mut stationary, mut bs, mut rs) = (0, 0, 0, 0., 0.);
    for &v in velocity.iter().filter(|v| v.is_finite()) {
        if v > deadband.abs() {
            bump += 1;
            bs += v;
        } else if v < -deadband.abs() {
            rebound += 1;
            rs += v;
        } else {
            stationary += 1;
        }
    }
    SuspensionSummary {
        finite_count: bump + rebound + stationary,
        bump_count: bump,
        rebound_count: rebound,
        stationary_count: stationary,
        bump_mean: (bump > 0).then(|| bs / bump as f64),
        rebound_mean: (rebound > 0).then(|| rs / rebound as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regression_known_line_missing_and_degenerate() {
        let r = linear_regression([(1., 5.), (2., 8.), (f64::NAN, 9.), (3., 11.)]).unwrap();
        assert_eq!(
            (r.count, r.slope, r.intercept, r.r_squared),
            (3, 3., 2., Some(1.))
        );
        assert!(linear_regression([(1., 2.), (1., 3.)]).is_none());
        assert_eq!(
            linear_regression([(1., 2.), (2., 2.)]).unwrap().r_squared,
            None
        );
    }
    #[test]
    fn interval_population_is_native_half_open_with_signed_mixed_rate_gate() {
        let r = interval_averages(
            &[0., 1., 2., 3., 4., 5.],
            2,
            &[(0.25, 2.5)],
            Some((&[-1., 1., f64::NAN], 1)),
        );
        assert_eq!((r[0].count, r[0].mean), (2, Some(2.5)));
    }
    #[test]
    fn shift_hold_has_exact_duration_and_missing_rpm_is_rejected() {
        let mut s = GearFilter {
            enabled: true,
            rpm_filter_seconds: 0.,
            ..Default::default()
        };
        let rpm = [
            1000.,
            1000.,
            4000.,
            4000.,
            4000.,
            4000.,
            4000.,
            4000.,
            f64::NAN,
        ];
        assert_eq!(
            gear_filter_mask(&rpm, 10, s),
            [true, true, false, false, false, false, false, true, false]
        );
        s.enabled = false;
        assert!(gear_filter_mask(&rpm, 10, s).iter().all(|v| *v));
    }
    #[test]
    fn suspension_split_inversion_and_gaps() {
        let v = suspension_velocity(&[0., 1., 0., f64::NAN, 2., 2.], 10, false);
        let s = suspension_summary(&v, 0.1);
        assert_eq!(
            (s.bump_count, s.rebound_count, s.stationary_count),
            (1, 1, 1)
        );
        assert_eq!((s.bump_mean, s.rebound_mean), (Some(10.), Some(-10.)));
        assert!(v[3].is_nan() && v[4].is_nan());
        assert_eq!(suspension_velocity(&[0., 1.], 10, true)[1], -10.);
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SuspensionSettings {
    pub input_is_velocity: bool,
    pub invert: bool,
    pub filter_seconds: f64,
    pub deadband: f64,
    pub split_velocity: f64,
}
impl Default for SuspensionSettings {
    fn default() -> Self {
        Self {
            input_is_velocity: false,
            invert: false,
            filter_seconds: 0.,
            deadband: 0.1,
            split_velocity: 50.,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct SuspensionSplit {
    pub summary: SuspensionSummary,
    pub low_bump: usize,
    pub high_bump: usize,
    pub low_rebound: usize,
    pub high_rebound: usize,
}
pub fn filtered_suspension_velocity(
    position: &[f64],
    hz: u16,
    settings: SuspensionSettings,
) -> Vec<f64> {
    let velocity = if settings.input_is_velocity {
        position
            .iter()
            .map(|v| if settings.invert { -v } else { *v })
            .collect()
    } else {
        suspension_velocity(position, hz, settings.invert)
    };
    let width =
        ((settings.filter_seconds.max(0.) * hz as f64).round() as usize).clamp(1, 100001) | 1;
    if width == 1 {
        return velocity;
    }
    let half = width / 2;
    (0..velocity.len())
        .map(|i| {
            if i < half || i + half >= velocity.len() {
                return f64::NAN;
            }
            let slice = &velocity[i - half..=i + half];
            if slice.iter().all(|v| v.is_finite()) {
                slice.iter().sum::<f64>() / width as f64
            } else {
                f64::NAN
            }
        })
        .collect()
}
pub fn suspension_split(velocity: &[f64], settings: SuspensionSettings) -> SuspensionSplit {
    let summary = suspension_summary(velocity, settings.deadband);
    let mut result = SuspensionSplit {
        summary,
        low_bump: 0,
        high_bump: 0,
        low_rebound: 0,
        high_rebound: 0,
    };
    let split = settings.split_velocity.abs().max(settings.deadband.abs());
    for &v in velocity.iter().filter(|v| v.is_finite()) {
        if v > settings.deadband.abs() {
            if v <= split {
                result.low_bump += 1;
            } else {
                result.high_bump += 1;
            }
        } else if v < -settings.deadband.abs() {
            if v >= -split {
                result.low_rebound += 1;
            } else {
                result.high_rebound += 1;
            }
        }
    }
    result
}

#[cfg(test)]
mod specialized_tests {
    use super::*;
    #[test]
    fn suspension_filter_respects_gaps_rate_sign_and_split_edges() {
        let s = SuspensionSettings {
            filter_seconds: 0.3,
            split_velocity: 10.,
            deadband: 0.1,
            ..Default::default()
        };
        let position = [0., 1., 2., 3., 4., f64::NAN, 6., 7., 8., 9.];
        let velocity = filtered_suspension_velocity(&position, 10, s);
        assert_eq!(velocity[2], 10.);
        assert!(velocity[4..8].iter().all(|v| v.is_nan()));
        let split = suspension_split(&[0., 10., 10.1, -10., -10.1, f64::NAN], s);
        assert_eq!(
            (
                split.summary.finite_count,
                split.low_bump,
                split.high_bump,
                split.low_rebound,
                split.high_rebound
            ),
            (5, 1, 1, 1, 1)
        );
        let inverse =
            filtered_suspension_velocity(&position, 10, SuspensionSettings { invert: true, ..s });
        assert_eq!(inverse[2], -10.);
        let slow = filtered_suspension_velocity(&[0., 0.5, 1., 1.5, 2.], 2, Default::default());
        assert_eq!(&slow[1..], &[1., 1., 1., 1.]);
    }
    #[test]
    fn least_squares_shift_slope_and_non_retrigger_hold_handle_missing_windows() {
        let rpm: Vec<_> = (0..30).map(|i| 1000. + 300. * i as f64).collect();
        let mut mask = gear_filter_mask(
            &rpm,
            10,
            GearFilter {
                enabled: true,
                rpm_filter_seconds: 0.3,
                ..Default::default()
            },
        );
        assert!(mask[1..29].iter().all(|v| !*v));
        let mut missing = rpm;
        missing[10] = f64::NAN;
        mask = gear_filter_mask(
            &missing,
            10,
            GearFilter {
                enabled: true,
                ..Default::default()
            },
        );
        assert!(mask[9..12].iter().all(|v| !*v));
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XIntervalAverage {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
    pub mean_x: f64,
    pub mean_y: f64,
}
/// Finite paired samples grouped into [origin+k*width, origin+(k+1)*width).
/// Empty intervals are omitted; each included pair contributes exactly once.
pub fn x_interval_averages(
    pairs: impl IntoIterator<Item = (f64, f64)>,
    width: f64,
    origin: f64,
) -> Vec<XIntervalAverage> {
    if !width.is_finite() || width <= 0. || !origin.is_finite() {
        return Vec::new();
    }
    let mut bins = std::collections::BTreeMap::<i64, (usize, f64, f64)>::new();
    for (x, y) in pairs {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let key = ((x - origin) / width).floor();
        if !key.is_finite() || key < i64::MIN as f64 || key >= i64::MAX as f64 {
            continue;
        }
        let b = bins.entry(key as i64).or_default();
        b.0 += 1;
        b.1 += (x - b.1) / b.0 as f64;
        b.2 += (y - b.2) / b.0 as f64;
    }
    bins.into_iter()
        .filter_map(|(k, (count, mean_x, mean_y))| {
            let lower = origin + k as f64 * width;
            let upper = lower + width;
            (lower.is_finite() && upper.is_finite()).then_some(XIntervalAverage {
                lower,
                upper,
                count,
                mean_x,
                mean_y,
            })
        })
        .collect()
}
/// CSV uses X units for interval edges and seconds only for selection metadata.
pub fn x_interval_csv(
    series: &str,
    x_unit: &str,
    y_unit: &str,
    selection: (f64, f64),
    rows: &[XIntervalAverage],
    header: bool,
) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut csv = if header {
        "Series,X lower,X upper,X unit,Count,Mean X,Mean Y,Y unit,Selection start (s),Selection end (s)\n".into()
    } else {
        String::new()
    };
    for r in rows {
        csv.push_str(&format!(
            "{},{:.12},{:.12},{},{},{:.12},{:.12},{},{:.9},{:.9}\n",
            quote(series),
            r.lower,
            r.upper,
            quote(x_unit),
            r.count,
            r.mean_x,
            r.mean_y,
            quote(y_unit),
            selection.0,
            selection.1
        ));
    }
    csv
}
#[cfg(test)]
mod x_average_tests {
    use super::*;
    #[test]
    fn x_averages_preserve_native_pairs_boundaries_negative_values_and_export_units() {
        let rows = x_interval_averages(
            [
                (-0.1, 1.),
                (0., 2.),
                (99.9, 4.),
                (100., 6.),
                (199.9, 8.),
                (200., 10.),
                (f64::NAN, 2.),
                (5., f64::NAN),
            ],
            100.,
            0.,
        );
        assert_eq!(
            rows.iter().map(|r| r.count).collect::<Vec<_>>(),
            [1, 2, 2, 1]
        );
        assert_eq!(
            rows.iter().map(|r| r.mean_y).collect::<Vec<_>>(),
            [1., 3., 7., 10.]
        );
        assert_eq!((rows[2].lower, rows[2].upper), (100., 200.));
        let csv = x_interval_csv("Oil, pressure", "rpm", "kPa", (10., 20.), &rows, true);
        assert!(csv.contains("\"Oil, pressure\",100.000000000000,200.000000000000,\"rpm\",2,149.950000000000,7.000000000000,\"kPa\",10.000000000,20.000000000"));
    }
}

/// Fixed symmetric velocity bins; finite values on the upper edge enter the last bin.
pub fn suspension_histogram(velocity: &[f64], bins: usize, limit: f64) -> Vec<crate::HistogramBin> {
    if bins == 0 || !limit.is_finite() || limit <= 0. {
        return Vec::new();
    }
    let width = 2. * limit / bins as f64;
    let mut result: Vec<_> = (0..bins)
        .map(|i| crate::HistogramBin {
            lower: -limit + i as f64 * width,
            upper: -limit + (i + 1) as f64 * width,
            count: 0,
        })
        .collect();
    for &v in velocity
        .iter()
        .filter(|v| v.is_finite() && v.abs() <= limit)
    {
        let i = (((v + limit) / width).floor() as usize).min(bins - 1);
        result[i].count += 1;
    }
    result
}
pub fn suspension_histogram_csv(
    series: &str,
    unit: &str,
    hz: u16,
    range: (f64, f64),
    bins: &[crate::HistogramBin],
    header: bool,
) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let count: usize = bins.iter().map(|b| b.count).sum();
    let mut csv = if header {
        "Series,Velocity lower,Velocity upper,Velocity unit,Count,Percent,Seconds,Selection start (s),Selection end (s)\n".into()
    } else {
        String::new()
    };
    for bin in bins {
        csv.push_str(&format!(
            "{},{:.12},{:.12},{},{},{:.12},{:.12},{:.9},{:.9}\n",
            quote(series),
            bin.lower,
            bin.upper,
            quote(unit),
            bin.count,
            100. * bin.count as f64 / count.max(1) as f64,
            bin.count as f64 / hz.max(1) as f64,
            range.0,
            range.1
        ));
    }
    csv
}
#[cfg(test)]
mod suspension_distribution_tests {
    use super::*;
    #[test]
    fn histogram_edges_percent_and_occupancy_export_share_counts() {
        let bins = suspension_histogram(&[-10., -5., 0., 5., 10., f64::NAN, 11.], 4, 10.);
        assert_eq!(
            bins.iter().map(|b| b.count).collect::<Vec<_>>(),
            [1, 1, 1, 2]
        );
        let csv = suspension_histogram_csv("Damper FL", "mm/s", 2, (0., 3.), &bins, true);
        assert!(csv.contains("\"mm/s\",2,40.000000000000,1.000000000000,0.000000000,3.000000000"));
    }
}

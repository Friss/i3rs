//! Reusable report statistics. Tables, charts and exports consume the same results.
use crate::{Interpolation, channels::sample_with_policy, sample_window_for_time};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportStatistic {
    Min,
    Max,
    Range,
    Average,
    AbsMax,
    Start,
    End,
    Change,
    StdDev,
}
impl ReportStatistic {
    pub const ALL: [Self; 9] = [
        Self::Min,
        Self::Max,
        Self::Range,
        Self::Average,
        Self::AbsMax,
        Self::Start,
        Self::End,
        Self::Change,
        Self::StdDev,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Min => "Min",
            Self::Max => "Max",
            Self::Range => "Range",
            Self::Average => "Avg",
            Self::AbsMax => "Abs Max",
            Self::Start => "Start",
            Self::End => "End",
            Self::Change => "Change",
            Self::StdDev => "Std Dev",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportBoundary {
    /// Native samples in [start, end), preserving the original sample population.
    #[default]
    Native,
    /// Add exact start/end values using the channel's interpolation policy.
    /// Native samples at those times are included only once. No extrapolation.
    Interpolate,
}

pub struct ReportSeries<'a> {
    pub data: &'a [f64],
    pub freq: u16,
    pub interpolation: Interpolation,
    pub scale: f64,
    pub offset: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportSummary {
    /// Native and (when requested) boundary samples, including missing values.
    pub sample_count: usize,
    pub finite_count: usize,
    /// Native samples in the half-open selection; synthetic endpoints are excluded.
    pub native_count: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub average: Option<f64>,
    pub stddev: Option<f64>,
    /// Missing endpoints stay missing rather than moving to the next finite sample.
    pub start: Option<f64>,
    pub end: Option<f64>,
}
impl ReportSummary {
    pub fn value(&self, statistic: ReportStatistic) -> Option<f64> {
        let value = match statistic {
            ReportStatistic::Min => self.min,
            ReportStatistic::Max => self.max,
            ReportStatistic::Range => self.max.zip(self.min).map(|(a, b)| a - b),
            ReportStatistic::Average => self.average,
            ReportStatistic::AbsMax => self.min.zip(self.max).map(|(a, b)| a.abs().max(b.abs())),
            ReportStatistic::Start => self.start,
            ReportStatistic::End => self.end,
            ReportStatistic::Change => self.end.zip(self.start).map(|(a, b)| a - b),
            ReportStatistic::StdDev => self.stddev,
        };
        value.filter(|v| v.is_finite())
    }
}

/// Union valid intervals so overlapping ranges never count samples twice.
pub fn report_windows(windows: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sorted: Vec<_> = windows
        .iter()
        .copied()
        .filter(|(a, b)| a.is_finite() && b.is_finite() && *b > a.max(0.))
        .map(|(a, b)| (a.max(0.), b))
        .collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut result: Vec<(f64, f64)> = Vec::new();
    for (a, b) in sorted {
        if let Some(last) = result.last_mut()
            && a <= last.1
        {
            last.1 = last.1.max(b);
        } else {
            result.push((a, b));
        }
    }
    result
}

/// Intersect a selection with trusted ranges or section intervals.
pub fn intersect_report_windows(selection: &[(f64, f64)], mask: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let selection = report_windows(selection);
    let mask = report_windows(mask);
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < selection.len() && j < mask.len() {
        let (a, b) = selection[i];
        let (c, d) = mask[j];
        if a.max(c) < b.min(d) {
            result.push((a.max(c), b.min(d)));
        }
        if b < d {
            i += 1;
        } else {
            j += 1;
        }
    }
    result
}

/// Sample-weighted statistics in display units, with population standard deviation.
/// Interpolated boundaries participate in the same population as native samples.
/// Gaps between intervals are never filled or interpolated across. Boundary support
/// may use adjacent source samples; the requested timestamp must be in source bounds.
pub fn report_statistics(
    series: &ReportSeries<'_>,
    windows: &[(f64, f64)],
    boundary: ReportBoundary,
) -> ReportSummary {
    let mut result = ReportSummary::default();
    if series.freq == 0 || series.data.is_empty() {
        return result;
    }
    let mut mean = 0.;
    let mut m2 = 0.;
    let mut push = |raw: Option<f64>, native: bool| {
        let value = raw
            .map(|v| v * series.scale + series.offset)
            .filter(|v| v.is_finite());
        if result.sample_count == 0 {
            result.start = value;
        }
        result.end = value;
        result.sample_count += 1;
        result.native_count += usize::from(native);
        if let Some(v) = value {
            result.finite_count += 1;
            result.min = Some(result.min.map_or(v, |min| min.min(v)));
            result.max = Some(result.max.map_or(v, |max| max.max(v)));
            let delta = v - mean;
            mean += delta / result.finite_count as f64;
            m2 += delta * (v - mean);
        }
    };
    let last_time = (series.data.len() - 1) as f64 / series.freq as f64;
    let sample = |t: f64| {
        if t > last_time {
            None
        } else {
            // Preserve exact grid values beside missing samples even when the
            // timestamp's multiplication back onto its grid rounds slightly.
            let nearest = (t * series.freq as f64).round() as usize;
            if nearest < series.data.len() && nearest as f64 / series.freq as f64 == t {
                Some(series.data[nearest]).filter(|v| v.is_finite())
            } else {
                sample_with_policy(series.data, series.freq, t, series.interpolation)
            }
        }
    };
    for (start, end) in report_windows(windows) {
        let window = sample_window_for_time(series.freq, series.data.len(), start, end);
        if boundary == ReportBoundary::Interpolate {
            let on_grid = window.start_sample < window.end_sample
                && window.start_sample as f64 / series.freq as f64 == start;
            push(sample(start), on_grid);
        }
        for i in window.start_sample..window.end_sample {
            let time = i as f64 / series.freq as f64;
            if boundary != ReportBoundary::Interpolate || time != start {
                push(Some(series.data[i]), true);
            }
        }
        if boundary == ReportBoundary::Interpolate {
            push(sample(end), false);
        }
    }
    if result.finite_count > 0 {
        result.average = mean.is_finite().then_some(mean);
        result.stddev = m2
            .is_finite()
            .then(|| (m2 / result.finite_count as f64).max(0.).sqrt());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn series(data: &[f64]) -> ReportSeries<'_> {
        ReportSeries {
            data,
            freq: 1,
            interpolation: Interpolation::Linear,
            scale: 1.,
            offset: 0.,
        }
    }
    #[test]
    fn all_statistics_use_transformed_values_and_population_deviation() {
        let mut s = series(&[1., 2., 3., 4.]);
        s.scale = -2.;
        s.offset = 5.;
        let r = report_statistics(&s, &[(0., 4.)], ReportBoundary::Native);
        let expected = [-3., 3., 6., 0., 3., 3., -3., -6., 5f64.sqrt()];
        for (stat, expected) in ReportStatistic::ALL.into_iter().zip(expected) {
            assert!(
                (r.value(stat).unwrap() - expected).abs() < 1e-12,
                "{stat:?}"
            );
        }
        assert_eq!((r.sample_count, r.native_count, r.finite_count), (4, 4, 4));
    }
    #[test]
    fn boundaries_are_half_open_or_interpolated_without_duplicate_samples() {
        let s = series(&[0., 10., 20., 30.]);
        let native = report_statistics(&s, &[(0.25, 2.25)], ReportBoundary::Native);
        assert_eq!(
            (native.start, native.end, native.average),
            (Some(10.), Some(20.), Some(15.))
        );
        let r = report_statistics(&s, &[(0.25, 2.25)], ReportBoundary::Interpolate);
        assert_eq!(
            (r.start, r.end, r.average, r.sample_count),
            (Some(2.5), Some(22.5), Some(13.75), 4)
        );
        let r = report_statistics(&s, &[(0., 2.)], ReportBoundary::Interpolate);
        assert_eq!((r.average, r.sample_count), (Some(10.), 3));
        let s = ReportSeries {
            interpolation: Interpolation::Previous,
            ..s
        };
        let r = report_statistics(&s, &[(0.25, 0.75)], ReportBoundary::Interpolate);
        assert_eq!((r.start, r.end), (Some(0.), Some(0.)));
    }
    #[test]
    fn missing_endpoints_and_empty_populations_never_become_zero() {
        let s = series(&[f64::NAN, 4., f64::NAN]);
        let r = report_statistics(&s, &[(0., 3.)], ReportBoundary::Native);
        assert_eq!(
            (r.start, r.end, r.average, r.finite_count),
            (None, None, Some(4.), 1)
        );
        assert_eq!(r.value(ReportStatistic::Change), None);
        let empty = report_statistics(&s, &[], ReportBoundary::Native);
        assert!(
            ReportStatistic::ALL
                .into_iter()
                .all(|s| empty.value(s).is_none())
        );
        let r = report_statistics(&series(&[1., 2.]), &[(0., 2.)], ReportBoundary::Interpolate);
        assert_eq!(r.end, None); // No extrapolation beyond the last timestamp.
    }
    #[test]
    fn trusted_disjoint_and_overlapping_ranges_use_a_union_without_bridging_gaps() {
        let windows = intersect_report_windows(&[(0.5, 7.)], &[(0., 2.), (1., 3.), (5., 6.)]);
        assert_eq!(windows, [(0.5, 3.), (5., 6.)]);
        let r = report_statistics(
            &series(&[0., 1., 2., 99., 99., 5., 6.]),
            &windows,
            ReportBoundary::Native,
        );
        assert_eq!(r.sample_count, 3);
        assert!((r.average.unwrap() - 8. / 3.).abs() < 1e-12);
        let r = report_statistics(
            &series(&[0., 1., 2.]),
            &[(0., 2.), (1., 3.)],
            ReportBoundary::Native,
        );
        assert_eq!((r.sample_count, r.average), (3, Some(1.)));
        assert!(report_windows(&[(f64::NAN, 1.), (2., 1.), (-3., -1.)]).is_empty());
    }
    #[test]
    fn native_boundaries_preserve_off_grid_membership_at_different_rates() {
        for hz in [1, 3, 10, 100] {
            let data: Vec<_> = (0..hz as usize * 2).map(|i| i as f64).collect();
            let s = ReportSeries {
                freq: hz,
                ..series(&data)
            };
            let r = report_statistics(&s, &[(0.51, 1.01)], ReportBoundary::Native);
            let expected: Vec<_> = data
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    let t = *i as f64 / hz as f64;
                    (0.51..1.01).contains(&t)
                })
                .map(|(_, v)| *v)
                .collect();
            assert_eq!(r.sample_count, expected.len());
            assert_eq!(r.start, expected.first().copied());
            assert_eq!(r.end, expected.last().copied());
        }
    }
    #[test]
    fn exact_boundary_samples_beside_missing_values_do_not_require_interpolation() {
        let mut data = vec![f64::NAN; 30];
        data[15] = 10.;
        data[16] = 20.;
        let s = ReportSeries {
            freq: 13,
            ..series(&data)
        };
        let r = report_statistics(&s, &[(15. / 13., 16. / 13.)], ReportBoundary::Interpolate);
        assert_eq!(
            (r.start, r.end, r.sample_count, r.native_count),
            (Some(10.), Some(20.), 2, 1)
        );
    }
}

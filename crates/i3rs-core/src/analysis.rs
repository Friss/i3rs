//! Analysis helpers shared by CLI and non-UI consumers.

use serde::Serialize;

use crate::Lap;

/// Inclusive/exclusive sample range for a time window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SampleWindow {
    pub start_sample: usize,
    pub end_sample: usize,
}

impl SampleWindow {
    pub fn len(self) -> usize {
        self.end_sample.saturating_sub(self.start_sample)
    }

    pub fn is_empty(self) -> bool {
        self.start_sample >= self.end_sample
    }
}

/// Aggregate statistics for a channel slice.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisStats {
    pub sample_count: usize,
    pub finite_count: usize,
    pub missing_count: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub stddev: Option<f64>,
    pub p01: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub rms: Option<f64>,
    /// Trapezoidal integral over time when a frequency is supplied.
    pub integral: Option<f64>,
}

/// One histogram bin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistogramBin {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
}

/// Difference metrics between two aligned series.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComparisonStats {
    pub point_count: usize,
    pub finite_pair_count: usize,
    pub delta_mean: Option<f64>,
    pub max_abs_delta: Option<f64>,
    pub rmse: Option<f64>,
    pub area_delta: Option<f64>,
}

/// One paired sample for scatter-style channel analysis.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScatterPoint {
    pub time: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// Convert a time range in seconds to a bounded sample window.
pub fn sample_window_for_time(
    freq: u16,
    data_len: usize,
    start_time: f64,
    end_time: f64,
) -> SampleWindow {
    if freq == 0 || !start_time.is_finite() || !end_time.is_finite() || end_time <= start_time {
        return SampleWindow {
            start_sample: 0,
            end_sample: 0,
        };
    }

    let hz = freq as f64;
    let start_sample = (start_time.max(0.0) * hz).floor() as usize;
    let end_sample = (end_time.max(0.0) * hz).ceil() as usize;
    SampleWindow {
        start_sample: start_sample.min(data_len),
        end_sample: end_sample.min(data_len),
    }
}

/// Compute finite-value statistics for a channel slice.
pub fn compute_stats(data: &[f64], freq: Option<u16>) -> AnalysisStats {
    let sample_count = data.len();
    let finite: Vec<f64> = data
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    let finite_count = finite.len();
    let missing_count = sample_count.saturating_sub(finite_count);

    if finite.is_empty() {
        return AnalysisStats {
            sample_count,
            finite_count,
            missing_count,
            min: None,
            max: None,
            mean: None,
            stddev: None,
            p01: None,
            p50: None,
            p95: None,
            rms: None,
            integral: None,
        };
    }

    let mut sorted = finite.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));

    let min = *sorted.first().unwrap();
    let max = *sorted.last().unwrap();
    let sum: f64 = finite.iter().sum();
    let mean = sum / finite_count as f64;
    let var_sum: f64 = finite
        .iter()
        .map(|value| {
            let diff = *value - mean;
            diff * diff
        })
        .sum();
    let square_sum: f64 = finite.iter().map(|value| value * value).sum();

    AnalysisStats {
        sample_count,
        finite_count,
        missing_count,
        min: Some(min),
        max: Some(max),
        mean: Some(mean),
        stddev: Some((var_sum / finite_count as f64).sqrt()),
        p01: percentile_sorted(&sorted, 0.01),
        p50: percentile_sorted(&sorted, 0.50),
        p95: percentile_sorted(&sorted, 0.95),
        rms: Some((square_sum / finite_count as f64).sqrt()),
        integral: freq.and_then(|hz| integrate_trapezoidal(data, hz)),
    }
}

fn percentile_sorted(sorted: &[f64], percentile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    if sorted.len() == 1 {
        return Some(sorted[0]);
    }

    let rank = percentile.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(sorted[lo] * (1.0 - frac) + sorted[hi] * frac)
}

fn integrate_trapezoidal(data: &[f64], freq: u16) -> Option<f64> {
    if freq == 0 || data.len() < 2 {
        return None;
    }

    let dt = 1.0 / freq as f64;
    let mut area = 0.0;
    let mut finite_pairs = 0usize;
    for pair in data.windows(2) {
        if pair[0].is_finite() && pair[1].is_finite() {
            area += (pair[0] + pair[1]) * 0.5 * dt;
            finite_pairs += 1;
        }
    }

    (finite_pairs > 0).then_some(area)
}

/// Sample a channel at an absolute session time with linear interpolation.
pub fn sample_at_time(data: &[f64], freq: u16, time: f64) -> Option<f64> {
    if data.is_empty() || freq == 0 || !time.is_finite() || time < 0.0 {
        return None;
    }

    let idx = time * freq as f64;
    let lo = idx.floor() as usize;
    if lo >= data.len() {
        return (idx <= data.len() as f64).then_some(*data.last().unwrap());
    }
    let hi = lo + 1;
    if hi >= data.len() {
        return Some(data[lo]);
    }
    let frac = idx - lo as f64;
    Some(data[lo] * (1.0 - frac) + data[hi] * frac)
}

/// Resample a channel at explicit absolute session times.
pub fn resample_at_times(data: &[f64], freq: u16, times: &[f64]) -> Vec<Option<f64>> {
    times
        .iter()
        .map(|time| sample_at_time(data, freq, *time))
        .collect()
}

/// Pair two channels at explicit absolute session times for scatter analysis.
pub fn scatter_pairs_at_times(
    x_data: &[f64],
    x_freq: u16,
    y_data: &[f64],
    y_freq: u16,
    times: &[f64],
) -> Vec<ScatterPoint> {
    times
        .iter()
        .map(|time| ScatterPoint {
            time: *time,
            x: sample_at_time(x_data, x_freq, *time).filter(|value| value.is_finite()),
            y: sample_at_time(y_data, y_freq, *time).filter(|value| value.is_finite()),
        })
        .collect()
}

/// Produce evenly-spaced absolute times for a range.
pub fn evenly_spaced_times(start: f64, end: f64, points: usize) -> Vec<f64> {
    match points {
        0 => Vec::new(),
        1 => vec![start],
        _ => {
            let span = end - start;
            (0..points)
                .map(|idx| start + span * idx as f64 / (points - 1) as f64)
                .collect()
        }
    }
}

/// Produce fixed-Hz absolute times for a range. The end is exclusive.
pub fn fixed_hz_times(start: f64, end: f64, hz: u16) -> Vec<f64> {
    if hz == 0 || !start.is_finite() || !end.is_finite() || end <= start {
        return Vec::new();
    }

    let count = ((end - start) * hz as f64).ceil() as usize;
    let dt = 1.0 / hz as f64;
    (0..count).map(|idx| start + idx as f64 * dt).collect()
}

/// Build histogram bins from finite values.
pub fn histogram_bins(data: &[f64], bins: usize) -> Vec<HistogramBin> {
    if bins == 0 {
        return Vec::new();
    }

    let finite: Vec<f64> = data
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if finite.is_empty() {
        return Vec::new();
    }

    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min == max {
        return vec![HistogramBin {
            lower: min,
            upper: max,
            count: finite.len(),
        }];
    }

    let width = (max - min) / bins as f64;
    let mut counts = vec![0usize; bins];
    for value in finite {
        let mut idx = ((value - min) / width).floor() as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }

    counts
        .into_iter()
        .enumerate()
        .map(|(idx, count)| HistogramBin {
            lower: min + width * idx as f64,
            upper: if idx + 1 == bins {
                max
            } else {
                min + width * (idx + 1) as f64
            },
            count,
        })
        .collect()
}

/// Compare two aligned series using finite pairs only.
pub fn compare_aligned(
    reference: &[Option<f64>],
    comparison: &[Option<f64>],
    x_step: f64,
) -> ComparisonStats {
    let point_count = reference.len().min(comparison.len());
    let mut finite_pair_count = 0usize;
    let mut delta_sum = 0.0;
    let mut square_sum = 0.0;
    let mut max_abs_delta = 0.0f64;
    let mut area_delta = 0.0;

    for idx in 0..point_count {
        let (Some(reference), Some(comparison)) = (reference[idx], comparison[idx]) else {
            continue;
        };
        if !reference.is_finite() || !comparison.is_finite() {
            continue;
        }
        let delta = comparison - reference;
        finite_pair_count += 1;
        delta_sum += delta;
        square_sum += delta * delta;
        max_abs_delta = max_abs_delta.max(delta.abs());
        area_delta += delta * x_step;
    }

    if finite_pair_count == 0 {
        return ComparisonStats {
            point_count,
            finite_pair_count,
            delta_mean: None,
            max_abs_delta: None,
            rmse: None,
            area_delta: None,
        };
    }

    ComparisonStats {
        point_count,
        finite_pair_count,
        delta_mean: Some(delta_sum / finite_pair_count as f64),
        max_abs_delta: Some(max_abs_delta),
        rmse: Some((square_sum / finite_pair_count as f64).sqrt()),
        area_delta: Some(area_delta),
    }
}

/// Find a lap by number or display name.
pub fn find_lap<'a>(laps: &'a [Lap], selector: &str) -> Option<(usize, &'a Lap)> {
    if let Ok(number) = selector.parse::<u32>() {
        return laps
            .iter()
            .enumerate()
            .find(|(_, lap)| lap.number == number);
    }

    let normalized = selector.trim().to_ascii_lowercase();
    laps.iter()
        .enumerate()
        .find(|(_, lap)| lap.name.to_ascii_lowercase() == normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_ignore_non_finite_values() {
        let stats = compute_stats(&[1.0, f64::NAN, 3.0, f64::INFINITY], Some(1));
        assert_eq!(stats.sample_count, 4);
        assert_eq!(stats.finite_count, 2);
        assert_eq!(stats.missing_count, 2);
        assert_eq!(stats.min, Some(1.0));
        assert_eq!(stats.max, Some(3.0));
        assert_eq!(stats.mean, Some(2.0));
        assert_eq!(stats.p50, Some(2.0));
    }

    #[test]
    fn sample_window_clamps_to_data_len() {
        let window = sample_window_for_time(10, 50, 1.2, 9.0);
        assert_eq!(window.start_sample, 12);
        assert_eq!(window.end_sample, 50);
    }

    #[test]
    fn sample_at_time_interpolates() {
        let data = [0.0, 10.0, 20.0];
        assert_eq!(sample_at_time(&data, 10, 0.05), Some(5.0));
    }

    #[test]
    fn histogram_counts_values() {
        let bins = histogram_bins(&[0.0, 1.0, 2.0, 3.0], 2);
        assert_eq!(bins.len(), 2);
        assert_eq!(bins[0].count, 2);
        assert_eq!(bins[1].count, 2);
    }

    #[test]
    fn scatter_pairs_resample_both_axes() {
        let pairs = scatter_pairs_at_times(&[0.0, 10.0], 10, &[100.0, 200.0], 20, &[0.05]);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].x, Some(5.0));
        assert_eq!(pairs[0].y, Some(200.0));
    }

    #[test]
    fn compare_uses_finite_pairs_only() {
        let reference = vec![Some(1.0), Some(2.0), None];
        let comparison = vec![Some(2.0), Some(4.0), Some(6.0)];
        let stats = compare_aligned(&reference, &comparison, 0.5);
        assert_eq!(stats.point_count, 3);
        assert_eq!(stats.finite_pair_count, 2);
        assert_eq!(stats.delta_mean, Some(1.5));
        assert_eq!(stats.max_abs_delta, Some(2.0));
    }

    #[test]
    fn find_lap_numeric_selector_uses_reported_lap_number() {
        let laps = vec![
            Lap {
                number: 0,
                name: "Out Lap".into(),
                start_time: 0.0,
                end_time: 10.0,
            },
            Lap {
                number: 1,
                name: "Lap 1".into(),
                start_time: 10.0,
                end_time: 70.0,
            },
        ];

        let (_, lap) = find_lap(&laps, "1").expect("Lap 1 should be found");

        assert_eq!(lap.name, "Lap 1");
    }
}

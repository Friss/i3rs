//! Min-max downsampling for efficient chart rendering.
//!
//! Given N samples and a target pixel width, produces (min, max) pairs
//! per bucket that preserve all visual peaks and valleys.

/// A downsampled point representing one pixel-width bucket.
#[derive(Debug, Clone, Copy)]
pub struct DownsampledPoint {
    /// Time at the center of this bucket (seconds from start).
    pub time: f64,
    /// Minimum value in this bucket.
    pub min: f64,
    /// Maximum value in this bucket.
    pub max: f64,
}

/// Downsample a slice of evenly-spaced samples to a target number of buckets.
///
/// - `samples`: scaled channel data values
/// - `freq`: sample rate in Hz (to compute time axis)
/// - `start_sample`: index of the first sample (for time offset calculation)
/// - `target_width`: desired number of output points (typically chart pixel width)
///
/// If the sample count is small enough (≤ 2 * target_width), returns one point
/// per sample with min == max. Otherwise, performs min-max decimation.
pub fn downsample_minmax(
    samples: &[f64],
    freq: u16,
    start_sample: usize,
    target_width: usize,
) -> Vec<DownsampledPoint> {
    if samples.is_empty() || target_width == 0 || freq == 0 {
        return vec![];
    }

    let freq_f = freq as f64;
    let n = samples.len();

    // If we have few enough samples, return raw points (no downsampling needed)
    if n <= target_width * 2 {
        return samples
            .iter()
            .enumerate()
            .map(|(i, &v)| DownsampledPoint {
                time: (start_sample + i) as f64 / freq_f,
                min: v,
                max: v,
            })
            .collect();
    }

    // Min-max decimation: divide samples into `target_width` buckets
    let mut result = Vec::with_capacity(target_width);
    let bucket_size_f = n as f64 / target_width as f64;

    for bucket in 0..target_width {
        let start = (bucket as f64 * bucket_size_f) as usize;
        let end = ((bucket + 1) as f64 * bucket_size_f) as usize;
        let end = end.min(n);

        if start >= end {
            continue;
        }

        // Split buckets at missing samples. Ignoring NaNs here would draw a
        // continuous trace through outages, including single-sample gaps.
        let mut run_start = start;
        while run_start < end {
            let finite = samples[run_start].is_finite();
            let mut run_end = run_start + 1;
            while run_end < end && samples[run_end].is_finite() == finite {
                run_end += 1;
            }
            let (min_v, max_v) = if finite {
                samples[run_start..run_end]
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                        (lo.min(v), hi.max(v))
                    })
            } else {
                (f64::NAN, f64::NAN)
            };
            result.push(DownsampledPoint {
                time: (start_sample + (run_start + run_end) / 2) as f64 / freq_f,
                min: min_v,
                max: max_v,
            });
            run_start = run_end;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_downsampling_needed() {
        let samples: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let result = downsample_minmax(&samples, 10, 0, 100);
        assert_eq!(result.len(), 100);
        assert_eq!(result[0].min, 0.0);
        assert_eq!(result[0].max, 0.0);
        assert!((result[0].time - 0.0).abs() < 1e-9);
        assert!((result[50].time - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_minmax_decimation() {
        // 1000 samples, target 10 buckets = 100 samples per bucket
        let samples: Vec<f64> = (0..1000).map(|i| (i as f64 * 0.01).sin()).collect();
        let result = downsample_minmax(&samples, 100, 0, 10);
        assert_eq!(result.len(), 10);
        // Each bucket should have min <= max
        for p in &result {
            assert!(p.min <= p.max);
        }
    }

    #[test]
    fn test_empty() {
        let result = downsample_minmax(&[], 10, 0, 100);
        assert!(result.is_empty());
    }

    #[test]
    fn test_with_offset() {
        let samples: Vec<f64> = vec![1.0, 2.0, 3.0];
        let result = downsample_minmax(&samples, 10, 500, 100);
        assert_eq!(result.len(), 3);
        assert!((result[0].time - 50.0).abs() < 1e-9); // sample 500 / 10 Hz
    }

    #[test]
    fn decimation_preserves_gaps_even_inside_a_single_bucket() {
        let samples = [1.0, 2.0, f64::NAN, 8.0, 9.0, f64::NAN];
        let result = downsample_minmax(&samples, 1, 0, 1);
        assert_eq!(result.len(), 4);
        assert_eq!((result[0].min, result[0].max), (1.0, 2.0));
        assert!(result[1].min.is_nan());
        assert_eq!((result[2].min, result[2].max), (8.0, 9.0));
        assert!(result[3].max.is_nan());
    }
}

//! FFT (Fast Fourier Transform) computation for frequency analysis.

use rustfft::{FftPlanner, num_complex::Complex};

/// Result of an FFT computation: frequency bins and their magnitudes.
pub struct FftResult {
    /// Frequency values in Hz.
    pub frequencies: Vec<f64>,
    /// Magnitude (amplitude) at each frequency.
    pub magnitudes: Vec<f64>,
}

/// Compute the FFT of a time-domain signal.
///
/// - `data`: time-domain samples
/// - `sample_rate`: sampling frequency in Hz
///
/// Returns frequency/magnitude pairs for the positive-frequency half of the spectrum.
/// Applies a Hann window to reduce spectral leakage.
pub fn compute_fft(data: &[f64], sample_rate: f64) -> FftResult {
    let n = data.len();
    if n == 0 {
        return FftResult {
            frequencies: Vec::new(),
            magnitudes: Vec::new(),
        };
    }

    // Apply Hann window
    let mut buffer: Vec<Complex<f64>> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let window = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
            Complex::new(v * window, 0.0)
        })
        .collect();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buffer);

    // Only take positive frequencies (first half)
    let half = n / 2;
    let freq_resolution = sample_rate / n as f64;

    let frequencies: Vec<f64> = (0..half).map(|i| i as f64 * freq_resolution).collect();
    let magnitudes: Vec<f64> = buffer[..half]
        .iter()
        .map(|c| 2.0 * c.norm() / n as f64)
        .collect();

    FftResult {
        frequencies,
        magnitudes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fft_sine_wave() {
        // Generate a 10 Hz sine wave sampled at 100 Hz for 1 second
        let sample_rate = 100.0;
        let n = 100;
        let freq = 10.0;
        let data: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin())
            .collect();

        let result = compute_fft(&data, sample_rate);
        assert_eq!(result.frequencies.len(), n / 2);

        // Peak should be near 10 Hz
        let peak_idx = result
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_freq = result.frequencies[peak_idx];
        assert!(
            (peak_freq - 10.0).abs() < 2.0,
            "Peak frequency should be near 10 Hz, got {}",
            peak_freq
        );
    }

    #[test]
    fn test_fft_empty() {
        let result = compute_fft(&[], 100.0);
        assert!(result.frequencies.is_empty());
        assert!(result.magnitudes.is_empty());
    }
}

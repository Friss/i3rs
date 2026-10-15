//! Shared color bands for sample-level and binned distribution views.
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ColorBands {
    pub min: f64,
    pub max: f64,
    pub count: usize,
    #[serde(default)]
    pub centered: bool,
}

impl Default for ColorBands {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            count: 11,
            centered: true,
        }
    }
}

impl ColorBands {
    pub fn valid(self) -> Self {
        if !self.min.is_finite() || !self.max.is_finite() || self.min >= self.max {
            return Self::default();
        }
        Self {
            count: self.count.clamp(1, 20),
            ..self
        }
    }
    // Underflow, N normal bands, overflow, missing color. Non-centered bands
    // are [lower, upper), except the last includes the outer upper endpoint.
    pub fn len(self) -> usize {
        self.valid().count + 3
    }
    /// Width of one normal band.
    fn step(self) -> f64 {
        let b = self.valid();
        let divisions = if b.centered {
            b.count.saturating_sub(1).max(1)
        } else {
            b.count
        };
        (b.max - b.min) / divisions as f64
    }
    /// Outer edges of the normal bands; centered bands extend half a step past min/max.
    fn edges(self) -> (f64, f64) {
        let b = self.valid();
        let half = if b.centered { self.step() * 0.5 } else { 0.0 };
        (b.min - half, b.max + half)
    }
    pub fn index(self, value: f64) -> usize {
        let b = self.valid();
        if !value.is_finite() {
            return b.count + 2;
        }
        let (lower, upper) = self.edges();
        if value < lower {
            return 0;
        }
        if value > upper {
            return b.count + 1;
        }
        (((value - lower) / self.step()).floor() as usize).min(b.count - 1) + 1
    }
    pub fn label(self, index: usize) -> String {
        let b = self.valid();
        let (lower, upper) = self.edges();
        let step = self.step();
        if index == 0 {
            format!("< {lower:.1}")
        } else if index == b.count + 1 {
            format!("> {upper:.1}")
        } else if index > b.count + 1 {
            "Missing".into()
        } else if b.centered {
            format!("{:.1}", b.min + (index - 1) as f64 * step)
        } else {
            format!(
                "{:.1}–{:.1}",
                b.min + (index - 1) as f64 * step,
                b.min + index as f64 * step
            )
        }
    }
    pub fn color(self, index: usize) -> egui::Color32 {
        let b = self.valid();
        if index > b.count + 1 {
            return egui::Color32::GRAY;
        }
        let f = index as f32 / (b.count + 1) as f32;
        let (r, g, blue) = i3rs_core::track::hsv_to_rgb((1.0 - f) * 250.0, 0.9, 1.0);
        egui::Color32::from_rgb(r, g, blue)
    }
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        ui.label("Color range:");
        ui.add(egui::DragValue::new(&mut self.min).speed(1.0));
        ui.add(egui::DragValue::new(&mut self.max).speed(1.0));
        ui.label("Bands:");
        ui.add(egui::DragValue::new(&mut self.count).range(1..=20));
        ui.checkbox(&mut self.centered, "Centered");
        *self = self.valid();
    }
    pub fn key(self) -> (u64, u64, usize, bool) {
        let b = self.valid();
        (b.min.to_bits(), b.max.to_bits(), b.count, b.centered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bands_account_for_boundaries_outliers_and_missing_values() {
        let b = ColorBands {
            count: 10,
            centered: false,
            ..ColorBands::default()
        };
        let values = [f64::NAN, -1.0, 0.0, 9.9, 10.0, 99.9, 100.0, 100.1];
        let indices: Vec<_> = values.into_iter().map(|v| b.index(v)).collect();
        assert_eq!(indices, [12, 0, 1, 1, 2, 10, 10, 11]);
        assert_eq!(b.label(11), "> 100.0");
        let b = ColorBands::default();
        assert_eq!(
            [0., 4.99, 5., 95., 100.].map(|v| b.index(v)),
            [1, 1, 2, 11, 11]
        );
        assert_eq!(b.index(105.0), 11);
        assert_eq!(b.index(105.1), 12);
    }
}

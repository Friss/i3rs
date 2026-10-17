//! Shared color bands for sample-level and binned distribution views.
use eframe::egui;
use serde::{Deserialize, Serialize};

/// A component's own selection predicate, independent of expression truth rules.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentFilter {
    pub gate: String,
    pub trusted_only: bool,
}

#[derive(Clone)]
pub struct PreparedFilter {
    pub constant: Option<f64>,
    pub trusted_ranges: Option<Vec<(f64, f64)>>,
    pub gate: Option<crate::state::PlottedChannelInfo>,
    pub available: bool,
    pub key: String,
}
impl PreparedFilter {
    pub fn includes(&self, time: f64) -> bool {
        self.available
            && self.constant.is_none_or(i3rs_core::component_gate_includes)
            && self
                .trusted_ranges
                .as_ref()
                .is_none_or(|ranges| ranges.iter().any(|(a, b)| time >= *a && time < *b))
            && self.gate.as_ref().is_none_or(|g| {
                i3rs_core::channels::sample_with_policy(&g.data, g.freq, time, g.interpolation)
                    .is_some_and(i3rs_core::component_gate_includes)
            })
    }
}
impl ComponentFilter {
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Gate channel (> 0):");
            ui.add(egui::TextEdit::singleline(&mut self.gate).desired_width(160.));
            ui.checkbox(&mut self.trusted_only, "Trusted ranges only");
        });
    }
    pub fn prepare(&self, ui: &mut egui::Ui, shared: &crate::state::SharedState) -> PreparedFilter {
        let mut constant = None;
        let mut generated = None;
        let gate = if self.gate.trim().is_empty() {
            None
        } else {
            let id = match shared.resolve_analysis_reference(self.gate.trim()) {
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Physical(i),
                    ..
                } => Some(crate::state::ChannelId::Physical(i)),
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Derived(id),
                    ..
                } => shared
                    .math_channel_index_by_id(id)
                    .map(crate::state::ChannelId::Math),
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Constant(idx),
                    ..
                } => {
                    constant = Some(shared.channel_settings.constants[idx].value);
                    None
                }
                i3rs_core::ChannelResolution::Resolved {
                    key: i3rs_core::ChannelKey::Generated(name),
                    ..
                } => {
                    shared.request_track_data_build();
                    generated = match name.as_str() {
                        "Corr Speed" => shared.correction_speed.clone(),
                        "Corr GPS Speed" => shared.correction_gps_speed.clone(),
                        _ => shared.correction_distance.clone(),
                    };
                    None
                }
                _ => None,
            };
            id.and_then(|id| super::utils::create_plotted_channel(id, shared, 0))
                .map(|pc| super::utils::build_plotted_channel_info(&pc, shared))
                .or_else(|| {
                    generated.map(|(data, freq)| crate::state::PlottedChannelInfo {
                        channel_id: crate::state::ChannelId::Physical(usize::MAX),
                        name: self.gate.clone(),
                        unit: String::new(),
                        freq,
                        dec_places: 3,
                        color: egui::Color32::GRAY,
                        data,
                        display_scale: 1.,
                        display_offset: 0.,
                        interpolation: i3rs_core::Interpolation::Linear,
                        enum_labels: Default::default(),
                    })
                })
        };
        let trusted_ranges = self.trusted_only.then(|| {
            shared
                .current_analysis()
                .into_iter()
                .flat_map(|a| &a.ranges)
                .filter(|r| r.trust == i3rs_core::ranges::Trust::Trusted)
                .map(|r| (r.start, r.end))
                .collect::<Vec<_>>()
        });
        let trusted = trusted_ranges
            .as_ref()
            .is_none_or(|ranges| !ranges.is_empty());
        let ready = self.gate.trim().is_empty()
            || constant.is_some_and(f64::is_finite)
            || gate.as_ref().is_some_and(|g| !g.data.is_empty());
        if !ready {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!(
                    "Gate '{}' unavailable or loading; no samples included",
                    self.gate
                ),
            );
        }
        if !trusted {
            ui.weak("Selected range is not trusted; no samples included");
        }
        let key = format!(
            "{}:{:?}:{:?}:{}:{:?}",
            self.gate,
            trusted_ranges,
            constant,
            ready,
            gate.as_ref().map(|g| (
                g.data.as_ptr() as usize,
                g.freq,
                g.display_scale.to_bits(),
                g.display_offset.to_bits(),
                g.interpolation
            ))
        );
        PreparedFilter {
            gate,
            constant,
            trusted_ranges,
            available: ready && trusted,
            key,
        }
    }
}

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

#[cfg(test)]
mod population_tests {
    use super::*;
    #[test]
    fn gates_use_result_values_and_trust_is_half_open() {
        let info = crate::state::PlottedChannelInfo {
            channel_id: crate::state::ChannelId::Math(0),
            name: "gate".into(),
            unit: String::new(),
            freq: 1.0,
            dec_places: 0,
            color: egui::Color32::WHITE,
            data: std::sync::Arc::from(vec![-1., 0., 0.5, f64::NAN, 1.]),
            display_scale: -1.,
            display_offset: 0.,
            interpolation: i3rs_core::Interpolation::Previous,
            enum_labels: Default::default(),
        };
        let filter = PreparedFilter {
            gate: Some(info),
            constant: None,
            trusted_ranges: Some(vec![(0., 3.)]),
            available: true,
            key: String::new(),
        };
        assert_eq!(
            [0., 1., 2., 3., 4.].map(|t| filter.includes(t)),
            [false, false, true, false, false]
        );
    }
    #[test]
    fn negative_constants_and_unavailable_gates_exclude_all_samples() {
        let mut filter = PreparedFilter {
            gate: None,
            constant: Some(-1.),
            trusted_ranges: None,
            available: true,
            key: String::new(),
        };
        assert!(!filter.includes(1.));
        filter.constant = Some(1.);
        assert!(filter.includes(1.));
        filter.available = false;
        assert!(!filter.includes(1.));
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BandPresentation {
    pub hidden: Vec<usize>,
    pub colors: std::collections::BTreeMap<usize, [u8; 3]>,
}
impl BandPresentation {
    pub fn visible(&self, index: usize) -> bool {
        !self.hidden.contains(&index)
    }
    pub fn color(&self, bands: ColorBands, index: usize) -> egui::Color32 {
        self.colors.get(&index).map_or_else(
            || bands.color(index),
            |c| egui::Color32::from_rgb(c[0], c[1], c[2]),
        )
    }
    pub fn controls(&mut self, ui: &mut egui::Ui, bands: ColorBands) {
        ui.collapsing("Band visibility / colours (totals unchanged)", |ui| {
            egui::ScrollArea::vertical()
                .max_height(160.)
                .show(ui, |ui| {
                    for index in 0..bands.len() {
                        ui.horizontal(|ui| {
                            let mut visible = self.visible(index);
                            if ui.checkbox(&mut visible, bands.label(index)).changed() {
                                self.hidden.retain(|i| *i != index);
                                if !visible {
                                    self.hidden.push(index);
                                }
                            }
                            let c = self.color(bands, index);
                            let mut color = [c.r(), c.g(), c.b()];
                            if ui.color_edit_button_srgb(&mut color).changed() {
                                self.colors.insert(index, color);
                            }
                        });
                    }
                });
            if ui.button("Reset band presentation").clicked() {
                *self = Default::default();
            }
        });
    }
}

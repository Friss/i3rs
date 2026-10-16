//! Interval and suspension results share the exact displayed/exported population.
use super::{distribution::ComponentFilter, utils::*};
use crate::state::{PlottedChannel, SharedState};
use eframe::egui;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalculationMode {
    #[default]
    IntervalAverages,
    Suspension,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistributionOutput {
    #[default]
    Count,
    Percent,
    Seconds,
}
struct VelocitySeries {
    name: String,
    unit: String,
    color: egui::Color32,
    hz: u16,
    range: (f64, f64),
    values: Vec<f64>,
}
pub struct CalculationsPanel {
    pub id: u64,
    pub title: String,
    pub channels: Vec<PlottedChannel>,
    pub filter: ComponentFilter,
    pub mode: CalculationMode,
    pub interval_seconds: f64,
    pub suspension: i3rs_core::distribution::SuspensionSettings,
    pub suspension_bins: usize,
    pub suspension_output: DistributionOutput,
    pub suspension_line: bool,
    velocities: Vec<VelocitySeries>,
    picker: String,
    cache_key: String,
    rows: Vec<ResultRow>,
    csv: String,
}
impl CalculationsPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            channels: Vec::new(),
            filter: Default::default(),
            mode: Default::default(),
            interval_seconds: 1.,
            suspension: Default::default(),
            suspension_bins: 41,
            suspension_output: DistributionOutput::Count,
            suspension_line: false,
            velocities: Vec::new(),
            picker: String::new(),
            cache_key: String::new(),
            rows: Vec::new(),
            csv: String::new(),
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.mode,
                CalculationMode::IntervalAverages,
                "Interval averages",
            );
            ui.selectable_value(&mut self.mode, CalculationMode::Suspension, "Suspension");
            let mut choice = None;
            if channel_picker(
                ui,
                ("calculations-picker", self.id),
                &mut choice,
                &mut self.picker,
                shared,
            ) && let Some(id) = choice
                && let Some(pc) = create_plotted_channel(id, shared, self.channels.len())
            {
                self.channels.push(pc);
            }
        });
        if let Some(id) = shared
            .dragging_channel
            .filter(|_| ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_released()))
        {
            shared.dragging_channel = None;
            if let Some(pc) = create_plotted_channel(id, shared, self.channels.len()) {
                self.channels.push(pc);
            }
        }
        self.filter.controls(ui);
        let filter = self.filter.prepare(ui, shared);
        if self.mode == CalculationMode::IntervalAverages {
            ui.horizontal(|ui| {
                ui.label("Interval width (s)");
                ui.add(egui::DragValue::new(&mut self.interval_seconds).range(0.01..=3600.0));
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut self.suspension.input_is_velocity,
                    "Input is already velocity",
                );
                ui.checkbox(&mut self.suspension.invert, "Invert sign (positive = bump)");
                ui.label("Filter window (s)");
                ui.add(
                    egui::DragValue::new(&mut self.suspension.filter_seconds)
                        .range(0.0..=5.0)
                        .speed(0.01),
                );
                ui.label("Stationary ±");
                ui.add(egui::DragValue::new(&mut self.suspension.deadband).range(0.0..=1000.0));
                ui.label("Low/high split (velocity units)");
                ui.add(
                    egui::DragValue::new(&mut self.suspension.split_velocity).range(0.0..=10000.0),
                );
            });
        }
        if self.mode == CalculationMode::Suspension {
            ui.horizontal_wrapped(|ui| {
                ui.label("Velocity bins");
                ui.add(egui::DragValue::new(&mut self.suspension_bins).range(3..=201));
                ui.checkbox(&mut self.suspension_line, "Line histogram");
                for (output, label) in [
                    (DistributionOutput::Count, "Count"),
                    (DistributionOutput::Percent, "Percent"),
                    (DistributionOutput::Seconds, "Seconds"),
                ] {
                    ui.selectable_value(&mut self.suspension_output, output, label);
                }
            });
        }
        let range = shared.analysis_range();
        let mut populations = Vec::new();
        let mut remove = None;
        for (index, pc) in self.channels.iter_mut().enumerate() {
            refresh_plotted_channel(pc, shared);
            let info = build_plotted_channel_info(pc, shared);
            ui.horizontal(|ui| {
                ui.colored_label(pc.color, &info.name);
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
            if info.freq == 0 || info.data.is_empty() {
                ui.weak("Channel unavailable or loading");
                continue;
            }
            populations.push((info, range, filter.clone()));
        }
        let slots: Vec<_> = self.channels.iter().collect();
        for overlay in crate::comparison::overlay_populations(shared, &slots, &self.filter) {
            for mut info in overlay.channels {
                info.name = format!("{} · {}", overlay.label, info.name);
                info.color = overlay.color;
                populations.push((info, overlay.range, overlay.filter.clone()));
            }
        }
        if shared
            .comparisons
            .sessions
            .iter()
            .any(|s| !s.requests.is_empty())
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        let key = format!(
            "{:?}:{}:{:?}:{:?}",
            self.mode,
            self.interval_seconds,
            self.suspension,
            populations
                .iter()
                .map(|(i, r, f)| (
                    &i.name,
                    &i.unit,
                    i.data.as_ptr() as usize,
                    i.data.len(),
                    i.freq,
                    i.display_scale,
                    i.display_offset,
                    r,
                    &f.key,
                    &f.trusted_ranges,
                    f.constant,
                    f.gate
                        .as_ref()
                        .map(|g| (g.data.as_ptr() as usize, g.freq, g.interpolation))
                ))
                .collect::<Vec<_>>()
        );
        if key != self.cache_key {
            self.rows.clear();
            self.velocities.clear();
            for (info, range, filter) in populations {
                let mut data: Vec<_> = info.data.iter().map(|v| info.transform_value(*v)).collect();
                if self.mode == CalculationMode::Suspension {
                    data = i3rs_core::distribution::filtered_suspension_velocity(
                        &data,
                        info.freq,
                        self.suspension,
                    );
                }
                for (i, v) in data.iter_mut().enumerate() {
                    if !filter.includes(i as f64 / info.freq as f64) {
                        *v = f64::NAN;
                    }
                }
                if self.mode == CalculationMode::Suspension {
                    let window =
                        i3rs_core::sample_window_for_time(info.freq, data.len(), range.0, range.1);
                    self.velocities.push(VelocitySeries {
                        name: info.name.clone(),
                        unit: if self.suspension.input_is_velocity {
                            info.unit.clone()
                        } else {
                            format!("{}/s", info.unit)
                        },
                        color: info.color,
                        hz: info.freq,
                        range,
                        values: data[window.start_sample..window.end_sample]
                            .iter()
                            .copied()
                            .filter(|v| v.is_finite())
                            .collect(),
                    });
                }
                append_results(
                    &mut self.rows,
                    &info.name,
                    &info.unit,
                    &data,
                    info.freq,
                    range,
                    self.mode,
                    self.interval_seconds,
                    self.suspension,
                );
            }
            self.csv = result_csv(&self.rows);
            self.cache_key = key;
        }
        if let Some(i) = remove {
            self.channels.remove(i);
        }
        let csv = &self.csv;
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} results · missing and excluded samples are omitted",
                self.rows.len()
            ));
            export_buttons(ui, csv);
        });
        if self.mode == CalculationMode::Suspension {
            self.show_velocity_distributions(ui);
        }
        let rows = &self.rows;
        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new(("calculation-results", self.id))
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        "Channel", "Result", "Start s", "End s", "Count", "Value", "Unit",
                    ] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for r in rows {
                        for value in [
                            &r.channel,
                            &r.result,
                            &format!("{:.6}", r.start),
                            &format!("{:.6}", r.end),
                            &r.count.to_string(),
                            &r.value
                                .map_or_else(|| "undefined".into(), |v| format!("{v:.9}")),
                            &r.unit,
                        ] {
                            ui.label(value);
                        }
                        ui.end_row();
                    }
                });
        });
    }
    fn show_velocity_distributions(&self, ui: &mut egui::Ui) {
        use egui_plot::{Bar, BarChart, Legend, Line, Plot, VLine};
        let units: std::collections::BTreeSet<_> =
            self.velocities.iter().map(|v| v.unit.clone()).collect();
        for unit in units {
            let series: Vec<_> = self.velocities.iter().filter(|v| v.unit == unit).collect();
            let limit = series
                .iter()
                .flat_map(|v| &v.values)
                .map(|v| v.abs())
                .fold(self.suspension.split_velocity.abs() * 1.2, f64::max)
                .max(1.)
                * 1.001;
            let histograms: Vec<_> = series
                .iter()
                .map(|v| {
                    i3rs_core::distribution::suspension_histogram(
                        &v.values,
                        self.suspension_bins.clamp(3, 201),
                        limit,
                    )
                })
                .collect();
            let mut csv = String::new();
            for (index, (series, bins)) in series.iter().zip(&histograms).enumerate() {
                csv.push_str(&i3rs_core::distribution::suspension_histogram_csv(
                    &series.name,
                    &unit,
                    series.hz,
                    series.range,
                    bins,
                    index == 0,
                ));
            }
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Suspension velocity ({unit}) · positive bump / negative rebound"
                ));
                export_buttons(ui, &csv);
            });
            Plot::new(("suspension-distribution", self.id, &unit))
                .height((ui.available_height() * 0.45).clamp(140., 280.))
                .legend(Legend::default())
                .x_axis_label(&unit)
                .y_axis_label(format!("{:?}", self.suspension_output))
                .show(ui, |plot| {
                    for (series, bins) in series.iter().zip(&histograms) {
                        let total = series.values.len();
                        let value = |n: usize| match self.suspension_output {
                            DistributionOutput::Count => n as f64,
                            DistributionOutput::Percent => 100. * n as f64 / total.max(1) as f64,
                            DistributionOutput::Seconds => n as f64 / series.hz.max(1) as f64,
                        };
                        let label = format!("{} · {} samples", series.name, total);
                        if self.suspension_line {
                            plot.line(
                                Line::new(
                                    label,
                                    bins.iter()
                                        .map(|b| [(b.lower + b.upper) / 2., value(b.count)])
                                        .collect::<Vec<_>>(),
                                )
                                .color(series.color),
                            );
                        } else {
                            plot.bar_chart(
                                BarChart::new(
                                    label,
                                    bins.iter()
                                        .map(|b| {
                                            Bar::new((b.lower + b.upper) / 2., value(b.count))
                                                .width((b.upper - b.lower) * 0.9)
                                                .fill(series.color.gamma_multiply(0.65))
                                        })
                                        .collect(),
                                )
                                .color(series.color),
                            );
                        }
                    }
                    for (name, value) in [
                        ("Rebound high/low", -self.suspension.split_velocity.abs()),
                        ("Bump low/high", self.suspension.split_velocity.abs()),
                        ("Stationary lower", -self.suspension.deadband.abs()),
                        ("Stationary upper", self.suspension.deadband.abs()),
                    ] {
                        plot.vline(
                            VLine::new(name, value)
                                .color(egui::Color32::GRAY)
                                .style(egui_plot::LineStyle::dashed_dense()),
                        );
                    }
                });
        }
    }
}
pub struct ResultRow {
    pub channel: String,
    pub result: String,
    pub start: f64,
    pub end: f64,
    pub count: usize,
    pub value: Option<f64>,
    pub unit: String,
}
#[allow(clippy::too_many_arguments)]
fn append_results(
    rows: &mut Vec<ResultRow>,
    name: &str,
    unit: &str,
    data: &[f64],
    hz: u16,
    range: (f64, f64),
    mode: CalculationMode,
    width: f64,
    settings: i3rs_core::distribution::SuspensionSettings,
) {
    if mode == CalculationMode::IntervalAverages {
        let n = ((range.1 - range.0) / width.max(0.01))
            .ceil()
            .clamp(0., 100000.) as usize;
        let intervals: Vec<_> = (0..n)
            .map(|i| {
                let a = range.0 + i as f64 * width.max(0.01);
                (a, (a + width.max(0.01)).min(range.1))
            })
            .collect();
        for r in i3rs_core::distribution::interval_averages(data, hz, &intervals, None) {
            rows.push(ResultRow {
                channel: name.into(),
                result: "mean".into(),
                start: r.start,
                end: r.end,
                count: r.count,
                value: r.mean,
                unit: unit.into(),
            });
        }
    } else {
        let velocity = data;
        let w = i3rs_core::sample_window_for_time(hz, velocity.len(), range.0, range.1);
        let s = i3rs_core::distribution::suspension_split(
            &velocity[w.start_sample..w.end_sample],
            settings,
        );
        for (label, count, mean) in [
            ("low bump", s.low_bump, None),
            ("high bump", s.high_bump, None),
            ("low rebound", s.low_rebound, None),
            ("high rebound", s.high_rebound, None),
            ("stationary", s.summary.stationary_count, None),
            ("bump mean", s.summary.bump_count, s.summary.bump_mean),
            (
                "rebound mean",
                s.summary.rebound_count,
                s.summary.rebound_mean,
            ),
        ] {
            rows.push(ResultRow {
                channel: name.into(),
                result: label.into(),
                start: range.0,
                end: range.1,
                count,
                value: if label.ends_with("mean") {
                    mean
                } else {
                    (s.summary.finite_count > 0)
                        .then_some(100. * count as f64 / s.summary.finite_count as f64)
                },
                unit: if label.ends_with("mean") {
                    if settings.input_is_velocity {
                        unit.into()
                    } else {
                        format!("{unit}/s")
                    }
                } else {
                    "%".into()
                },
            });
        }
    }
}
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn result_csv(rows: &[ResultRow]) -> String {
    let mut csv = "Channel,Result,Start (s),End (s),Count,Value,Unit\n".to_owned();
    for r in rows {
        csv.push_str(&format!(
            "{},{},{:.9},{:.9},{},{},{}\n",
            quoted(&r.channel),
            quoted(&r.result),
            r.start,
            r.end,
            r.count,
            r.value.map_or_else(String::new, |v| format!("{v:.12}")),
            quoted(&r.unit)
        ));
    }
    csv
}
pub fn export_buttons(ui: &mut egui::Ui, csv: &str) {
    if ui.button("Copy CSV").clicked() {
        ui.ctx().copy_text(csv.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Export CSV…").clicked()
        && let Some(path) = crate::platform::save_csv_file()
        && let Err(error) = std::fs::write(path, csv)
    {
        ui.label(format!("Export failed: {error}"));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_and_export_share_off_grid_gated_rows() {
        let mut rows = Vec::new();
        append_results(
            &mut rows,
            "A,\"B",
            "mm",
            &[0., f64::NAN, 2., 3., 4., 5.],
            2,
            (0.25, 2.5),
            CalculationMode::IntervalAverages,
            1.,
            Default::default(),
        );
        assert_eq!((rows[0].count, rows[0].value), (1, Some(2.)));
        assert_eq!((rows[1].count, rows[1].value), (2, Some(3.5)));
        let csv = result_csv(&rows);
        assert!(csv.contains("\"A,\"\"B\""));
        assert!(csv.contains(",1,2.000000000000,\"mm\""));
    }
}

#[cfg(test)]
mod panel_population_tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn suspension_chart_summary_and_csv_share_post_velocity_gate_population() {
        let mut shared = SharedState::new();
        shared.data_duration = Some(4.);
        shared.zoom_range = Some((0., 4.));
        for (name, data, freq) in [
            ("Position", vec![0., 1., 3., 2., 0., f64::NAN, 2., 3.], 2),
            ("Gate", vec![1., 1., 0., 1.], 1),
        ] {
            let mut m = shared.create_math_channel_def(name.into(), "0".into(), "mm".into(), 1);
            m.freq = freq;
            m.data = Some(Arc::from(data));
            shared.math_channels.push(m);
        }
        let mut panel = CalculationsPanel::new(1, "Suspension");
        panel.mode = CalculationMode::Suspension;
        panel.filter.gate = "Gate".into();
        panel
            .channels
            .push(create_plotted_channel(crate::state::ChannelId::Math(0), &shared, 0).unwrap());
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| panel.ui(ui, &mut shared));
        });
        output.textures_delta.clear();
        assert_eq!(panel.velocities[0].values, [2., 4., -2., 2.]);
        assert_eq!(
            panel
                .rows
                .iter()
                .find(|r| r.result == "low bump")
                .unwrap()
                .count,
            3
        );
        assert!(panel.csv.contains(",3,75.000000000000,\"%\""));
        let bins =
            i3rs_core::distribution::suspension_histogram(&panel.velocities[0].values, 41, 60.);
        assert_eq!(bins.iter().map(|b| b.count).sum::<usize>(), 4);
    }
}

use super::*;
use i3rs_core::acceleration::{AccelerationDirection, acceleration_report};
impl ReportPanel {
    pub(super) fn acceleration_ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Speed points [km/h]:");
            let mut remove = None;
            for (i, p) in self.settings.acceleration.points_mps.iter_mut().enumerate() {
                let mut kmh = *p * 3.6;
                ui.add(egui::DragValue::new(&mut kmh).speed(1.).range(0. ..=2000.));
                *p = kmh / 3.6;
                if ui
                    .small_button("×")
                    .on_hover_text("Remove speed point")
                    .clicked()
                {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.settings.acceleration.points_mps.remove(i);
            }
            if ui.small_button("+").clicked() {
                let last = self
                    .settings
                    .acceleration
                    .points_mps
                    .last()
                    .copied()
                    .unwrap_or(0.);
                self.settings.acceleration.points_mps.push(last + 40. / 3.6);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.settings.acceleration;
            let mut hysteresis = s.hysteresis_mps * 3.6;
            ui.label("Hysteresis [km/h]");
            ui.add(
                egui::DragValue::new(&mut hysteresis)
                    .speed(0.1)
                    .range(0. ..=100.),
            );
            s.hysteresis_mps = hysteresis / 3.6;
            for (label, value) in [
                ("Accel hold [s]", &mut s.accel_hold_secs),
                ("Decel hold [s]", &mut s.decel_hold_secs),
                ("Start offset [s]", &mut s.start_padding_secs),
                ("End offset [s]", &mut s.end_padding_secs),
            ] {
                ui.label(label);
                ui.add(egui::DragValue::new(value).speed(0.1).range(0. ..=600.));
            }
            ui.checkbox(&mut self.settings.trusted_only, "Trusted ranges only");
        });
        ui.horizontal_wrapped(|ui| {
            for (metric, label) in [
                (AccelerationMetric::Time, "Time [s]"),
                (AccelerationMetric::Distance, "Distance [m]"),
                (AccelerationMetric::AverageG, "Average G"),
            ] {
                ui.selectable_value(&mut self.settings.acceleration_metric, metric, label);
            }
            for (view, label) in [
                (ReportView::Table, "Table"),
                (ReportView::Chart, "Chart"),
                (ReportView::Split, "Split"),
            ] {
                ui.selectable_value(&mut self.settings.view, view, label);
            }
        });
        if ui.ui_contains_pointer()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::U))
        {
            self.settings.acceleration_metric = match self.settings.acceleration_metric {
                AccelerationMetric::Time => AccelerationMetric::Distance,
                AccelerationMetric::Distance => AccelerationMetric::AverageG,
                AccelerationMetric::AverageG => AccelerationMetric::Time,
            };
        }
        let Some(pc) = self.channels.first() else {
            ui.label("Choose a speed channel");
            return;
        };
        let info = build_plotted_channel_info(pc, shared);
        let Ok((scale, offset)) = i3rs_core::channels::unit_transform(&info.unit, "m/s") else {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("{} [{}] is not a speed channel", info.name, info.unit),
            );
            return;
        };
        if info.freq == 0.0 || info.data.is_empty() {
            ui.weak("Speed channel is unavailable or loading");
            ui.ctx().request_repaint();
            return;
        }
        let selection = shared
            .comparisons
            .main_lap
            .or(shared.selected_lap)
            .and_then(|i| shared.laps.get(i))
            .map(|l| (l.start_time, l.end_time))
            .unwrap_or_else(|| shared.analysis_range());
        let allowed: Option<Vec<_>> = self.settings.trusted_only.then(|| {
            shared
                .current_analysis()
                .into_iter()
                .flat_map(|a| &a.ranges)
                .filter(|r| r.trust == i3rs_core::ranges::Trust::Trusted)
                .map(|r| (r.start, r.end))
                .collect()
        });
        let key = format!(
            "{:?}:{:?}:{:?}:{:?}:{:?}",
            ChannelKey::from(&info),
            selection,
            allowed,
            self.settings.acceleration,
            (scale, offset)
        );
        if self.acceleration_key != key {
            match acceleration_report(
                &ReportSeries {
                    data: &info.data,
                    freq: info.freq,
                    interpolation: info.interpolation,
                    scale: info.display_scale * scale,
                    offset: info.display_offset * scale + offset,
                },
                &[selection],
                allowed.as_deref(),
                &self.settings.acceleration,
            ) {
                Ok(report) => {
                    self.acceleration_result = report;
                    self.acceleration_key = key;
                    self.selected_event = None;
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::YELLOW, error);
                    return;
                }
            }
        }
        // Retain the allocation used in the cache identity.
        self.sources = vec![info.clone()];
        ui.horizontal(|ui| {
            crate::output_ui::result_buttons(ui, &self.acceleration_csv());
            if ui.button("Copy event JSON").clicked() {
                ui.ctx().copy_text(
                    serde_json::to_string_pretty(&self.acceleration_result)
                        .expect("finite event results"),
                );
            }
            ui.weak(format!(
                "{} speed-pair results · Main {:.3}–{:.3} s · green lines mark selection",
                self.acceleration_result.events.len(),
                selection.0,
                selection.1
            ));
        });
        ui.weak("Offsets search across range boundaries. Hold applies to the confirmed direction region; hysteresis confirms reversals. Missing speed breaks an event. U cycles units.");
        if self.acceleration_result.events.is_empty() {
            ui.label("No complete speed-pair events satisfy these settings");
        }
        let mut selected = self.selected_event;
        if self.settings.view != ReportView::Chart {
            let height = if self.settings.view == ReportView::Split {
                ui.available_height() * 0.4
            } else {
                ui.available_height()
            };
            egui::ScrollArea::both()
                .id_salt(("acceleration_table", self.id))
                .max_height(height.max(0.))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.strong("Speed-pair minimum / maximum");
                    egui::Grid::new(("acceleration_summary", self.id))
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Direction / pair [km/h]");
                            ui.strong("Minimum");
                            ui.strong("Maximum");
                            ui.strong("Events");
                            ui.end_row();
                            for direction in [
                                AccelerationDirection::Acceleration,
                                AccelerationDirection::Deceleration,
                            ] {
                                let points = &self.settings.acceleration.points_mps;
                                for i in 0..points.len().saturating_sub(1) {
                                    for j in i + 1..points.len() {
                                        let (from, to) =
                                            if direction == AccelerationDirection::Acceleration {
                                                (points[i], points[j])
                                            } else {
                                                (points[j], points[i])
                                            };
                                        let rows: Vec<_> = self
                                            .acceleration_result
                                            .events
                                            .iter()
                                            .filter(|e| {
                                                e.direction == direction
                                                    && e.from_mps == from
                                                    && e.to_mps == to
                                            })
                                            .map(|e| self.event_value(e))
                                            .collect();
                                        ui.label(format!(
                                            "{direction:?} · {:.1} to {:.1}",
                                            from * 3.6,
                                            to * 3.6
                                        ));
                                        for value in [
                                            rows.iter().copied().reduce(f64::min),
                                            rows.iter().copied().reduce(f64::max),
                                        ] {
                                            ui.monospace(
                                                value.map_or_else(
                                                    || "—".into(),
                                                    |v| format!("{v:.3}"),
                                                ),
                                            );
                                        }
                                        ui.label(rows.len().to_string());
                                        ui.end_row();
                                    }
                                }
                            }
                        });
                    ui.separator();
                    ui.strong("All events (select to highlight chart and cursor)");
                    egui::Grid::new(("acceleration_events", self.id))
                        .striped(true)
                        .show(ui, |ui| {
                            for label in [
                                "Event",
                                "Speed pair [km/h]",
                                "Start [s]",
                                "End [s]",
                                "Time [s]",
                                "Distance [m]",
                                "Average G",
                            ] {
                                ui.strong(label);
                            }
                            ui.end_row();
                            for (i, e) in self.acceleration_result.events.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        selected == Some(i),
                                        format!("{} · {:?}", i + 1, e.direction),
                                    )
                                    .clicked()
                                {
                                    selected = Some(i);
                                }
                                ui.label(format!(
                                    "{:.1} to {:.1}",
                                    e.from_mps * 3.6,
                                    e.to_mps * 3.6
                                ));
                                for v in [
                                    e.start,
                                    e.end,
                                    e.elapsed_secs,
                                    e.distance_metres,
                                    e.average_g,
                                ] {
                                    ui.monospace(format!("{v:.3}"));
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        if self.settings.view != ReportView::Table {
            use egui_plot::{Line, Plot, PlotPoints, Points, VLine};
            egui::ScrollArea::vertical()
                .id_salt(("acceleration_charts", self.id))
                .show(ui, |ui| {
                    Plot::new(("acceleration_speed", self.id))
                        .height(170.)
                        .y_axis_label("Speed [km/h]")
                        .x_axis_label("Recording time [s]")
                        .show(ui, |plot| {
                            for (a, b) in &self.acceleration_result.search_windows {
                                let window = i3rs_core::sample_window_for_time(
                                    info.freq,
                                    info.data.len(),
                                    *a,
                                    *b,
                                );
                                let mut run = Vec::new();
                                // Pixel-scale min/max decimation preserves speed extrema and gaps.
                                let mut flush = |run: &mut Vec<[f64; 2]>| {
                                    if !run.is_empty() {
                                        let data = std::mem::take(run);
                                        plot.line(
                                            Line::new(&info.name, PlotPoints::from(data))
                                                .color(info.color),
                                        );
                                    }
                                };
                                let stride =
                                    ((window.end_sample - window.start_sample) / 1000).max(1);
                                for chunk in (window.start_sample..window.end_sample)
                                    .collect::<Vec<_>>()
                                    .chunks(stride)
                                {
                                    if chunk.iter().any(|i| !info.data[*i].is_finite()) {
                                        flush(&mut run);
                                        continue;
                                    }
                                    let min = *chunk
                                        .iter()
                                        .min_by(|a, b| info.data[**a].total_cmp(&info.data[**b]))
                                        .unwrap();
                                    let max = *chunk
                                        .iter()
                                        .max_by(|a, b| info.data[**a].total_cmp(&info.data[**b]))
                                        .unwrap();
                                    let mut indices = [min, max];
                                    indices.sort();
                                    for i in indices {
                                        run.push([
                                            i as f64 / info.freq,
                                            info.transform_value(info.data[i]) * scale * 3.6
                                                + offset * 3.6,
                                        ]);
                                    }
                                }
                                flush(&mut run);
                            }
                            for t in [selection.0, selection.1] {
                                plot.vline(
                                    VLine::new("Main boundary", t).color(egui::Color32::GREEN),
                                );
                            }
                            if let Some(e) =
                                selected.and_then(|i| self.acceleration_result.events.get(i))
                            {
                                plot.line(
                                    Line::new(
                                        "Selected event",
                                        vec![[e.start, e.from_mps * 3.6], [e.end, e.to_mps * 3.6]],
                                    )
                                    .width(3.)
                                    .color(egui::Color32::WHITE),
                                );
                            }
                        });
                    Plot::new(("acceleration_results", self.id))
                        .height(190.)
                        .x_axis_label("Event (click to select)")
                        .y_axis_label(match self.settings.acceleration_metric {
                            AccelerationMetric::Time => "Time [s]",
                            AccelerationMetric::Distance => "Distance [m]",
                            AccelerationMetric::AverageG => "Average G",
                        })
                        .show(ui, |plot| {
                            let points: Vec<_> = self
                                .acceleration_result
                                .events
                                .iter()
                                .enumerate()
                                .map(|(i, e)| [i as f64, self.event_value(e)])
                                .collect();
                            plot.points(Points::new("Events", PlotPoints::from(points)).radius(4.));
                            if let Some(i) = selected.and_then(|i| {
                                self.acceleration_result.events.get(i).map(|e| (i, e))
                            }) {
                                plot.points(
                                    Points::new(
                                        "Selected",
                                        vec![[i.0 as f64, self.event_value(i.1)]],
                                    )
                                    .radius(7.)
                                    .color(egui::Color32::YELLOW),
                                );
                            }
                            if plot.response().clicked()
                                && let Some(p) = plot.pointer_coordinate()
                            {
                                let i = p.x.round();
                                if i >= 0. && (i as usize) < self.acceleration_result.events.len() {
                                    selected = Some(i as usize);
                                }
                            }
                        });
                });
        }
        if selected != self.selected_event {
            self.selected_event = selected;
            if let Some(e) = selected.and_then(|i| self.acceleration_result.events.get(i)) {
                shared.cursor_time = Some(e.start);
            }
        }
        shared.plotted_channel_registry.push(info);
    }
    fn event_value(&self, e: &i3rs_core::acceleration::AccelerationEvent) -> f64 {
        match self.settings.acceleration_metric {
            AccelerationMetric::Time => e.elapsed_secs,
            AccelerationMetric::Distance => e.distance_metres,
            AccelerationMetric::AverageG => e.average_g,
        }
    }
    pub(super) fn acceleration_csv(&self) -> String {
        let mut out = String::from(
            "Region,Direction,From [m/s],To [m/s],Start [s],End [s],Time [s],Distance [m],Average G\n",
        );
        for e in &self.acceleration_result.events {
            out.push_str(&format!(
                "{},{:?},{},{},{},{},{},{},{}\n",
                e.region,
                e.direction,
                e.from_mps,
                e.to_mps,
                e.start,
                e.end,
                e.elapsed_secs,
                e.distance_metres,
                e.average_g
            ));
        }
        out
    }
}

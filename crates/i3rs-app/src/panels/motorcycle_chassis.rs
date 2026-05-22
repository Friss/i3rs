//! Motorcycle chassis panel: animated wireframe schematic with real-time geometry readout.
//!
//! Displays a side-elevation wireframe of the motorcycle chassis animated from
//! suspension pot readings and lean angle at the current cursor time.
//! Chassis geometry (rake, trail, wheelbase, anti-squat, CoG, etc.) is shown
//! in a real-time attribute panel alongside the schematic.
//!
//! Chassis definition is loaded from a MotoSPEC MS1/MS3 chassis file.
//! Use the toolbar to open a file or pop the panel out to its own window.

use std::path::PathBuf;

use eframe::egui;
use egui::{Color32, Painter, Sense, Stroke, Vec2, pos2};
use i3rs_core::{ChassisModel, ChassisSolver, FrameState, SchematicStroke, compute_schematic,
               detect_columns, parse_motospec_file};

use crate::panels::cursor_readout::interpolate_at_time;
use crate::state::SharedState;

// Default MoTeC channel names for suspension pots and lean angle.
const DEFAULT_RR_POT_CHANNEL: &str = "s_susp_rr";
const DEFAULT_FR_POT_CHANNEL: &str = "s_susp_fr";
const DEFAULT_LEAN_CHANNEL: &str = "phi_lean";

/// Motorcycle chassis panel state.
pub struct MotorcycleChassisPanel {
    pub id: u64,
    pub title: String,

    /// Path to the loaded MotoSPEC chassis file.
    pub motospec_path: Option<PathBuf>,
    /// Active setup column (1-based).
    pub motospec_column: u8,
    /// Column IDs present in the current file; populated on load.
    available_columns: Vec<u8>,
    /// File path awaiting column selection (set after file dialog, cleared after user picks column).
    pending_path: Option<PathBuf>,

    /// Loaded chassis model (None if file not loaded or failed).
    chassis_model: Option<ChassisModel>,
    /// Pre-built geometry solver.
    solver: Option<ChassisSolver>,
    /// Error string shown in the panel when loading fails.
    load_error: Option<String>,

    /// MoTeC channel name for rear suspension pot.
    pub rr_pot_channel: String,
    /// MoTeC channel name for front suspension pot.
    pub fr_pot_channel: String,
    /// MoTeC channel name for lean angle.
    pub lean_channel: String,

    /// Whether the legend overlay is visible.
    show_legend: bool,

    // Pop-out window state (mirrors TrackMapPanel pattern)
    /// Whether this panel is currently in a popped-out OS window.
    pub is_popped_out: bool,
    /// Set by the "Pop Out" button; consumed by App to move panel to its own window.
    pub pop_out_requested: bool,
    /// Set by the "Dock" button or window close; consumed by App to move panel back to dock.
    pub dock_requested: bool,
    /// Worksheet index this panel belongs to when docked.
    pub home_worksheet: usize,
}

impl MotorcycleChassisPanel {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            motospec_path: None,
            motospec_column: 1,
            available_columns: vec![1],
            pending_path: None,
            chassis_model: None,
            solver: None,
            load_error: None,
            rr_pot_channel: DEFAULT_RR_POT_CHANNEL.into(),
            fr_pot_channel: DEFAULT_FR_POT_CHANNEL.into(),
            lean_channel: DEFAULT_LEAN_CHANNEL.into(),
            show_legend: true,
            is_popped_out: false,
            pop_out_requested: false,
            dock_requested: false,
            home_worksheet: 0,
        }
    }

    /// Load a chassis model from a MotoSPEC MS1/MS3 file using the specified column.
    pub fn load_motospec(&mut self, path: PathBuf, column: u8) {
        // Detect available columns so the toolbar selector knows what to offer.
        self.available_columns = detect_columns(&path).unwrap_or_else(|_| vec![1]);
        // Clamp requested column to what the file actually contains.
        let column = if self.available_columns.contains(&column) {
            column
        } else {
            *self.available_columns.first().unwrap_or(&1)
        };
        match parse_motospec_file(&path, column) {
            Ok(model) => {
                self.solver = Some(ChassisSolver::prepare(model.clone()));
                self.chassis_model = Some(model);
                self.motospec_path = Some(path);
                self.motospec_column = column;
                self.load_error = None;
            }
            Err(e) => {
                self.load_error = Some(e);
                self.chassis_model = None;
                self.solver = None;
                self.motospec_path = Some(path);
                self.motospec_column = column;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        if self.is_popped_out && ui.input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dock_requested = true;
        }

        self.show_column_picker_modal(ui);
        self.show_toolbar(ui, shared);
        ui.separator();
        self.show_main(ui, shared);
    }

    // ---------------------------------------------------------------------------
    // Column-picker modal — shown immediately after a file is chosen
    // ---------------------------------------------------------------------------

    fn show_column_picker_modal(&mut self, ui: &mut egui::Ui) {
        let Some(pending) = self.pending_path.clone() else { return };

        let file_name = pending.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "chassis file".into());

        let mut open = true;
        egui::Window::new("Select Setup Column")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(format!("File: {file_name}"));
                ui.label(egui::RichText::new(
                    "This file contains multiple setup columns.\nSelect the column to load."
                ).weak());
                ui.add_space(6.0);

                let cols = self.available_columns.clone();
                for col in &cols {
                    ui.radio_value(&mut self.motospec_column, *col, format!("Column {col}"));
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Load").clicked() {
                        let col = self.motospec_column;
                        self.load_motospec(pending, col);
                        self.pending_path = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_path = None;
                    }
                });
            });

        // Window X button dismissed
        if !open {
            self.pending_path = None;
        }
    }

    // ---------------------------------------------------------------------------
    // Toolbar
    // ---------------------------------------------------------------------------

    fn show_toolbar(&mut self, ui: &mut egui::Ui, _shared: &mut SharedState) {
        ui.horizontal(|ui| {
            if ui.button("📂 Open Chassis File").clicked() {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("MotoSPEC chassis", &["ms1", "ms3"])
                    .pick_file()
                {
                    // Detect columns first; if only one is available, load immediately.
                    // Otherwise open the column-picker modal.
                    let cols = detect_columns(&path).unwrap_or_else(|_| vec![1]);
                    if cols.len() <= 1 {
                        let col = *cols.first().unwrap_or(&1);
                        self.load_motospec(path, col);
                    } else {
                        self.available_columns = cols;
                        self.motospec_column = *self.available_columns.first().unwrap_or(&1);
                        self.pending_path = Some(path);
                    }
                }
            }

            // Filename
            if let Some(ref p) = self.motospec_path {
                let name = p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.label(format!("  {name}"));
            } else {
                ui.label(egui::RichText::new("  No chassis file loaded").weak());
            }

            // Column selector — only shown when the file has more than one column
            if self.motospec_path.is_some() && self.available_columns.len() > 1 {
                ui.separator();
                let cols = self.available_columns.clone();
                let mut changed = false;
                for col in &cols {
                    if ui.radio_value(&mut self.motospec_column, *col, col.to_string()).changed() {
                        changed = true;
                    }
                }
                if changed {
                    if let Some(path) = self.motospec_path.clone() {
                        let col = self.motospec_column;
                        self.load_motospec(path, col);
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.is_popped_out {
                    if ui.button("⬅ Dock").clicked() {
                        self.dock_requested = true;
                    }
                } else if ui.button("⬆ Pop Out").clicked() {
                    self.pop_out_requested = true;
                }
                ui.toggle_value(&mut self.show_legend, "Legend");
            });
        });
    }

    // ---------------------------------------------------------------------------
    // Main content: schematic + attributes
    // ---------------------------------------------------------------------------

    fn show_main(&mut self, ui: &mut egui::Ui, shared: &mut SharedState) {
        // Solve current frame state from cursor time
        let frame_state = self.solve_at_cursor(shared);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Left column: wireframe schematic
                let available = ui.available_size();
                let schematic_width = (available.x * 0.62).min(available.x - 220.0).max(200.0);
                let schematic_height = available.y.clamp(300.0, 700.0);

                ui.allocate_ui(Vec2::new(schematic_width, schematic_height), |ui| {
                    self.show_schematic(ui, frame_state.as_ref());
                });

                ui.separator();

                // Right column: attribute readout
                ui.allocate_ui(Vec2::new(available.x - schematic_width - 10.0, schematic_height), |ui| {
                    self.show_attributes(ui, frame_state.as_ref());
                });
            });
        });
    }

    // ---------------------------------------------------------------------------
    // Solve frame state at cursor
    // ---------------------------------------------------------------------------

    fn solve_at_cursor(&self, shared: &SharedState) -> Option<FrameState> {
        let solver = self.solver.as_ref()?;
        let ld = shared.ld_file.as_ref()?;
        let t = shared.cursor_time?;

        let rr_pot = channel_value_at(ld, &self.rr_pot_channel, t);
        let fr_pot = channel_value_at(ld, &self.fr_pot_channel, t);
        let lean = channel_value_at(ld, &self.lean_channel, t);

        Some(solver.solve(rr_pot.unwrap_or(0.0), fr_pot.unwrap_or(0.0), lean.unwrap_or(0.0)))
    }

    // ---------------------------------------------------------------------------
    // Schematic renderer
    // ---------------------------------------------------------------------------

    fn show_schematic(&self, ui: &mut egui::Ui, frame_state: Option<&FrameState>) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let rect = resp.rect;
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

        if let Some(err) = &self.load_error {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("⚠ {err}"),
                egui::FontId::proportional(13.0),
                ui.visuals().warn_fg_color,
            );
            return;
        }

        if self.chassis_model.is_none() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Open a MotoSPEC MS1/MS3 chassis file\nto display the chassis schematic.",
                egui::FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
            return;
        }

        let schematic = compute_schematic(self.chassis_model.as_ref(), frame_state, 0.0, 0.0);

        // Map solver space to screen: flip Y (solver +Y = up, screen +Y = down)
        let bounds = &schematic.bounds;
        let bw = bounds.width();
        let bh = bounds.height();
        if bw < 1.0 || bh < 1.0 { return; }

        let pad = 16.0;
        let draw_w = rect.width() - pad * 2.0;
        let draw_h = rect.height() - pad * 2.0;
        let scale = (draw_w / bw as f32).min(draw_h / bh as f32);
        let origin_x = rect.left() + pad + (draw_w - bw as f32 * scale) * 0.5 - bounds.min_x as f32 * scale;
        let origin_y = rect.bottom() - pad - (draw_h - bh as f32 * scale) * 0.5 + bounds.min_y as f32 * scale;

        let to_screen = |sx: f64, sy: f64| -> egui::Pos2 {
            pos2(origin_x + sx as f32 * scale, origin_y - sy as f32 * scale)
        };

        // Draw circles first (background layers)
        for c in &schematic.circles {
            let center = to_screen(c.cx, c.cy);
            let r = c.radius_mm as f32 * scale;
            let color = Color32::from_rgb(c.r, c.g, c.b);
            if c.fill {
                painter.circle_filled(center, r, color.gamma_multiply(0.18));
            } else {
                painter.circle_stroke(center, r, Stroke::new(c.thickness as f32 * scale.min(2.0), color));
            }
        }

        // Draw lines
        for ln in &schematic.lines {
            let a = to_screen(ln.x1, ln.y1);
            let b = to_screen(ln.x2, ln.y2);
            let color = Color32::from_rgb(ln.r, ln.g, ln.b);
            let th = (ln.thickness as f32 * scale.min(2.5)).clamp(0.8, 5.0);
            match ln.stroke {
                SchematicStroke::Solid => {
                    painter.line_segment([a, b], Stroke::new(th, color));
                }
                SchematicStroke::Dashed => {
                    draw_dashed_line(&painter, a, b, th, color, 8.0 * scale);
                }
                SchematicStroke::Dotted => {
                    draw_dashed_line(&painter, a, b, th, color, 4.0 * scale);
                }
            }
        }

        // Legend overlay
        if self.show_legend && !schematic.legend.is_empty() {
            let lx = rect.left() + 6.0;
            let mut ly = rect.top() + 6.0;
            for entry in &schematic.legend {
                let color = Color32::from_rgb(entry.r, entry.g, entry.b);
                let a = pos2(lx, ly + 5.0);
                let b = pos2(lx + 22.0, ly + 5.0);
                painter.line_segment([a, b], Stroke::new(entry.thickness as f32 * 0.8, color));
                painter.text(
                    pos2(lx + 26.0, ly + 5.0),
                    egui::Align2::LEFT_CENTER,
                    &entry.label,
                    egui::FontId::proportional(11.0),
                    ui.visuals().text_color(),
                );
                ly += 14.0;
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Attribute readout panel
    // ---------------------------------------------------------------------------

    fn show_attributes(&self, ui: &mut egui::Ui, frame_state: Option<&FrameState>) {
        let text_color = ui.visuals().text_color();
        let weak_color = ui.visuals().weak_text_color();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);

            egui::Grid::new("chassis_attrs")
                .num_columns(2)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    // Helper: bold section heading spanning both columns.
                    let heading = |ui: &mut egui::Ui, title: &str| {
                        ui.label(egui::RichText::new(title).strong().size(12.0));
                        ui.label("");
                        ui.end_row();
                    };

                    // Helper: attribute row — weak label + monospace value.
                    let row = |ui: &mut egui::Ui, label: &str, value: Option<String>| {
                        ui.label(egui::RichText::new(label).color(weak_color).size(11.0));
                        match value {
                            Some(v) => ui.label(egui::RichText::new(v).monospace().color(text_color).size(11.0)),
                            None    => ui.label(egui::RichText::new("—").color(weak_color).size(11.0)),
                        };
                        ui.end_row();
                    };

                    // ── Geometry ──────────────────────────────────────────
                    heading(ui, "Geometry");
                    if let Some(st) = frame_state {
                        row(ui, "Wheelbase",        Some(format!("{:.1} mm", st.wheelbase_mm)));
                        row(ui, "Rake",              Some(format!("{:.2}°",   st.rake_deg)));
                        row(ui, "Ground trail",      Some(format!("{:.1} mm", st.ground_trail_mm)));
                        row(ui, "Normal trail",      Some(format!("{:.1} mm", st.trail_mm)));
                        row(ui, "Swingarm angle",    Some(format!("{:.2}°",   st.inst_sw_angle_deg)));
                        row(ui, "Rear ride height",  Some(format!("{:.1} mm", st.inst_ride_ht_mm)));
                        row(ui, "Ground angle",      Some(format!("{:.2}°",   st.ground_angle_deg)));
                    } else {
                        for lbl in &["Wheelbase", "Rake", "Ground trail", "Normal trail",
                                     "Swingarm angle", "Rear ride height", "Ground angle"] {
                            row(ui, lbl, None);
                        }
                    }

                    // ── Suspension ────────────────────────────────────────
                    ui.label(""); ui.label(""); ui.end_row(); // spacer
                    heading(ui, "Suspension");
                    if let Some(st) = frame_state {
                        row(ui, "Rr pot",             Some(format!("{:.1} mm", st.rr_pot_mm)));
                        row(ui, "Fr pot",             Some(format!("{:.1} mm", st.fr_pot_mm)));
                        row(ui, "Rr wheel travel",    Some(format!("{:.1} mm", st.rr_wheel_travel_mm)));
                        row(ui, "Fr fork comp",       Some(format!("{:.1} mm", st.fr_fork_comp_mm)));
                        row(ui, "Rr wheel rate",      fmt_n_mm(st.rr_wheel_rate_n_per_mm));
                        row(ui, "Fr wheel rate",      fmt_n_mm(st.fr_wheel_rate_n_per_mm));
                        row(ui, "Rr wheel force",     fmt_newton(st.rr_wheel_force_n));
                        row(ui, "Fr wheel force",     fmt_newton(st.fr_wheel_force_n));
                        row(ui, "Rr MR (shock/wheel)", Some(format!("{:.3}", st.rr_motion_ratio_shock_per_wheel)));
                    } else {
                        for lbl in &["Rr pot", "Fr pot", "Rr wheel travel", "Fr fork comp",
                                     "Rr wheel rate", "Fr wheel rate", "Rr wheel force",
                                     "Fr wheel force", "Rr MR (shock/wheel)"] {
                            row(ui, lbl, None);
                        }
                    }

                    // ── Anti-squat / CoG ──────────────────────────────────
                    ui.label(""); ui.label(""); ui.end_row(); // spacer
                    heading(ui, "Anti-squat / CoG");
                    if let Some(st) = frame_state {
                        row(ui, "Anti-squat",        fmt_pct(st.anti_squat_pct));
                        row(ui, "Anti-squat angle",  Some(fmt_nan_deg(st.anti_squat_angle_deg)));
                        row(ui, "IC height",         Some(format!("{:.1} mm", st.instant_center_height_mm)));
                        row(ui, "CoG front",         fmt_pct(st.cog_percent_front));
                        row(ui, "CoG rear",          fmt_pct(st.cog_percent_rear));
                        row(ui, "CoG height",        Some(format!("{:.1} mm", st.cog_y_mm)));
                        row(ui, "Load xfer angle",   Some(fmt_nan_deg(st.load_transfer_angle_deg)));
                        row(ui, "Pivot height",      Some(format!("{:.1} mm", st.pivot_height_mm)));
                    } else {
                        for lbl in &["Anti-squat", "Anti-squat angle", "IC height",
                                     "CoG front", "CoG rear", "CoG height",
                                     "Load xfer angle", "Pivot height"] {
                            row(ui, lbl, None);
                        }
                    }
                });

            // Channel assignments section
            ui.add_space(8.0);
            ui.collapsing("Channel Assignments", |ui| {
                egui::Grid::new("chassis_channel_assignments")
                    .num_columns(2)
                    .spacing([4.0, 2.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Rr pot").size(11.0));
                        ui.label(egui::RichText::new(&self.rr_pot_channel).monospace().size(11.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Fr pot").size(11.0));
                        ui.label(egui::RichText::new(&self.fr_pot_channel).monospace().size(11.0));
                        ui.end_row();
                        ui.label(egui::RichText::new("Lean").size(11.0));
                        ui.label(egui::RichText::new(&self.lean_channel).monospace().size(11.0));
                        ui.end_row();
                    });
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn channel_value_at(ld: &i3rs_core::LdFile, name: &str, time: f64) -> Option<f64> {
    let ch = ld.channels.iter().find(|c| c.name == name)?;
    let data = ld.read_channel_data(ch)?;
    if data.is_empty() { return None; }
    Some(interpolate_at_time(&data, ch.freq, time))
}

fn fmt_n_mm(v: f64) -> Option<String> {
    if v.is_nan() { None } else { Some(format!("{:.1} N/mm", v)) }
}

fn fmt_newton(v: f64) -> Option<String> {
    if v.is_nan() { None } else { Some(format!("{:.0} N", v)) }
}

fn fmt_pct(v: f64) -> Option<String> {
    if v.is_nan() { None } else { Some(format!("{:.1}%", v)) }
}

fn fmt_nan_deg(v: f64) -> String {
    if v.is_nan() { "—".into() } else { format!("{:.2}°", v) }
}

fn draw_dashed_line(
    painter: &Painter,
    a: egui::Pos2,
    b: egui::Pos2,
    thickness: f32,
    color: Color32,
    dash_len: f32,
) {
    let delta = b - a;
    let total = delta.length();
    if total < 1e-3 { return; }
    let dir = delta / total;
    let mut pos = 0.0_f32;
    let mut draw = true;
    while pos < total {
        let next = (pos + dash_len).min(total);
        if draw {
            let p1 = a + dir * pos;
            let p2 = a + dir * next;
            painter.line_segment([p1, p2], Stroke::new(thickness, color));
        }
        pos = next;
        draw = !draw;
    }
}

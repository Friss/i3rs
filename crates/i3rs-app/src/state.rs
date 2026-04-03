//! Shared application state accessible by all panels.

use eframe::egui;
use i3rs_core::{Lap, LdFile, LdxFile};
use std::path::PathBuf;
use std::sync::Arc;

/// Identifies a channel: either a physical channel from the .ld file or a math channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelId {
    /// Index into `LdFile::channels`.
    Physical(usize),
    /// Index into `SharedState::math_channels`.
    Math(usize),
}

/// Which Y-axis a channel is assigned to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum YAxis {
    Left,
    Right,
}

/// A loaded channel's cached display data.
pub struct PlottedChannel {
    pub channel_id: ChannelId,
    pub color: egui::Color32,
    pub data: Arc<Vec<f64>>,
    pub y_axis: YAxis,
    /// Cached min value (computed once on load).
    pub cached_min: f64,
    /// Cached max value (computed once on load).
    pub cached_max: f64,
    /// Cached average value (computed once on load).
    pub cached_avg: f64,
}

/// Info about a plotted channel, registered by graph panels each frame for the readout panel.
pub struct PlottedChannelInfo {
    pub name: String,
    pub unit: String,
    pub freq: u16,
    pub dec_places: i16,
    pub color: egui::Color32,
    pub data: Arc<Vec<f64>>,
}

/// A user-defined math channel.
pub struct MathChannelDef {
    pub name: String,
    pub expression: String,
    pub unit: String,
    pub dec_places: i16,
    pub freq: u16,
    /// Cached evaluation result.
    pub data: Option<Arc<Vec<f64>>>,
    /// Parse or evaluation error.
    pub error: Option<String>,
    pub cached_min: f64,
    pub cached_max: f64,
    pub cached_avg: f64,
}

impl MathChannelDef {
    pub fn new(name: String, expression: String, unit: String, dec_places: i16) -> Self {
        Self {
            name,
            expression,
            unit,
            dec_places,
            freq: 0,
            data: None,
            error: None,
            cached_min: 0.0,
            cached_max: 0.0,
            cached_avg: 0.0,
        }
    }
}

/// Compute min, max, avg, stddev for a slice of finite f64 values.
pub fn compute_channel_stats(data: &[f64]) -> (f64, f64, f64, f64) {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut sum = 0.0;
    let mut count = 0u64;

    for &v in data {
        if v.is_finite() {
            if v < min { min = v; }
            if v > max { max = v; }
            sum += v;
            count += 1;
        }
    }

    if count == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let avg = sum / count as f64;
    let mut var_sum = 0.0;
    for &v in data {
        if v.is_finite() {
            let diff = v - avg;
            var_sum += diff * diff;
        }
    }
    let stddev = (var_sum / count as f64).sqrt();

    (min, max, avg, stddev)
}

/// Graph display mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphMode {
    /// All channels overlaid on one graph.
    Overlay,
    /// Each channel in its own vertically stacked tile.
    Tiled,
}

pub const CHANNEL_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(255, 100, 100), // red
    egui::Color32::from_rgb(100, 180, 255), // blue
    egui::Color32::from_rgb(100, 255, 100), // green
    egui::Color32::from_rgb(255, 200, 50),  // yellow
    egui::Color32::from_rgb(200, 100, 255), // purple
    egui::Color32::from_rgb(255, 150, 50),  // orange
    egui::Color32::from_rgb(50, 255, 200),  // cyan
    egui::Color32::from_rgb(255, 100, 200), // pink
];

/// State shared across all panels.
pub struct SharedState {
    pub ld_file: Option<Arc<LdFile>>,
    pub ld_path: Option<PathBuf>,
    pub file_name: String,

    // Lap data
    pub laps: Vec<Lap>,
    pub ldx: Option<LdxFile>,
    pub selected_lap: Option<usize>,
    pub show_lap_markers: bool,

    // Cross-panel synchronization
    pub cursor_time: Option<f64>,
    pub zoom_range: Option<(f64, f64)>,
    pub data_duration: Option<f64>,
    /// Set to true when the timeline (or other external control) changes the zoom.
    /// Graphs read this to apply the zoom, then clear it.
    pub zoom_from_timeline: bool,

    pub plotted_channel_registry: Vec<PlottedChannelInfo>,
    pub display_channel_registry: Vec<PlottedChannelInfo>,

    // Channel browser
    pub channel_filter: String,
    pub dragging_channel: Option<ChannelId>,

    // Channels pending addition (set by browser, consumed by graph panels)
    pub pending_toggle_channel: Option<ChannelId>,

    // Math channels
    pub math_channels: Vec<MathChannelDef>,

    // Next panel ID counter
    pub next_panel_id: u64,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            ld_file: None,
            ld_path: None,
            file_name: String::new(),
            laps: Vec::new(),
            ldx: None,
            selected_lap: None,
            show_lap_markers: true,
            cursor_time: None,
            zoom_range: None,
            data_duration: None,
            zoom_from_timeline: false,
            plotted_channel_registry: Vec::new(),
            display_channel_registry: Vec::new(),
            channel_filter: String::new(),
            dragging_channel: None,
            pending_toggle_channel: None,
            math_channels: Vec::new(),
            next_panel_id: 1,
        }
    }
}

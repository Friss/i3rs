//! MoTeC i2 .ld log file parser and data access library.
//!
//! Provides memory-mapped file access for efficient parsing of large log files
//! (100MB+) with on-demand channel data decoding.

pub mod analysis;
mod downsample;
pub mod export;
pub mod fft;
mod lap_detect;
mod ld_parser;
mod ldx_parser;
pub mod math_engine;
pub mod math_expr;
mod state_labels;
pub mod track;

pub use analysis::{
    AnalysisStats, ComparisonStats, HistogramBin, SampleWindow, ScatterPoint, compare_aligned,
    compute_stats, evenly_spaced_times, find_lap, fixed_hz_times, histogram_bins,
    resample_at_times, sample_at_time, sample_window_for_time, scatter_pairs_at_times,
};
pub use downsample::{DownsampledPoint, downsample_minmax};
pub use export::{ExportChannel, export_csv};
pub use fft::{FftResult, compute_fft, compute_fft_with_planner};
pub use lap_detect::{Lap, detect_laps, format_duration};
pub use ld_parser::{ChannelMeta, DataType, Event, LdFile, Session, normalize_channel_name};
pub use ldx_parser::{LdxFile, LdxLap, find_ldx_for_ld};
pub use math_engine::{
    ChannelData, MathError, evaluate_expression, evaluate_expression_with_aliases,
    resolve_alias_target,
};
pub use math_expr::{Expr, ParseError, parse_expression, referenced_channels};
pub use rustfft::FftPlanner;
pub use state_labels::{format_state_value, is_state_channel, state_channel_labels};
pub use track::{
    Sector, SectorTime, TrackData, compute_color_map, compute_sector_times, extract_gps_track,
    find_nearest_sample,
};

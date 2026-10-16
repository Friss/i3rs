//! MoTeC i2 .ld log file parser and data access library.
//!
//! Provides memory-mapped file access for efficient parsing of large log files
//! (100MB+) with on-demand channel data decoding.

pub mod analysis;
pub mod channels;
pub mod distance;
pub mod distribution;
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
pub use channels::{
    ChannelKey, ChannelMapping, ChannelReference, ChannelResolution, ChannelSettings,
    Interpolation, MathOptions, SetupConstant, SourcePolicy, component_gate_includes,
};
pub use downsample::{DownsampledPoint, downsample_minmax};
pub use export::{ExportChannel, export_csv};
pub use fft::{FftResult, compute_fft, compute_fft_with_planner};
pub use lap_detect::{Lap, detect_laps, format_duration, recorded_beacon_times};
pub use ld_parser::{ChannelMeta, DataType, Event, LdFile, Session, normalize_channel_name};
pub use ldx_parser::{LdxFile, LdxLap, find_ldx_for_ld};
pub use math_engine::{
    ChannelData, EvaluationOptions, MathError, evaluate_expression,
    evaluate_expression_with_aliases, evaluate_expression_with_options, lowpass,
    resolve_alias_target, smooth_samples,
};
pub use math_expr::{Expr, ParseError, parse_expression, referenced_channels};
pub use rustfft::FftPlanner;
pub use state_labels::{format_state_value, is_state_channel, state_channel_labels};
pub use track::{
    Sector, SectorTime, TrackData, TrackDefinition, compute_color_map, compute_sector_times,
    export_track_definition, extract_gps_track, find_nearest_sample, find_nearest_sample_in_range,
    import_track_definition, sector_interval, split_lap_into_sectors,
};

pub mod corrections;
pub mod ranges;

pub mod annotations;

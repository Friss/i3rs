//! MoTeC i2 .ld log file parser and data access library.
//!
//! Provides memory-mapped file access for efficient parsing of large log files
//! (100MB+) with on-demand channel data decoding.

mod downsample;
pub mod export;
mod lap_detect;
mod ld_parser;
mod ldx_parser;
pub mod math_engine;
pub mod math_expr;

pub use downsample::{DownsampledPoint, downsample_minmax};
pub use export::{ExportChannel, export_csv};
pub use lap_detect::{Lap, detect_laps};
pub use ld_parser::{ChannelMeta, DataType, Event, LdFile, Session};
pub use ldx_parser::{LdxFile, LdxLap, find_ldx_for_ld};
pub use math_engine::{ChannelData, MathError, evaluate_expression, evaluate_expression_with_aliases, resolve_alias_target};
pub use math_expr::{Expr, ParseError, parse_expression, referenced_channels};

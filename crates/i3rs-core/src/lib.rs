//! MoTeC i2 .ld log file parser and data access library.
//!
//! Provides memory-mapped file access for efficient parsing of large log files
//! (100MB+) with on-demand channel data decoding.

mod downsample;
mod lap_detect;
mod ld_parser;
mod ldx_parser;

pub use downsample::{DownsampledPoint, downsample_minmax};
pub use lap_detect::{Lap, detect_laps};
pub use ld_parser::{ChannelMeta, DataType, Event, LdFile, Session};
pub use ldx_parser::{LdxFile, LdxLap, find_ldx_for_ld};

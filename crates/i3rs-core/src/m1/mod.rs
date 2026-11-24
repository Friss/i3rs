//! Native MoTeC M1 log acquisition and conversion.
//!
//! The transport and raw-image readers are separate: an acquired M1 image is
//! not an i2 `.ld` recording. See `docs/motec-log-download-investigation.md`
//! for supported structures, validation evidence and remaining hardware limits.

pub mod catalog;
pub mod image_decode;
pub mod ld_export;
pub mod log_image;
pub mod log_info;
pub mod telemetry;
pub mod transform;
pub mod xcp;

#[cfg(not(target_arch = "wasm32"))]
pub mod transport;

#[cfg(not(target_arch = "wasm32"))]
pub mod discovery;

#[cfg(not(target_arch = "wasm32"))]
pub mod capture;

#[cfg(not(target_arch = "wasm32"))]
pub mod capture_decode;

#[cfg(not(target_arch = "wasm32"))]
pub mod conversion;

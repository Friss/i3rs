//! Shared library entry points for the native and web i3rs app.

mod app;
mod background_jobs;
mod default_layouts;
mod panels;
mod perf_metrics;
mod platform;
mod preferences;
mod project;
mod state;
mod workspace;

pub use app::App;
pub use app::LoadedSessionSummary;

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
pub use web::WebHandle;

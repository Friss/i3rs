use std::path::PathBuf;
use std::sync::Arc;

use crate::state::{ChannelStats, compute_channel_stats};
#[cfg(not(target_arch = "wasm32"))]
use i3rs_core::extract_gps_track;
use i3rs_core::{ChannelData, Lap, LdFile, LdxFile, TrackData, detect_laps, find_ldx_for_ld};

pub struct LoadedSession {
    pub file_name: String,
    pub ld_path: Option<PathBuf>,
    pub ldx: Option<LdxFile>,
    pub ld: LdFile,
    pub laps: Vec<Lap>,
    pub data_duration: f64,
}

pub enum LoadSessionSource {
    Path(PathBuf),
    Bytes {
        file_name: String,
        bytes: Vec<u8>,
        ldx: Option<LdxFile>,
    },
}

#[allow(dead_code)]
pub enum JobRequest {
    LoadSession {
        request_id: u64,
        source: LoadSessionSource,
    },
    DecodePhysicalChannel {
        request_id: u64,
        session_id: u64,
        ld: Arc<LdFile>,
        channel_idx: usize,
    },
    EvaluateMathChannel {
        request_id: u64,
        session_id: u64,
        math_id: u64,
        expression: String,
        evaluation_key: String,
        options: i3rs_core::math_engine::EvaluationOptions,
        aliases: std::collections::HashMap<String, String>,
        channel_data: std::collections::HashMap<String, ChannelData>,
    },
    BuildTrackData {
        request_id: u64,
        session_id: u64,
        ld: Arc<LdFile>,
    },
}

pub struct DecodedPhysicalChannel {
    pub channel_idx: usize,
    pub data: Vec<f64>,
    pub stats: ChannelStats,
}

pub struct EvaluatedMathChannel {
    pub samples: Vec<f64>,
    pub freq: u16,
    pub stats: ChannelStats,
}

pub enum JobResult {
    LoadSession {
        request_id: u64,
        result: Box<Result<LoadedSession, String>>,
    },
    DecodePhysicalChannel {
        session_id: u64,
        channel_idx: usize,
        result: Result<DecodedPhysicalChannel, String>,
    },
    BuildTrackData {
        session_id: u64,
        track_data: Option<TrackData>,
    },
    EvaluateMathChannel {
        session_id: u64,
        math_id: u64,
        expression: String,
        evaluation_key: String,
        result: Result<EvaluatedMathChannel, String>,
    },
}

fn perform_load_session(source: LoadSessionSource) -> Result<LoadedSession, String> {
    let _perf = crate::perf_metrics::scope("session open");
    match source {
        LoadSessionSource::Path(path) => {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let ldx = find_ldx_for_ld(&path);
            let ld = LdFile::open(&path)?;
            let data_duration = ld.duration_secs();
            // Compute source identity in the load job, before UI metadata needs it.
            ld.recording_id();
            let ld = std::sync::Arc::new(ld);
            let laps = {
                let _perf = crate::perf_metrics::scope("lap detection");
                detect_laps(&ld)
            };

            let ld = std::sync::Arc::into_inner(ld)
                .ok_or_else(|| "internal error while finalizing loaded session".to_string())?;

            Ok(LoadedSession {
                file_name,
                ld_path: Some(path),
                ldx,
                ld,
                laps,
                data_duration,
            })
        }
        LoadSessionSource::Bytes {
            file_name,
            bytes,
            ldx,
        } => {
            let ld = LdFile::from_bytes(bytes)?;
            let data_duration = ld.duration_secs();
            // Compute source identity in the load job, before UI metadata needs it.
            ld.recording_id();
            let ld = std::sync::Arc::new(ld);
            let laps = {
                let _perf = crate::perf_metrics::scope("lap detection");
                detect_laps(&ld)
            };

            let ld = std::sync::Arc::into_inner(ld)
                .ok_or_else(|| "internal error while finalizing loaded session".to_string())?;

            Ok(LoadedSession {
                file_name,
                ld_path: None,
                ldx,
                ld,
                laps,
                data_duration,
            })
        }
    }
}

fn perform_decode_physical_channel(
    ld: Arc<LdFile>,
    channel_idx: usize,
) -> Result<DecodedPhysicalChannel, String> {
    let channel = ld
        .channels
        .get(channel_idx)
        .ok_or_else(|| format!("channel index {channel_idx} out of range"))?;
    let _perf = crate::perf_metrics::scope("channel decode");
    let data = ld
        .read_channel_data(channel)
        .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
    let stats = compute_channel_stats(&data);
    Ok(DecodedPhysicalChannel {
        channel_idx,
        data,
        stats,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn perform_build_track_data(ld: Arc<LdFile>) -> Option<TrackData> {
    let _perf = crate::perf_metrics::scope("track-map draw");
    extract_gps_track(&ld)
}

fn perform_evaluate_math_channel(
    expression: String,
    channel_data: std::collections::HashMap<String, ChannelData>,
    aliases: std::collections::HashMap<String, String>,
    options: i3rs_core::math_engine::EvaluationOptions,
) -> Result<EvaluatedMathChannel, String> {
    let _perf = crate::perf_metrics::scope("math evaluation");
    let (samples, freq) = i3rs_core::math_engine::evaluate_expression_with_options(
        &expression,
        &channel_data,
        &aliases,
        &options,
    )
    .map_err(|err| err.to_string())?;
    let stats = compute_channel_stats(&samples);
    Ok(EvaluatedMathChannel {
        samples,
        freq,
        stats,
    })
}

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() {
    gloo_timers::future::TimeoutFuture::new(0).await;
}

#[cfg(target_arch = "wasm32")]
async fn perform_build_track_data_cooperative(ld: Arc<LdFile>) -> Option<TrackData> {
    let _perf = crate::perf_metrics::scope("track-map draw");
    // The same phases as `extract_gps_track`, yielding to the browser between them.
    let (lat_ch, lon_ch) = i3rs_core::track::gps_channels(&ld)?;
    yield_to_browser().await;
    let lat_data = ld.read_channel_data(lat_ch)?;
    yield_to_browser().await;
    let lon_data = ld.read_channel_data(lon_ch)?;
    yield_to_browser().await;
    let mut track =
        i3rs_core::track::gps_track_from_samples(&lat_data, lat_ch.freq, &lon_data, lon_ch.freq)?;
    yield_to_browser().await;
    i3rs_core::track::attach_corrected_distance(&mut track, &ld);
    Some(track)
}

#[cfg(target_arch = "wasm32")]
async fn run_wasm_job(request: JobRequest) -> JobResult {
    match request {
        JobRequest::LoadSession { request_id, source } => {
            yield_to_browser().await;
            let result = Box::new(perform_load_session(source));
            yield_to_browser().await;
            JobResult::LoadSession { request_id, result }
        }
        JobRequest::DecodePhysicalChannel {
            request_id: _,
            session_id,
            ld,
            channel_idx,
        } => {
            yield_to_browser().await;
            let result = perform_decode_physical_channel(ld, channel_idx);
            yield_to_browser().await;
            JobResult::DecodePhysicalChannel {
                session_id,
                channel_idx,
                result,
            }
        }
        JobRequest::BuildTrackData {
            request_id: _,
            session_id,
            ld,
        } => JobResult::BuildTrackData {
            session_id,
            track_data: perform_build_track_data_cooperative(ld).await,
        },
        JobRequest::EvaluateMathChannel {
            request_id: _,
            session_id,
            math_id,
            expression,
            evaluation_key,
            options,
            aliases,
            channel_data,
        } => {
            yield_to_browser().await;
            let result =
                perform_evaluate_math_channel(expression.clone(), channel_data, aliases, options);
            yield_to_browser().await;
            JobResult::EvaluateMathChannel {
                session_id,
                math_id,
                expression,
                evaluation_key,
                result,
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct BackgroundJobs {
    request_tx: crossbeam_channel::Sender<(JobRequest, egui::Context)>,
    result_rx: crossbeam_channel::Receiver<JobResult>,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundJobs {
    pub fn new() -> Self {
        let (request_tx, request_rx) =
            crossbeam_channel::unbounded::<(JobRequest, egui::Context)>();
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<JobResult>();

        std::thread::spawn(move || {
            while let Ok((request, ctx)) = request_rx.recv() {
                let result = match request {
                    JobRequest::LoadSession { request_id, source } => JobResult::LoadSession {
                        request_id,
                        result: Box::new(perform_load_session(source)),
                    },
                    JobRequest::DecodePhysicalChannel {
                        request_id: _,
                        session_id,
                        ld,
                        channel_idx,
                    } => JobResult::DecodePhysicalChannel {
                        session_id,
                        channel_idx,
                        result: perform_decode_physical_channel(ld, channel_idx),
                    },
                    JobRequest::BuildTrackData {
                        request_id: _,
                        session_id,
                        ld,
                    } => JobResult::BuildTrackData {
                        session_id,
                        track_data: perform_build_track_data(ld),
                    },
                    JobRequest::EvaluateMathChannel {
                        request_id: _,
                        session_id,
                        math_id,
                        expression,
                        evaluation_key,
                        options,
                        aliases,
                        channel_data,
                    } => JobResult::EvaluateMathChannel {
                        session_id,
                        math_id,
                        expression: expression.clone(),
                        evaluation_key,
                        result: perform_evaluate_math_channel(
                            expression,
                            channel_data,
                            aliases,
                            options,
                        ),
                    },
                };

                if result_tx.send(result).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        Self {
            request_tx,
            result_rx,
        }
    }

    pub fn submit(&self, request: JobRequest, ctx: &egui::Context) -> Result<(), String> {
        self.request_tx
            .send((request, ctx.clone()))
            .map_err(|err| format!("failed to submit background job: {err}"))
    }

    pub fn try_recv(&self) -> Option<JobResult> {
        self.result_rx.try_recv().ok()
    }
}

#[cfg(target_arch = "wasm32")]
pub struct BackgroundJobs {
    result_tx: std::sync::mpsc::Sender<JobResult>,
    result_rx: std::sync::mpsc::Receiver<JobResult>,
}

#[cfg(target_arch = "wasm32")]
impl BackgroundJobs {
    pub fn new() -> Self {
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        Self {
            result_tx,
            result_rx,
        }
    }

    pub fn submit(&self, request: JobRequest, ctx: &egui::Context) -> Result<(), String> {
        let tx = self.result_tx.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = run_wasm_job(request).await;
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Ok(())
    }

    pub fn try_recv(&self) -> Option<JobResult> {
        self.result_rx.try_recv().ok()
    }
}

pub fn load_session_from_path(path: PathBuf) -> Result<LoadedSession, String> {
    perform_load_session(LoadSessionSource::Path(path))
}

pub fn load_session_from_bytes(
    file_name: String,
    bytes: Vec<u8>,
    ldx: Option<LdxFile>,
) -> Result<LoadedSession, String> {
    perform_load_session(LoadSessionSource::Bytes {
        file_name,
        bytes,
        ldx,
    })
}

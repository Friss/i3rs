use std::path::PathBuf;
use std::sync::Arc;

use crate::state::{ChannelStats, DownsampleSeriesKey, compute_channel_stats};
use i3rs_core::{
    ChannelData, DownsampledPoint, Lap, LdFile, LdxFile, TrackData, detect_laps,
    evaluate_expression_with_aliases, find_ldx_for_ld,
};
#[cfg(not(target_arch = "wasm32"))]
use i3rs_core::{downsample_minmax, extract_gps_track};

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
        aliases: std::collections::HashMap<String, String>,
        channel_data: std::collections::HashMap<String, ChannelData>,
    },
    BuildTrackData {
        request_id: u64,
        session_id: u64,
        ld: Arc<LdFile>,
    },
    BuildDownsampledSeries {
        request_id: u64,
        session_id: u64,
        key: DownsampleSeriesKey,
        data: Arc<[f64]>,
        freq: u16,
        start_sample: usize,
        end_sample: usize,
        target_width: usize,
    },
}

pub struct DecodedPhysicalChannel {
    pub channel_idx: usize,
    pub data: Vec<f64>,
    pub stats: ChannelStats,
    pub freq: u16,
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
        result: Result<EvaluatedMathChannel, String>,
    },
    BuildDownsampledSeries {
        session_id: u64,
        key: DownsampleSeriesKey,
        points: Vec<DownsampledPoint>,
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
        freq: channel.freq,
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
) -> Result<EvaluatedMathChannel, String> {
    let _perf = crate::perf_metrics::scope("math evaluation");
    let (samples, freq) = evaluate_expression_with_aliases(&expression, &channel_data, &aliases)
        .map_err(|err| err.to_string())?;
    let stats = compute_channel_stats(&samples);
    Ok(EvaluatedMathChannel {
        samples,
        freq,
        stats,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn perform_build_downsampled_series(
    data: Arc<[f64]>,
    freq: u16,
    start_sample: usize,
    end_sample: usize,
    target_width: usize,
) -> Vec<DownsampledPoint> {
    let visible_end = end_sample.min(data.len());
    let visible_start = start_sample.min(visible_end);
    if visible_start >= visible_end {
        return Vec::new();
    }
    downsample_minmax(
        &data[visible_start..visible_end],
        freq,
        visible_start,
        target_width,
    )
}

#[cfg(target_arch = "wasm32")]
const WASM_COOPERATIVE_CHUNK: usize = 4_096;

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() {
    gloo_timers::future::TimeoutFuture::new(0).await;
}

#[cfg(target_arch = "wasm32")]
fn find_gps_channel<'a>(ld: &'a LdFile, suffixes: &[&str]) -> Option<&'a i3rs_core::ChannelMeta> {
    ld.channels.iter().find(|channel| {
        let normalized = i3rs_core::normalize_channel_name(&channel.name);
        normalized.contains("gps") && suffixes.iter().any(|suffix| normalized.contains(suffix))
    })
}

#[cfg(target_arch = "wasm32")]
async fn perform_build_track_data_cooperative(ld: Arc<LdFile>) -> Option<TrackData> {
    let _perf = crate::perf_metrics::scope("track-map draw");
    let lat_ch = find_gps_channel(&ld, &["latitude", "lat"])?;
    let lon_ch = find_gps_channel(&ld, &["longitude", "lon", "long"])?;

    yield_to_browser().await;
    let lat_data = ld.read_channel_data(lat_ch)?;
    yield_to_browser().await;
    let lon_data = ld.read_channel_data(lon_ch)?;

    let n = lat_data.len().min(lon_data.len());
    if n == 0 {
        return None;
    }

    let freq = lat_ch.freq;
    let mut lat_sum = 0.0;
    let mut lat_count = 0usize;
    let mut lon_sum = 0.0;
    let mut lon_count = 0usize;

    for start in (0..n).step_by(WASM_COOPERATIVE_CHUNK) {
        let end = (start + WASM_COOPERATIVE_CHUNK).min(n);
        for idx in start..end {
            let lat = lat_data[idx];
            if lat.is_finite() {
                lat_sum += lat;
                lat_count += 1;
            }
            let lon = lon_data[idx];
            if lon.is_finite() {
                lon_sum += lon;
                lon_count += 1;
            }
        }
        yield_to_browser().await;
    }

    let mean_lat = lat_sum / lat_count.max(1) as f64;
    let mean_lon = lon_sum / lon_count.max(1) as f64;
    let cos_lat = mean_lat.to_radians().cos();

    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut time = Vec::with_capacity(n);

    for start in (0..n).step_by(WASM_COOPERATIVE_CHUNK) {
        let end = (start + WASM_COOPERATIVE_CHUNK).min(n);
        for idx in start..end {
            let lat = lat_data[idx];
            let lon = lon_data[idx];
            if lat.is_finite() && lon.is_finite() {
                x.push((lon - mean_lon) * cos_lat);
                y.push(lat - mean_lat);
            } else if let (Some(&prev_x), Some(&prev_y)) = (x.last(), y.last()) {
                x.push(prev_x);
                y.push(prev_y);
            } else {
                x.push(0.0);
                y.push(0.0);
            }
            time.push(idx as f64 / freq as f64);
        }
        yield_to_browser().await;
    }

    Some(TrackData::from_normalized_parts(x, y, time, freq))
}

#[cfg(target_arch = "wasm32")]
async fn perform_build_downsampled_series_cooperative(
    data: Arc<[f64]>,
    freq: u16,
    start_sample: usize,
    end_sample: usize,
    target_width: usize,
) -> Vec<DownsampledPoint> {
    let visible_end = end_sample.min(data.len());
    let visible_start = start_sample.min(visible_end);
    if visible_start >= visible_end || target_width == 0 || freq == 0 {
        return Vec::new();
    }

    let samples = &data[visible_start..visible_end];
    let freq_f = freq as f64;
    if samples.len() <= target_width.saturating_mul(2) {
        let mut points = Vec::with_capacity(samples.len());
        for start in (0..samples.len()).step_by(WASM_COOPERATIVE_CHUNK) {
            let end = (start + WASM_COOPERATIVE_CHUNK).min(samples.len());
            for (offset, &value) in samples[start..end].iter().enumerate() {
                let idx = start + offset;
                points.push(DownsampledPoint {
                    time: (visible_start + idx) as f64 / freq_f,
                    min: value,
                    max: value,
                });
            }
            yield_to_browser().await;
        }
        return points;
    }

    let mut result = Vec::with_capacity(target_width);
    let bucket_size_f = samples.len() as f64 / target_width as f64;
    for bucket in 0..target_width {
        let start = (bucket as f64 * bucket_size_f) as usize;
        let end = (((bucket + 1) as f64) * bucket_size_f) as usize;
        let end = end.min(samples.len());
        if start >= end {
            continue;
        }

        let mut min_v = samples[start];
        let mut max_v = samples[start];
        for &value in &samples[start + 1..end] {
            if value < min_v {
                min_v = value;
            }
            if value > max_v {
                max_v = value;
            }
        }

        let mid_sample = visible_start + (start + end) / 2;
        result.push(DownsampledPoint {
            time: mid_sample as f64 / freq_f,
            min: min_v,
            max: max_v,
        });

        if bucket % 256 == 255 {
            yield_to_browser().await;
        }
    }

    result
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
            aliases,
            channel_data,
        } => {
            yield_to_browser().await;
            let result = perform_evaluate_math_channel(expression.clone(), channel_data, aliases);
            yield_to_browser().await;
            JobResult::EvaluateMathChannel {
                session_id,
                math_id,
                expression,
                result,
            }
        }
        JobRequest::BuildDownsampledSeries {
            request_id: _,
            session_id,
            key,
            data,
            freq,
            start_sample,
            end_sample,
            target_width,
        } => JobResult::BuildDownsampledSeries {
            session_id,
            key,
            points: perform_build_downsampled_series_cooperative(
                data,
                freq,
                start_sample,
                end_sample,
                target_width,
            )
            .await,
        },
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
                        aliases,
                        channel_data,
                    } => JobResult::EvaluateMathChannel {
                        session_id,
                        math_id,
                        expression: expression.clone(),
                        result: perform_evaluate_math_channel(expression, channel_data, aliases),
                    },
                    JobRequest::BuildDownsampledSeries {
                        request_id: _,
                        session_id,
                        key,
                        data,
                        freq,
                        start_sample,
                        end_sample,
                        target_width,
                    } => JobResult::BuildDownsampledSeries {
                        session_id,
                        key,
                        points: perform_build_downsampled_series(
                            data,
                            freq,
                            start_sample,
                            end_sample,
                            target_width,
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

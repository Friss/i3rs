use std::path::PathBuf;
use std::sync::Arc;

use crate::state::{ChannelStats, compute_channel_stats};
use i3rs_core::{ChannelData, Lap, LdFile, LdxFile, detect_laps, find_ldx_for_ld};

pub struct LoadedSession {
    pub file_name: String,
    pub ld_path: Option<PathBuf>,
    pub ldx: Option<LdxFile>,
    pub ld: LdFile,
    pub laps: Vec<Lap>,
    pub data_duration: f64,
    pub analysis: Option<i3rs_core::ranges::RecordingAnalysis>,
    pub analysis_warning: Option<String>,
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
        correction_key: String,
        settings: i3rs_core::corrections::CorrectionSettings,
        laps: Vec<Lap>,
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
    pub freq: f64,
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
        correction_key: String,
        result: Box<Result<i3rs_core::corrections::CorrectedRecording, String>>,
    },
    EvaluateMathChannel {
        session_id: u64,
        math_id: u64,
        expression: String,
        evaluation_key: String,
        result: Result<EvaluatedMathChannel, String>,
    },
}

pub(crate) fn read_analysis_sidecar(
    path: &std::path::Path,
    ld: &LdFile,
) -> (Option<i3rs_core::ranges::RecordingAnalysis>, Option<String>) {
    let path = path.with_extension("i3rs.json");
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None),
        Err(e) => return (None, Some(format!("Analysis sidecar: {e}"))),
    };
    let result = serde_json::from_str::<i3rs_core::ranges::RecordingAnalysis>(&json)
        .map_err(|e| e.to_string())
        .and_then(|analysis| {
            analysis.validate()?;
            if analysis.recording != ld.recording_id()
                || (analysis.duration - ld.duration_secs()).abs() > 1e-6
            {
                return Err("Sidecar belongs to another recording".into());
            }
            Ok(analysis)
        });
    match result {
        Ok(a) => (Some(a), None),
        Err(e) => (None, Some(format!("Analysis sidecar: {e}"))),
    }
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
            let (mut analysis, mut analysis_warning) = read_analysis_sidecar(&path, &ld);
            if analysis.is_none()
                && let Some(ldx) = &ldx
            {
                match ldx.specialty_analysis(ld.recording_id().into(), data_duration) {
                    Ok(imported) => analysis = imported,
                    Err(error) => analysis_warning = Some(format!("LDX analysis: {error}")),
                }
            }
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
                analysis,
                analysis_warning,
            })
        }
        LoadSessionSource::Bytes {
            file_name,
            bytes,
            ldx,
        } => {
            let ld = LdFile::from_bytes(bytes)?;
            let data_duration = ld.duration_secs();
            let (analysis, analysis_warning) = ldx.as_ref().map_or((None, None), |ldx| {
                match ldx.specialty_analysis(ld.recording_id().into(), data_duration) {
                    Ok(a) => (a, None),
                    Err(e) => (None, Some(format!("LDX analysis: {e}"))),
                }
            });
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
                analysis,
                analysis_warning,
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

fn perform_build_track_data(
    ld: Arc<LdFile>,
    settings: i3rs_core::corrections::CorrectionSettings,
    laps: Vec<Lap>,
) -> Result<i3rs_core::corrections::CorrectedRecording, String> {
    let _perf = crate::perf_metrics::scope("track-map draw");
    i3rs_core::corrections::build_corrected_recording(&ld, &settings, &laps)
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
async fn perform_build_track_data_cooperative(
    ld: Arc<LdFile>,
    settings: i3rs_core::corrections::CorrectionSettings,
    laps: Vec<Lap>,
) -> Result<i3rs_core::corrections::CorrectedRecording, String> {
    if settings != i3rs_core::corrections::CorrectionSettings::default() {
        yield_to_browser().await;
        return perform_build_track_data(ld, settings, laps);
    }
    let mut track = None;
    if let Some((a, b)) = i3rs_core::track::gps_channels(&ld) {
        yield_to_browser().await;
        let lat = ld.read_channel_data(a).ok_or("Latitude decode failed")?;
        yield_to_browser().await;
        let lon = ld.read_channel_data(b).ok_or("Longitude decode failed")?;
        yield_to_browser().await;
        track = i3rs_core::track::gps_track_from_samples(&lat, a.freq, &lon, b.freq);
    }
    yield_to_browser().await;
    let speed = i3rs_core::distance::corrected_speed_from_ld(&ld);
    yield_to_browser().await;
    let distance = speed.as_ref().and_then(|(v, f)| {
        i3rs_core::distance::integrate_speed_series(v, *f, "m/s").map(|d| (d, *f))
    });
    if let Some(t) = &mut track {
        t.corrected_speed = speed.clone();
        t.distance = distance.clone();
    }
    let gps_speed = ld
        .find_channel_by_name("GPS Speed")
        .filter(|c| c.freq > 0.0)
        .and_then(|c| {
            let scale = i3rs_core::distance::speed_unit_to_mps(&c.unit)?;
            Some((
                ld.read_channel_data(c)?.iter().map(|v| v * scale).collect(),
                c.freq,
            ))
        });
    Ok(i3rs_core::corrections::CorrectedRecording {
        gps_speed,
        speed,
        distance,
        track,
        messages: vec![],
    })
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
            correction_key,
            settings,
            laps,
            ld,
        } => {
            yield_to_browser().await;
            let result = perform_build_track_data_cooperative(ld, settings, laps).await;
            yield_to_browser().await;
            JobResult::BuildTrackData {
                session_id,
                correction_key,
                result: Box::new(result),
            }
        }
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
                        correction_key,
                        settings,
                        laps,
                        ld,
                    } => JobResult::BuildTrackData {
                        session_id,
                        correction_key,
                        result: Box::new(perform_build_track_data(ld, settings, laps)),
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

#[cfg(test)]
mod sidecar_tests {
    use super::*;
    #[test]
    fn sidecar_loading_validates_identity_and_reports_invalid_json() {
        let ld = LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        let path = std::env::temp_dir().join(format!("i3r3-sidecar-{}.ld", std::process::id()));
        let sidecar = path.with_extension("i3rs.json");
        let mut analysis = i3rs_core::ranges::RecordingAnalysis::from_laps(
            ld.recording_id().into(),
            ld.duration_secs(),
            &detect_laps(&ld),
        );
        analysis.details.driver = "Sidecar driver".into();
        std::fs::write(&sidecar, serde_json::to_string(&analysis).unwrap()).unwrap();
        let (a, w) = read_analysis_sidecar(&path, &ld);
        assert!(w.is_none());
        assert_eq!(a.unwrap().details.driver, "Sidecar driver");
        analysis.recording = "wrong".into();
        std::fs::write(&sidecar, serde_json::to_string(&analysis).unwrap()).unwrap();
        let (a, w) = read_analysis_sidecar(&path, &ld);
        assert!(a.is_none());
        assert!(w.unwrap().contains("another recording"));
        std::fs::write(&sidecar, "{").unwrap();
        let (a, w) = read_analysis_sidecar(&path, &ld);
        assert!(a.is_none());
        assert!(w.is_some());
        std::fs::remove_file(sidecar).unwrap();
        assert!(read_analysis_sidecar(&path, &ld).0.is_none());
    }
}

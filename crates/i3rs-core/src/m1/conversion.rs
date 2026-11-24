//! Native conversion persistence for M1 images and complete raw captures.
//!
//! Each nonempty recording segment becomes a fresh `.ld` file. The source is
//! read only; a new child directory and create-new files protect existing data.
//! Failed/cancelled output retains its incomplete manifest and received bytes.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::capture_decode::{decode_capture_with_cancel, decode_partial_capture};
use super::image_decode::{DecodedImage, ImageDecodeLimits, decode_image};
use super::ld_export::{RESAMPLING_POLICY, encode_segment};
use super::transport::CancellationToken;

pub const CONVERSION_FORMAT: &str = "i3rs.m1.converted-recordings.v1";
const LD_HEADER_SIZE: usize = 0x6e2;
const MAX_FILES: usize = 10_000;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const IO_CHUNK: usize = 64 * 1024;
static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct ConversionLimits {
    pub decode: ImageDecodeLimits,
    pub max_file_bytes: usize,
    /// Sum of LD bytes, excluding the separately bounded JSON manifest.
    pub max_total_output_bytes: u64,
    pub max_files: usize,
}

impl Default for ConversionLimits {
    fn default() -> Self {
        Self {
            decode: ImageDecodeLimits::default(),
            max_file_bytes: 256 * 1024 * 1024,
            max_total_output_bytes: 2 * 1024 * 1024 * 1024,
            max_files: 1024,
        }
    }
}

impl ConversionLimits {
    fn validate(&self) -> Result<(), String> {
        if self.max_file_bytes < LD_HEADER_SIZE
            || self.max_total_output_bytes < LD_HEADER_SIZE as u64
            || !(1..=MAX_FILES).contains(&self.max_files)
            || self.decode.max_image_bytes == 0
        {
            return Err("invalid M1 conversion output/input limits".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Image,
    RawCapture,
    PartialRawCapture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionSource {
    pub kind: SourceKind,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionState {
    Incomplete,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedFile {
    pub file_name: String,
    /// Zero-based source logging system ID.
    pub system: u8,
    pub segment: usize,
    pub start_ticks: u64,
    pub end_ticks: u64,
    pub channels: usize,
    pub expected_bytes: u64,
    pub bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionManifest {
    pub format: String,
    pub state: ConversionState,
    pub source: ConversionSource,
    pub started_unix_millis: u64,
    pub resampling_policy: String,
    pub clock_hz: f64,
    pub used_default_clock: bool,
    /// Internal checks do not establish compatibility with MoTeC i2/Tune.
    pub vendor_compatibility_validated: bool,
    pub warnings: Vec<String>,
    pub files: Vec<ConvertedFile>,
    pub total_bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub directory: PathBuf,
    pub manifest: ConversionManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionStage {
    ReadingInput,
    Decoding,
    PreparingOutput,
    WritingOutput,
    Finished,
}

#[derive(Debug, Clone)]
pub struct ConversionProgress {
    pub stage: ConversionStage,
    pub system: Option<u8>,
    pub segment: Option<usize>,
    pub files_complete: usize,
    pub files_total: usize,
    pub bytes_written: u64,
}

/// Convert a regular `.m1dli` source into a new child of an existing directory.
/// Input growth is rejected using at most one byte beyond the configured cap.
/// Cancellation is checked during file I/O and between decode/encode stages;
/// bounded CPU decoding/encoding is not interrupted in the middle of a stage.
pub fn convert_image_file(
    source: &Path,
    destination_base: &Path,
    limits: &ConversionLimits,
    cancellation: CancellationToken,
    mut progress: impl FnMut(ConversionProgress),
) -> Result<ConversionResult, ConversionError> {
    limits.validate().map_err(ConversionError::before_output)?;
    check_cancelled(&cancellation).map_err(ConversionError::before_output)?;
    progress(initial_progress(ConversionStage::ReadingInput));
    let bytes = read_bounded(source, limits.decode.max_image_bytes, &cancellation)
        .map_err(ConversionError::before_output)?;
    check_cancelled(&cancellation).map_err(ConversionError::before_output)?;
    progress(initial_progress(ConversionStage::Decoding));
    let decoded = decode_image(&bytes, &limits.decode).map_err(ConversionError::before_output)?;
    drop(bytes);
    persist_decoded(
        &decoded,
        ConversionSource {
            kind: SourceKind::Image,
            path: source.display().to_string(),
        },
        destination_base,
        limits,
        cancellation,
        progress,
    )
}

/// Convert only a complete, consistent i3rs raw capture through its bounded
/// adapter. This does not write or label a reconstructed official M1 image.
pub fn convert_capture_directory(
    source: &Path,
    destination_base: &Path,
    limits: &ConversionLimits,
    cancellation: CancellationToken,
    progress: impl FnMut(ConversionProgress),
) -> Result<ConversionResult, ConversionError> {
    convert_capture(
        source,
        destination_base,
        limits,
        cancellation,
        progress,
        SourceKind::RawCapture,
    )
}

/// Export the saved portion of an interrupted acquisition with partial filenames and provenance.
pub fn recover_capture_directory(
    source: &Path,
    destination_base: &Path,
    limits: &ConversionLimits,
    cancellation: CancellationToken,
    progress: impl FnMut(ConversionProgress),
) -> Result<ConversionResult, ConversionError> {
    convert_capture(
        source,
        destination_base,
        limits,
        cancellation,
        progress,
        SourceKind::PartialRawCapture,
    )
}

fn convert_capture(
    source: &Path,
    destination_base: &Path,
    limits: &ConversionLimits,
    cancellation: CancellationToken,
    mut progress: impl FnMut(ConversionProgress),
    kind: SourceKind,
) -> Result<ConversionResult, ConversionError> {
    limits.validate().map_err(ConversionError::before_output)?;
    check_cancelled(&cancellation).map_err(ConversionError::before_output)?;
    progress(initial_progress(ConversionStage::Decoding));
    let decoded = if kind == SourceKind::PartialRawCapture {
        decode_partial_capture(source, &limits.decode, &cancellation)
    } else {
        decode_capture_with_cancel(source, &limits.decode, &cancellation)
    }
    .map_err(ConversionError::before_output)?;
    persist_decoded(
        &decoded,
        ConversionSource {
            kind,
            path: source.display().to_string(),
        },
        destination_base,
        limits,
        cancellation,
        progress,
    )
}

/// Persist prepared telemetry with the LD writer's explicit resampling policy.
/// No existing directory/file is reused. Before directory creation, plan and
/// metadata limits are checked. After creation, failures retain incomplete state.
pub fn persist_decoded(
    decoded: &DecodedImage,
    source: ConversionSource,
    destination_base: &Path,
    limits: &ConversionLimits,
    cancellation: CancellationToken,
    mut progress: impl FnMut(ConversionProgress),
) -> Result<ConversionResult, ConversionError> {
    let mut directory = None;
    let mut manifest: Option<ConversionManifest> = None;
    let result = (|| -> Result<ConversionResult, String> {
        limits.validate()?;
        check_cancelled(&cancellation)?;
        if !fs::metadata(destination_base).map_err(io_error)?.is_dir() {
            return Err("conversion destination must be an existing directory".into());
        }
        if source.path.len() > 16 * 1024
            || decoded.warnings.len() > 128
            || decoded
                .warnings
                .iter()
                .try_fold(0usize, |sum, warning| sum.checked_add(warning.len()))
                .is_none_or(|total| total > 64 * 1024)
            || !decoded.telemetry.clock_hz.is_finite()
            || decoded.telemetry.clock_hz <= 0.0
        {
            return Err(
                "conversion source metadata exceeds its bounds or has an invalid clock".into(),
            );
        }
        let mut plan = Vec::new();
        let mut identifiers = BTreeSet::new();
        for system in &decoded.telemetry.systems {
            if system.system_id >= 8 {
                return Err("conversion logging system must be in 0..8".into());
            }
            for segment in &system.segments {
                if !segment
                    .channels
                    .iter()
                    .any(|channel| !channel.samples.is_empty())
                {
                    continue;
                }
                if plan.len() >= limits.max_files {
                    return Err("M1 conversion exceeds its output file-count limit".into());
                }
                if segment.index.checked_add(1).is_none() {
                    return Err("M1 segment output identifier overflow".into());
                }
                if !identifiers.insert((system.system_id, segment.index)) {
                    return Err("duplicate M1 system/segment output identifier".into());
                }
                plan.push((system.system_id, segment));
            }
        }
        if plan.is_empty() {
            return Err("M1 source contains no numeric recording segments".into());
        }
        progress(ConversionProgress {
            files_total: plan.len(),
            ..initial_progress(ConversionStage::PreparingOutput)
        });
        check_cancelled(&cancellation)?;
        let child = create_directory(destination_base)?;
        directory = Some(child.clone());
        manifest = Some(ConversionManifest {
            format: CONVERSION_FORMAT.into(),
            state: ConversionState::Incomplete,
            source,
            started_unix_millis: unix_millis(),
            resampling_policy: RESAMPLING_POLICY.into(),
            clock_hz: decoded.telemetry.clock_hz,
            used_default_clock: decoded.telemetry.used_default_clock,
            vendor_compatibility_validated: false,
            warnings: decoded.warnings.clone(),
            files: Vec::new(),
            total_bytes: 0,
            error: None,
        });
        let current = manifest.as_mut().unwrap();
        write_manifest(&child.join("manifest.json"), current)?;
        let mut completed = 0;
        for (system, segment) in &plan {
            check_cancelled(&cancellation)?;
            let report = ConversionProgress {
                stage: ConversionStage::WritingOutput,
                system: Some(*system),
                segment: Some(segment.index),
                files_complete: completed,
                files_total: plan.len(),
                bytes_written: current.total_bytes,
            };
            progress(report.clone());
            let remaining = limits.max_total_output_bytes - current.total_bytes;
            let cap = limits
                .max_file_bytes
                .min(usize::try_from(remaining).unwrap_or(usize::MAX));
            if cap < LD_HEADER_SIZE {
                return Err("M1 conversion aggregate output byte limit exhausted".into());
            }
            let bytes = encode_segment(segment, &decoded.session, cap)?;
            if bytes.len() > cap {
                return Err("LD encoder exceeded its explicit output byte limit".into());
            }
            check_cancelled(&cancellation)?;
            let prefix = if current.source.kind == SourceKind::PartialRawCapture {
                "partial-"
            } else {
                ""
            };
            let name = format!(
                "{prefix}system-{:02}-segment-{:04}.ld",
                u16::from(*system) + 1,
                segment.index + 1
            );
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(child.join(&name))
                .map_err(io_error)?;
            current.files.push(ConvertedFile {
                file_name: name,
                system: *system,
                segment: segment.index,
                start_ticks: segment.start_ticks,
                end_ticks: segment.end_ticks,
                channels: segment.channels.len(),
                expected_bytes: bytes.len() as u64,
                bytes: 0,
                complete: false,
            });
            replace_manifest(&child, current)?;
            let result = (|| -> Result<(), String> {
                for chunk in bytes.chunks(IO_CHUNK) {
                    check_cancelled(&cancellation)?;
                    file.write_all(chunk).map_err(io_error)?;
                }
                file.sync_all().map_err(io_error)
            })();
            let actual = file.metadata().map_err(io_error)?.len();
            let entry = current.files.last_mut().unwrap();
            entry.bytes = actual;
            entry.complete = result.is_ok() && actual == entry.expected_bytes;
            current.total_bytes += actual;
            result?;
            if !entry.complete {
                return Err("LD output file length differs from encoded bytes".into());
            }
            completed += 1;
            replace_manifest(&child, current)?;
            progress(ConversionProgress {
                files_complete: completed,
                bytes_written: current.total_bytes,
                ..report
            });
        }
        check_cancelled(&cancellation)?;
        current.state = ConversionState::Complete;
        replace_manifest(&child, current)?;
        progress(ConversionProgress {
            stage: ConversionStage::Finished,
            system: None,
            segment: None,
            files_complete: completed,
            files_total: plan.len(),
            bytes_written: current.total_bytes,
        });
        Ok(ConversionResult {
            directory: child,
            manifest: current.clone(),
        })
    })();
    result.map_err(|message| {
        let mut manifest_diagnostic = None;
        if let (Some(path), Some(current)) = (&directory, &mut manifest) {
            current.state = ConversionState::Incomplete;
            current.error = Some(message.clone());
            if let Err(error) = replace_manifest(path, current) {
                manifest_diagnostic =
                    Some(format!("could not save conversion failure state: {error}"));
            }
        }
        ConversionError {
            message,
            directory,
            manifest_diagnostic,
        }
    })
}

fn initial_progress(stage: ConversionStage) -> ConversionProgress {
    ConversionProgress {
        stage,
        system: None,
        segment: None,
        files_complete: 0,
        files_total: 0,
        bytes_written: 0,
    }
}

fn read_bounded(
    path: &Path,
    limit: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
    check_cancelled(cancellation)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO must be rejected as a non-file without waiting for a writer
        // in open(), where cooperative cancellation cannot be observed.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("M1 input must be a regular file within its image byte limit".into());
    }
    let expected = usize::try_from(metadata.len()).map_err(|_| "M1 input length overflow")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected)
        .map_err(|_| "could not allocate bounded M1 input buffer")?;
    let mut chunk = [0; IO_CHUNK];
    loop {
        check_cancelled(cancellation)?;
        let available = limit - bytes.len();
        let want = chunk.len().min(available.saturating_add(1));
        let count = file.read(&mut chunk[..want]).map_err(io_error)?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > available {
            return Err("M1 input grew beyond its image byte limit".into());
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| "could not grow bounded M1 input buffer")?;
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("M1 conversion cancelled".into())
    } else {
        Ok(())
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| u64::try_from(time.as_millis()).ok())
        .unwrap_or(0)
}

fn create_directory(base: &Path) -> Result<PathBuf, String> {
    for _ in 0..32 {
        let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let child = base.join(format!(
            "i3rs-m1-converted-{}-{}-{sequence}",
            unix_millis(),
            std::process::id()
        ));
        match fs::create_dir(&child) {
            Ok(()) => return Ok(child),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Err("could not create a unique M1 conversion directory".into())
}

struct LimitedWriter<W> {
    inner: W,
    remaining: usize,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other(
                "conversion manifest exceeds its byte limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn write_manifest(path: &Path, manifest: &ConversionManifest) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    let mut writer = LimitedWriter {
        inner: file,
        remaining: MAX_MANIFEST_BYTES,
    };
    let result = (|| -> Result<(), String> {
        serde_json::to_writer_pretty(&mut writer, manifest).map_err(|error| error.to_string())?;
        writer.write_all(b"\n").map_err(io_error)?;
        writer.inner.sync_all().map_err(io_error)
    })();
    drop(writer);
    if result.is_err() {
        // Creation succeeded above, so this partial file is ours. A
        // preexisting destination fails create_new before reaching this point.
        let _ = fs::remove_file(path);
    }
    result
}

fn replace_manifest(directory: &Path, manifest: &ConversionManifest) -> Result<(), String> {
    let next = directory.join("manifest.next.json");
    write_manifest(&next, manifest)?;
    fs::rename(next, directory.join("manifest.json")).map_err(io_error)
}

fn io_error(error: io::Error) -> String {
    format!("M1 conversion I/O failed: {error}")
}

#[derive(Debug, Clone)]
pub struct ConversionError {
    pub message: String,
    pub directory: Option<PathBuf>,
    pub manifest_diagnostic: Option<String>,
}
impl ConversionError {
    fn before_output(message: String) -> Self {
        Self {
            message,
            directory: None,
            manifest_diagnostic: None,
        }
    }
}
impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(path) = &self.directory {
            write!(f, "; incomplete conversion retained at {}", path.display())?;
        }
        if let Some(message) = &self.manifest_diagnostic {
            write!(f, "; {message}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ConversionError {}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    use crate::Session;
    use crate::m1::catalog::ValueType;
    use crate::m1::telemetry::{
        DecodedSegment, DecodedSystem, DecodedTelemetry, TelemetryChannel, TimestampedSample,
    };

    struct TempDirectory(PathBuf);
    impl TempDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "i3rs-m1-conversion-test-{}-{}-{}",
                unix_millis(),
                std::process::id(),
                DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn decoded() -> DecodedImage {
        let segments = (0..2)
            .map(|index| DecodedSegment {
                index,
                start_ticks: index as u64 * 250_000_000,
                end_ticks: index as u64 * 250_000_000 + 125_000_000,
                channels: vec![TelemetryChannel {
                    data_type: ValueType::F32,
                    base: 1,
                    address: 0x100,
                    name: "Engine Speed".into(),
                    unit: "rpm".into(),
                    enum_labels: Default::default(),
                    display_decimals: 1,
                    frequency_hz: 1.,
                    samples: vec![
                        TimestampedSample {
                            time_seconds: 0.,
                            value: 1000.,
                            source_word: 1000f32.to_bits(),
                        },
                        TimestampedSample {
                            time_seconds: 1.,
                            value: 2000.,
                            source_word: 2000f32.to_bits(),
                        },
                    ],
                }],
            })
            .collect();
        DecodedImage {
            telemetry: DecodedTelemetry {
                systems: vec![DecodedSystem {
                    system_id: 0,
                    segments,
                }],
                clock_hz: 125_000_000.,
                used_default_clock: true,
            },
            session: Session {
                date: String::new(),
                time: String::new(),
                driver: "Test Driver".into(),
                vehicle_id: "Test Vehicle".into(),
                venue: "Test Venue".into(),
                short_comment: String::new(),
                device_serial: 42,
                device_type: "M1".into(),
                device_version: 0,
                num_channels_header: 0,
            },
            warnings: vec!["Synthetic source; vendor compatibility unvalidated.".into()],
        }
    }

    fn source() -> ConversionSource {
        ConversionSource {
            kind: SourceKind::Image,
            path: "synthetic.m1dli".into(),
        }
    }
    fn saved_manifest(directory: &Path) -> ConversionManifest {
        serde_json::from_reader(File::open(directory.join("manifest.json")).unwrap()).unwrap()
    }

    #[test]
    fn writes_fresh_recordings_and_complete_manifest_without_overwriting_parent_files() {
        let base = TempDirectory::new();
        fs::write(base.0.join("manifest.json"), b"existing manifest").unwrap();
        fs::write(
            base.0.join("system-01-segment-0001.ld"),
            b"existing recording",
        )
        .unwrap();
        let decoded = decoded();
        let result = persist_decoded(
            &decoded,
            source(),
            &base.0,
            &ConversionLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        let manifest = saved_manifest(&result.directory);
        assert_eq!(manifest.state, ConversionState::Complete);
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.resampling_policy, RESAMPLING_POLICY);
        assert_eq!(manifest.warnings, decoded.warnings);
        assert!(!manifest.vendor_compatibility_validated);
        for file in &manifest.files {
            assert!(file.complete && file.bytes == file.expected_bytes);
            let bytes = fs::read(result.directory.join(&file.file_name)).unwrap();
            let recording = crate::LdFile::from_bytes(bytes).unwrap();
            assert_eq!(recording.channels.len(), 1);
            assert_eq!(recording.channels[0].name, "Engine Speed");
            assert_eq!(
                recording.read_channel_data(&recording.channels[0]).unwrap(),
                [1000., 2000.]
            );
        }
        assert_eq!(
            manifest.total_bytes,
            manifest.files.iter().map(|file| file.bytes).sum::<u64>()
        );
        assert_eq!(
            fs::read(base.0.join("manifest.json")).unwrap(),
            b"existing manifest"
        );
        assert_eq!(
            fs::read(base.0.join("system-01-segment-0001.ld")).unwrap(),
            b"existing recording"
        );
        let again = persist_decoded(
            &decoded,
            source(),
            &base.0,
            &ConversionLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        assert_ne!(again.directory, result.directory);
    }

    #[test]
    fn cancellation_retains_completed_output_and_readable_incomplete_manifest() {
        let base = TempDirectory::new();
        let cancellation = CancellationToken::default();
        let token = cancellation.clone();
        let error = persist_decoded(
            &decoded(),
            source(),
            &base.0,
            &ConversionLimits::default(),
            cancellation,
            |progress| {
                if progress.stage == ConversionStage::WritingOutput && progress.files_complete == 1
                {
                    token.cancel();
                }
            },
        )
        .unwrap_err();
        assert!(error.message.contains("cancelled"));
        let path = error.directory.unwrap();
        let manifest = saved_manifest(&path);
        assert_eq!(manifest.state, ConversionState::Incomplete);
        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.files[0].complete);
        assert!(manifest.error.unwrap().contains("cancelled"));
        assert!(path.join(&manifest.files[0].file_name).is_file());
    }

    #[test]
    fn per_file_and_aggregate_output_caps_retain_incomplete_state() {
        let base = TempDirectory::new();
        let decoded = decoded();
        let limits = ConversionLimits {
            max_file_bytes: LD_HEADER_SIZE,
            ..ConversionLimits::default()
        };
        let error = persist_decoded(
            &decoded,
            source(),
            &base.0,
            &limits,
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        let manifest = saved_manifest(&error.directory.unwrap());
        assert_eq!(manifest.state, ConversionState::Incomplete);
        assert!(manifest.files.is_empty());
        let expected = encode_segment(
            &decoded.telemetry.systems[0].segments[0],
            &decoded.session,
            1_000_000,
        )
        .unwrap()
        .len();
        let limits = ConversionLimits {
            max_total_output_bytes: (expected * 2 - 1) as u64,
            ..ConversionLimits::default()
        };
        let error = persist_decoded(
            &decoded,
            source(),
            &base.0,
            &limits,
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        let manifest = saved_manifest(&error.directory.unwrap());
        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.files[0].complete);
        assert!(manifest.total_bytes <= limits.max_total_output_bytes);
        assert_eq!(manifest.state, ConversionState::Incomplete);
    }

    #[test]
    fn plan_caps_empty_data_duplicate_identifiers_and_precancel_fail_before_creation() {
        let base = TempDirectory::new();
        let limits = ConversionLimits {
            max_files: 1,
            ..ConversionLimits::default()
        };
        let error = persist_decoded(
            &decoded(),
            source(),
            &base.0,
            &limits,
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(error.directory.is_none());
        let mut empty = decoded();
        empty.telemetry.systems.clear();
        assert!(
            persist_decoded(
                &empty,
                source(),
                &base.0,
                &ConversionLimits::default(),
                CancellationToken::default(),
                |_| {}
            )
            .unwrap_err()
            .directory
            .is_none()
        );
        let mut duplicate = decoded();
        duplicate.telemetry.systems[0].segments[1].index = 0;
        assert!(
            persist_decoded(
                &duplicate,
                source(),
                &base.0,
                &ConversionLimits::default(),
                CancellationToken::default(),
                |_| {}
            )
            .unwrap_err()
            .directory
            .is_none()
        );
        duplicate.telemetry.systems[0].segments[1].index = usize::MAX;
        assert!(
            persist_decoded(
                &duplicate,
                source(),
                &base.0,
                &ConversionLimits::default(),
                CancellationToken::default(),
                |_| {}
            )
            .unwrap_err()
            .message
            .contains("overflow")
        );
        let token = CancellationToken::default();
        token.cancel();
        assert!(
            persist_decoded(
                &decoded(),
                source(),
                &base.0,
                &ConversionLimits::default(),
                token,
                |_| {}
            )
            .unwrap_err()
            .directory
            .is_none()
        );
        assert_eq!(fs::read_dir(&base.0).unwrap().count(), 0);
    }

    #[test]
    fn bounded_input_reader_and_invalid_image_do_not_create_output() {
        let base = TempDirectory::new();
        let input = base.0.join("source.m1dli");
        fs::write(&input, b"invalid image").unwrap();
        let token = CancellationToken::default();
        assert!(read_bounded(&input, 12, &token).is_err());
        assert_eq!(read_bounded(&input, 13, &token).unwrap(), b"invalid image");
        let error =
            convert_image_file(&input, &base.0, &ConversionLimits::default(), token, |_| {})
                .unwrap_err();
        assert!(error.directory.is_none());
        assert_eq!(fs::read_dir(&base.0).unwrap().count(), 1);
        assert_eq!(fs::read(input).unwrap(), b"invalid image");
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_fifo_input_is_rejected_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let base = TempDirectory::new();
        let path = base.0.join("input.m1dli");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a live NUL-terminated pathname in our fresh test
        // directory; mkfifo creates a FIFO and does not take ownership of it.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let error = read_bounded(&path, 1024, &CancellationToken::default()).unwrap_err();
        assert!(error.contains("regular file"));
    }

    #[test]
    fn create_new_manifest_replacement_preserves_existing_staged_file() {
        let base = TempDirectory::new();
        let result = persist_decoded(
            &decoded(),
            source(),
            &base.0,
            &ConversionLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        let current = fs::read(result.directory.join("manifest.json")).unwrap();
        fs::write(
            result.directory.join("manifest.next.json"),
            b"retained staged state",
        )
        .unwrap();
        assert!(replace_manifest(&result.directory, &result.manifest).is_err());
        assert_eq!(
            fs::read(result.directory.join("manifest.json")).unwrap(),
            current
        );
        assert_eq!(
            fs::read(result.directory.join("manifest.next.json")).unwrap(),
            b"retained staged state"
        );
        let mut oversized = result.manifest;
        oversized.source.path = "x".repeat(MAX_MANIFEST_BYTES + 1);
        let temporary = result.directory.join("oversized.json");
        assert!(write_manifest(&temporary, &oversized).is_err());
        assert!(!temporary.exists());
    }

    #[test]
    fn local_numeric_image_conversion_if_requested() {
        let Some(path) = std::env::var_os("I3RS_M1_TELEMETRY_FIXTURE") else {
            return;
        };
        let base = TempDirectory::new();
        let result = convert_image_file(
            Path::new(&path),
            &base.0,
            &ConversionLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.manifest.files.len(), 4);
        assert_eq!(
            saved_manifest(&result.directory).state,
            ConversionState::Complete
        );
        for file in result.manifest.files {
            let recording =
                crate::LdFile::from_bytes(fs::read(result.directory.join(file.file_name)).unwrap())
                    .unwrap();
            assert_eq!(recording.channels.len(), 196);
        }
    }

    #[test]
    fn local_raw_capture_conversion_if_requested() {
        let Some(path) = std::env::var_os("I3RS_M1_RAW_CAPTURE_FIXTURE") else {
            return;
        };
        let base = TempDirectory::new();
        let result = convert_capture_directory(
            Path::new(&path),
            &base.0,
            &ConversionLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.manifest.files.len(), 4);
        assert_eq!(result.manifest.source.kind, SourceKind::RawCapture);
        for file in result.manifest.files {
            assert!(file.complete);
            let recording =
                crate::LdFile::from_bytes(fs::read(result.directory.join(file.file_name)).unwrap())
                    .unwrap();
            assert_eq!(recording.channels.len(), 196);
        }
    }
}

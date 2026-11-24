//! Experimental native M1 acquisition and conversion commands.

use std::fs::OpenOptions;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use i3rs_core::m1::capture::{self, CaptureLimits, CaptureProgress, CaptureStage};
use i3rs_core::m1::conversion::{self, ConversionLimits, ConversionProgress, ConversionStage};
use i3rs_core::m1::discovery::{self, DiscoveryReport};
use i3rs_core::m1::log_image::{
    MetadataGroupContents, MoRecord, MoRecordHeader, PropertyFields, RawLogImage,
};
use i3rs_core::m1::telemetry::TelemetryLimits;
use i3rs_core::m1::transport::CancellationToken;
use serde_json::{Value, json};

const PREVIEW_BYTES: usize = 256;
const PROPERTY_LIMIT: usize = 256;
const GROUP_LIMIT: usize = 256;

pub(super) const EXPERIMENTAL_HELP: &str = "Experimental native M1 discovery, read-only acquisition and conversion. Downloads retain an i3rs raw bundle; download also converts supported data to .ld files with an explicit resampling policy. No official M1 image is constructed. Validated on one M150 ECU. Other ECU and firmware combinations remain unvalidated. Output uses i3rs timing and resampling. Interrupted work retains incomplete directories.";

#[derive(Args)]
#[command(about = "Experimental native M1 acquisition and conversion", long_about = EXPERIMENTAL_HELP)]
pub(super) struct M1Args {
    #[command(subcommand)]
    command: M1Command,
}

#[derive(Subcommand)]
enum M1Command {
    /// Browse candidate advertisements; never connect to discovered devices.
    Devices {
        #[arg(long, default_value_t = 3000, value_parser = clap::value_parser!(u64).range(1..=60_000))]
        window_ms: u64,
    },
    /// Read device logging information and active-user download permissions.
    Inspect {
        #[arg(value_parser = parse_endpoint)]
        endpoint: SocketAddr,
        #[arg(long, default_value_t = 3000, value_parser = clap::value_parser!(u64).range(1..=60_000))]
        timeout_ms: u64,
    },
    /// Download an experimental raw bundle, without producing .ld or official M1 images.
    DownloadRaw(CaptureArgs),
    /// Download raw records and convert supported logging data natively to .ld files.
    Download {
        #[command(flatten)]
        capture: CaptureArgs,
        #[command(flatten)]
        conversion: ConversionLimitArgs,
    },
    /// Convert a supported local .m1dli into a fresh child directory containing .ld files.
    ConvertImage {
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[command(flatten)]
        limits: ConversionLimitArgs,
    },
    /// Convert a complete i3rs raw capture directory into native .ld files.
    ConvertCapture {
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        /// Export the saved portion of an interrupted download as partial recordings.
        #[arg(long)]
        recover_partial: bool,
        #[command(flatten)]
        limits: ConversionLimitArgs,
    },
    /// Inspect a local uncompressed .m1dli outer record and bounded metadata; no conversion.
    ImageInfo {
        path: PathBuf,
        #[arg(long, default_value_t = 268_435_456, value_parser = parse_record_limit)]
        max_bytes: usize,
    },
}

#[derive(Args)]
struct CaptureArgs {
    #[arg(value_parser = parse_endpoint)]
    endpoint: SocketAddr,
    /// Logging systems numbered 1 through 8, comma-separated (e.g. 1,2).
    #[arg(long, required = true, value_delimiter = ',', value_parser = parse_system)]
    systems: Vec<u8>,
    /// Existing base directory; a new uniquely named child is created.
    #[arg(long)]
    destination: PathBuf,
    #[arg(long, default_value_t = 2_147_483_648_u64, value_parser = clap::value_parser!(u64).range(12..))]
    max_total_bytes: u64,
    #[arg(long, default_value_t = 16_777_216, value_parser = parse_record_limit)]
    max_sector_bytes: usize,
    #[arg(long, default_value_t = 10_000, value_parser = clap::value_parser!(u64).range(1..))]
    max_sectors: u64,
    /// Total automatic reconnect attempts after a transient download failure (0 disables recovery and Fast Upload).
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=8))]
    reconnect_attempts: u8,
    /// Disable verified MoTeC Fast Upload and use standard XCP reads only.
    #[arg(long)]
    standard_upload: bool,
    #[arg(long, default_value_t = 3000, value_parser = clap::value_parser!(u64).range(1..=60_000))]
    timeout_ms: u64,
}

#[derive(Args)]
struct ConversionLimitArgs {
    /// Maximum source image or combined captured setup, metadata and sector bytes.
    #[arg(long, default_value_t=536_870_912, value_parser=parse_record_limit)]
    max_input_bytes: usize,
    #[arg(long, default_value_t=67_108_864, value_parser=parse_record_limit)]
    max_decoded_bytes: usize,
    #[arg(long, default_value_t=268_435_456, value_parser=parse_ld_limit)]
    max_output_file_bytes: usize,
    #[arg(long, default_value_t=2_147_483_648_u64, value_parser=clap::value_parser!(u64).range(1762..))]
    max_total_output_bytes: u64,
    #[arg(long, default_value_t=1024, value_parser=clap::value_parser!(u32).range(1..=10_000))]
    max_output_files: u32,
    /// Maximum number of source records, including telemetry packets.
    #[arg(long, default_value_t = TelemetryLimits::default().max_records, value_parser = parse_positive_usize)]
    max_records: usize,
    #[arg(long, default_value_t = TelemetryLimits::default().max_samples, value_parser = parse_positive_usize)]
    max_samples: usize,
    /// Maximum number of channel series across all recording segments.
    #[arg(long, default_value_t = TelemetryLimits::default().max_channels, value_parser = parse_positive_usize)]
    max_channels: usize,
}
impl ConversionLimitArgs {
    fn into_limits(self) -> ConversionLimits {
        let mut limits = ConversionLimits::default();
        limits.decode.max_image_bytes = self.max_input_bytes;
        limits.decode.max_decoded_bytes = self.max_decoded_bytes;
        limits.decode.telemetry.max_samples = self.max_samples;
        limits.decode.telemetry.max_channels = self.max_channels;
        limits.decode.telemetry.max_records = self.max_records;
        limits.max_file_bytes = self.max_output_file_bytes;
        limits.max_total_output_bytes = self.max_total_output_bytes;
        limits.max_files = self.max_output_files as usize;
        limits
    }
}

fn parse_system(value: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .ok()
        .filter(|number| (1..=8).contains(number))
        .ok_or_else(|| "logging systems are numbered 1 through 8".into())
}
fn parse_positive_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| *count >= 1)
        .ok_or_else(|| "limit must be at least 1 and fit this platform".into())
}
fn parse_record_limit(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|bytes| *bytes >= 12)
        .ok_or_else(|| "byte limit must be at least 12 and fit this platform".into())
}
fn parse_ld_limit(value: &str) -> Result<usize, String> {
    let bytes = parse_record_limit(value)?;
    if bytes < 1762 {
        return Err("LD output byte limit must allow its 1762-byte header".into());
    }
    Ok(bytes)
}
fn parse_endpoint(value: &str) -> Result<SocketAddr, String> {
    let endpoint: SocketAddr = value.parse().map_err(|_| "expected an IP endpoint such as 192.0.2.10:5555 or [fe80::1%4]:5555; IPv6 scope must be numeric".to_string())?;
    if endpoint.port() == 0 {
        return Err("endpoint port must be greater than zero".into());
    }
    let unicast = |ip: IpAddr| match ip {
        IpAddr::V4(ip) => !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast(),
        IpAddr::V6(ip) => !ip.is_unspecified() && !ip.is_multicast(),
    };
    if !unicast(endpoint.ip()) {
        return Err("endpoint must be a unicast IP address".into());
    }
    if let SocketAddr::V6(endpoint) = endpoint {
        if endpoint.ip().is_unicast_link_local() && endpoint.scope_id() == 0 {
            return Err("IPv6 link-local endpoints require a numeric interface scope, e.g. [fe80::1%4]:5555".into());
        }
        if endpoint
            .ip()
            .to_ipv4_mapped()
            .is_some_and(|ip| !unicast(IpAddr::V4(ip)))
        {
            return Err("endpoint must be a unicast IP address".into());
        }
    }
    Ok(endpoint)
}

pub(super) fn run(args: M1Args) -> Result<(), String> {
    match args.command {
        M1Command::Devices { window_ms } => {
            let sigint = SigintCancellation::install();
            let report = discovery::discover(Duration::from_millis(window_ms), sigint.token())
                .map_err(|error| error.to_string())?;
            super::print_json(&discovery_json(report))
        }
        M1Command::Inspect {
            endpoint,
            timeout_ms,
        } => {
            let sigint = SigintCancellation::install();
            let inspection = capture::inspect_device(
                endpoint,
                Duration::from_millis(timeout_ms),
                sigint.token().clone(),
                &CaptureLimits::default(),
            )
            .map_err(|error| error.to_string())?;
            let mut value = serde_json::to_value(inspection).map_err(|error| error.to_string())?;
            value["schema_version"] = json!("i3rs-cli.m1.inspect.v1");
            value["system_numbering"] = json!(
                "system_number is 1..8; id and download_allowed array indices are wire IDs 0..7"
            );
            if let Some(systems) = value["systems"].as_array_mut() {
                for system in systems {
                    if let Some(id) = system["id"].as_u64() {
                        system["system_number"] = json!(id + 1);
                    }
                }
            }
            super::print_json(&value)
        }
        M1Command::DownloadRaw(args) => {
            let sigint = SigintCancellation::install();
            let result = acquire(args, sigint.token().clone())?;
            super::print_json(&json!({
                "schema_version": "i3rs-cli.m1.raw-capture.v1",
                "directory": result.directory,
                "manifest": result.manifest,
                "system_numbering": "CLI --systems uses 1..8; manifest system IDs use 0..7",
            }))
        }
        M1Command::Download {
            capture,
            conversion,
        } => {
            let sigint = SigintCancellation::install();
            let destination = capture.destination.clone();
            let conversion_limits = conversion.into_limits();
            let raw = acquire(capture, sigint.token().clone())?;
            let mut progress = ConversionPrinter::default();
            let converted = conversion::convert_capture_directory(
                &raw.directory,
                &destination,
                &conversion_limits,
                sigint.token().clone(),
                |update| progress.print(update),
            )
            .map_err(|error| {
                format!(
                    "{error}; raw capture retained at {}",
                    raw.directory.display()
                )
            })?;
            super::print_json(&json!({
                "schema_version":"i3rs-cli.m1.download.v1",
                "raw_directory":raw.directory,
                "raw_manifest":raw.manifest,
                "directory":converted.directory,
                "manifest":converted.manifest,
                "system_numbering":"CLI --systems uses 1..8; manifest system IDs use 0..7",
            }))
        }
        M1Command::ConvertImage {
            source,
            destination,
            limits,
        } => {
            let sigint = SigintCancellation::install();
            let mut progress = ConversionPrinter::default();
            let result = conversion::convert_image_file(
                &source,
                &destination,
                &limits.into_limits(),
                sigint.token().clone(),
                |update| progress.print(update),
            )
            .map_err(|e| e.to_string())?;
            super::print_json(&conversion_json(result))
        }
        M1Command::ConvertCapture {
            source,
            destination,
            limits,
            recover_partial,
        } => {
            let sigint = SigintCancellation::install();
            let mut progress = ConversionPrinter::default();
            let convert = if recover_partial {
                conversion::recover_capture_directory
            } else {
                conversion::convert_capture_directory
            };
            let result = convert(
                &source,
                &destination,
                &limits.into_limits(),
                sigint.token().clone(),
                |update| progress.print(update),
            )
            .map_err(|e| e.to_string())?;
            super::print_json(&conversion_json(result))
        }
        M1Command::ImageInfo { path, max_bytes } => {
            super::print_json(&image_info(path, max_bytes)?)
        }
    }
}

fn acquire(
    mut args: CaptureArgs,
    cancellation: CancellationToken,
) -> Result<capture::CaptureResult, String> {
    args.systems.sort_unstable();
    if args.systems.windows(2).any(|ids| ids[0] == ids[1]) {
        return Err("logging systems must be distinct".into());
    }
    let systems = args.systems.into_iter().map(|number| number - 1).collect();
    let limits = CaptureLimits {
        max_total_bytes: args.max_total_bytes,
        max_sector_bytes: args.max_sector_bytes,
        max_sectors: args.max_sectors,
        max_reconnects: args.reconnect_attempts,
        fast_upload: !args.standard_upload,
        ..CaptureLimits::default()
    };
    let mut last_progress: Option<(CaptureStage, Instant)> = None;
    capture::download_raw(
        args.endpoint,
        systems,
        &args.destination,
        &limits,
        Duration::from_millis(args.timeout_ms),
        cancellation,
        |progress| {
            let now = Instant::now();
            if last_progress.is_none_or(|(stage, time)| {
                stage != progress.stage || now.duration_since(time) >= Duration::from_secs(1)
            }) {
                print_progress(&progress);
                last_progress = Some((progress.stage, now));
            }
        },
    )
    .map_err(|e| e.to_string())
}
fn conversion_json(result: conversion::ConversionResult) -> Value {
    json!({"schema_version":"i3rs-cli.m1.conversion.v1","directory":result.directory,"manifest":result.manifest,"system_numbering":"manifest system IDs use 0..7"})
}
#[derive(Default)]
struct ConversionPrinter {
    last: Option<(ConversionStage, Instant)>,
}
impl ConversionPrinter {
    fn print(&mut self, progress: ConversionProgress) {
        let now = Instant::now();
        if self.last.is_some_and(|(stage, time)| {
            stage == progress.stage && now.duration_since(time) < Duration::from_secs(1)
        }) {
            return;
        }
        self.last = Some((progress.stage, now));
        let label = match progress.stage {
            ConversionStage::ReadingInput => "Reading conversion source",
            ConversionStage::Decoding => "Decoding M1 data",
            ConversionStage::PreparingOutput => "Preparing LD recordings",
            ConversionStage::WritingOutput => "Writing LD recording",
            ConversionStage::Finished => "Conversion complete",
        };
        let system = progress
            .system
            .map(|id| format!("; system {}", id + 1))
            .unwrap_or_default();
        let segment = progress
            .segment
            .map(|id| format!("; segment {}", id + 1))
            .unwrap_or_default();
        eprintln!(
            "{label}{system}{segment}; files {}/{}; {} bytes",
            progress.files_complete, progress.files_total, progress.bytes_written
        );
    }
}

fn print_progress(progress: &CaptureProgress) {
    let label = match progress.stage {
        CaptureStage::Inspecting => "Inspecting device permissions and logging info",
        CaptureStage::ReadingSetups => "Reading logging setups",
        CaptureStage::ReadingMetadata => "Reading logging metadata",
        CaptureStage::ReadingSector => "Reading logging sector",
        CaptureStage::Reconnecting => "Reconnecting and verifying saved data",
        CaptureStage::CheckingConsistency => "Checking logging boundaries",
        CaptureStage::Finished => "Raw capture complete",
    };
    let system = progress
        .system
        .map(|id| format!("; system {}", id + 1))
        .unwrap_or_default();
    let sector = progress
        .sector
        .map(|sector| format!("; sector {sector}"))
        .unwrap_or_default();
    eprintln!(
        "{label}{system}{sector}; sectors {}/{}; {} bytes",
        progress.sectors_complete, progress.sectors_total, progress.bytes
    );
}

fn endpoint_json(endpoint: SocketAddr) -> Value {
    json!({
        "address": endpoint.to_string(),
        "ipv6_scope_id": match endpoint { SocketAddr::V6(ip) => Some(ip.scope_id()), SocketAddr::V4(_) => None },
    })
}
fn discovery_json(report: DiscoveryReport) -> Value {
    json!({
        "schema_version": "i3rs-cli.m1.devices.v1",
        "candidate_service_types": discovery::CANDIDATE_SERVICE_TYPES,
        "candidate_only": true,
        "connected": false,
        "warnings": report.warnings,
        "devices": report.devices.into_iter().map(|device| json!({
            "service_type": device.service_type,
            "fullname": device.fullname,
            "hostname": device.hostname,
            "endpoints": device.endpoints.into_iter().map(|endpoint| {
                let mut value = endpoint_json(endpoint.address);
                value["interfaces"] = json!(endpoint.interfaces.into_iter().map(|interface| json!({"name": interface.name, "index": interface.index})).collect::<Vec<_>>());
                value
            }).collect::<Vec<_>>(),
            "unusable_endpoints": device.unusable_endpoints.into_iter().map(endpoint_json).collect::<Vec<_>>(),
            "txt": device.txt.into_iter().map(|property| json!({
                "key": property.key,
                "flag": property.value.is_none(),
                "value_hex": property.value.as_ref().map(|bytes| hex(bytes)),
                "value_utf8": property.value.as_ref().and_then(|bytes| std::str::from_utf8(bytes).ok()),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(&mut text, "{byte:02x}");
    }
    text
}
fn preview(bytes: &[u8]) -> Value {
    let prefix = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    json!({ "hex": hex(prefix), "utf8": std::str::from_utf8(prefix).ok(), "total_bytes": bytes.len(), "truncated": prefix.len() < bytes.len() })
}
fn header_json(header: MoRecordHeader) -> Value {
    json!({
        "control_byte_2": header.flags,
        "encoding": header.encoding,
        "record_type": header.record_type,
        "record_type_hex": format!("0x{:08x}", header.record_type),
        "payload_bytes": header.payload_len,
    })
}
fn record_json(record: &MoRecord<'_>) -> Value {
    json!({ "offset": record.offset, "header": header_json(record.header), "opaque": record.header.encoding != 0 })
}
fn image_info(path: PathBuf, max_bytes: usize) -> Result<Value, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Reject FIFOs as non-regular files without blocking inside open().
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("image-info requires a regular local file".into());
    }
    if metadata.len() > max_bytes as u64 {
        return Err(format!(
            "image is {} bytes, exceeding --max-bytes {max_bytes}",
            metadata.len()
        ));
    }
    let read_limit = u64::try_from(max_bytes)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .ok_or_else(|| "--max-bytes is too large".to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(
            usize::try_from(metadata.len())
                .map_err(|_| "file length does not fit this platform")?,
        )
        .map_err(|_| "could not allocate bounded image buffer")?;
    file.by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "image grew beyond --max-bytes {max_bytes} while reading"
        ));
    }
    let image = RawLogImage::parse(&bytes).map_err(|error| error.to_string())?;
    let inspection = image
        .inspect_metadata()
        .map_err(|error| error.to_string())?;
    let mut properties_left = PROPERTY_LIMIT;
    let groups = inspection
        .groups
        .iter()
        .take(GROUP_LIMIT)
        .map(|group| {
            let mut value = record_json(&group.record);
            match &group.contents {
                MetadataGroupContents::Opaque => {
                    value["opaque"] = json!(true);
                }
                MetadataGroupContents::Plain {
                    properties,
                    other_records,
                } => {
                    let shown = properties.len().min(properties_left);
                    properties_left -= shown;
                    value["properties"] = json!(
                        properties
                            .iter()
                            .take(shown)
                            .map(|property| {
                                let mut value = record_json(&property.record);
                                value["name"] = property.name().map(preview).unwrap_or(Value::Null);
                                value["value"] = property
                                    .value()
                                    .map(|field| {
                                        let mut value = record_json(field);
                                        value["payload_preview"] = preview(field.payload);
                                        value
                                    })
                                    .unwrap_or(Value::Null);
                                value["field_count"] = match &property.fields {
                                    PropertyFields::Opaque => Value::Null,
                                    PropertyFields::Plain(fields) => json!(fields.len()),
                                };
                                value
                            })
                            .collect::<Vec<_>>()
                    );
                    value["properties_omitted"] = json!(properties.len() - shown);
                    value["other_records"] = json!(
                        other_records
                            .iter()
                            .take(PROPERTY_LIMIT)
                            .map(record_json)
                            .collect::<Vec<_>>()
                    );
                    value["other_records_omitted"] =
                        json!(other_records.len().saturating_sub(PROPERTY_LIMIT));
                }
            }
            value
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema_version": "i3rs-cli.m1.image-info.v1",
        "file": path,
        "file_bytes": bytes.len(),
        "header": header_json(image.header),
        "converter_validated": false,
        "produces_ld": false,
        "checksums_validated": false,
        "opaque_root": inspection.opaque_root,
        "opaque_payload_preview": inspection.opaque_root.then(|| preview(image.payload)),
        "records_examined": inspection.records_examined,
        "groups": groups,
        "groups_omitted": inspection.groups.len().saturating_sub(GROUP_LIMIT),
        "rendered_properties": PROPERTY_LIMIT - properties_left,
        "property_preview_limit": PREVIEW_BYTES,
        "property_limit": PROPERTY_LIMIT,
        "other_root_records": inspection.other_root_records.iter().take(PROPERTY_LIMIT).map(record_json).collect::<Vec<_>>(),
        "other_root_records_omitted": inspection.other_root_records.len().saturating_sub(PROPERTY_LIMIT),
    }))
}

#[cfg(unix)]
mod sigint {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use i3rs_core::m1::transport::CancellationToken;

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_sigint(_: libc::c_int) {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            // Second Ctrl-C restores default disposition and terminates promptly.
            unsafe {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::raise(libc::SIGINT);
            }
        }
    }

    pub(super) struct SigintCancellation {
        token: CancellationToken,
        done: Arc<AtomicBool>,
        watcher: Option<thread::JoinHandle<()>>,
        previous: libc::sighandler_t,
    }

    impl SigintCancellation {
        pub(super) fn install() -> Self {
            INTERRUPTED.store(false, Ordering::SeqCst);
            let previous =
                unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
            let token = CancellationToken::default();
            let done = Arc::new(AtomicBool::new(false));
            let watch_token = token.clone();
            let watch_done = done.clone();
            let watcher = thread::Builder::new()
                .name("i3rs-sigint".into())
                .spawn(move || {
                    while !watch_done.load(Ordering::Acquire) {
                        if INTERRUPTED.load(Ordering::Acquire) {
                            watch_token.cancel();
                            break;
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                })
                .ok();
            Self {
                token,
                done,
                watcher,
                previous,
            }
        }

        pub(super) fn token(&self) -> &CancellationToken {
            &self.token
        }
    }

    impl Drop for SigintCancellation {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Release);
            unsafe {
                libc::signal(libc::SIGINT, self.previous);
            }
            if let Some(watcher) = self.watcher.take() {
                let _ = watcher.join();
            }
        }
    }
}

#[cfg(not(unix))]
mod sigint {
    use i3rs_core::m1::transport::CancellationToken;

    pub(super) struct SigintCancellation(CancellationToken);

    impl SigintCancellation {
        pub(super) fn install() -> Self {
            Self(CancellationToken::default())
        }

        pub(super) fn token(&self) -> &CancellationToken {
            &self.0
        }
    }
}

use sigint::SigintCancellation;

#[cfg(test)]
mod tests {
    use super::*;
    use i3rs_core::m1::discovery::{CandidateDevice, Endpoint, Interface, TxtProperty};

    #[test]
    fn scoped_unicast_endpoints_preserve_numeric_ipv6_interface() {
        let endpoint = parse_endpoint("[fe80::1%4]:5555").unwrap();
        assert_eq!(endpoint_json(endpoint)["ipv6_scope_id"], 4);
        assert!(parse_endpoint("[::1]:5555").is_ok());
        assert!(parse_endpoint("127.0.0.1:5555").is_ok());
        assert!(parse_endpoint("[fe80::1%en0]:5555").is_err());
    }

    #[test]
    fn discovery_json_preserves_txt_flags_empty_bytes_and_interfaces() {
        let scoped = "[fe80::1%7]:5555".parse().unwrap();
        let json = discovery_json(DiscoveryReport {
            devices: vec![CandidateDevice {
                service_type: discovery::CANDIDATE_SERVICE_TYPES[0].into(),
                fullname: "candidate.local.".into(),
                hostname: "ecu.local.".into(),
                endpoints: vec![Endpoint {
                    address: scoped,
                    interfaces: vec![Interface {
                        name: "en7".into(),
                        index: 7,
                    }],
                }],
                unusable_endpoints: vec!["[fe80::2]:5555".parse().unwrap()],
                txt: vec![
                    TxtProperty {
                        key: "flag".into(),
                        value: None,
                    },
                    TxtProperty {
                        key: "empty".into(),
                        value: Some(vec![]),
                    },
                    TxtProperty {
                        key: "raw".into(),
                        value: Some(vec![0xff, 0]),
                    },
                ],
            }],
            warnings: vec![],
        });
        let device = &json["devices"][0];
        assert_eq!(device["endpoints"][0]["address"], "[fe80::1%7]:5555");
        assert_eq!(device["endpoints"][0]["interfaces"][0]["index"], 7);
        assert_eq!(device["txt"][0]["flag"], true);
        assert!(device["txt"][0]["value_hex"].is_null());
        assert_eq!(device["txt"][1]["flag"], false);
        assert_eq!(device["txt"][1]["value_hex"], "");
        assert_eq!(device["txt"][2]["value_hex"], "ff00");
        assert!(device["txt"][2]["value_utf8"].is_null());
    }
}

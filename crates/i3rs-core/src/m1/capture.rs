//! Read-only acquisition into an i3rs raw investigation bundle.
//!
//! Files contain exact received Mo records. This format is deliberately distinct
//! from MoTeC `.m1dli` and `.ld`: packet decoding, signatures/checksums, package
//! reconstruction and conversion have not been validated. No erase, programming
//! or authentication command is available through this module.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::log_image::MO_HEADER_SIZE;
use super::log_info::{LogInfo, LogInfoError, SectorPlan};
use super::transport::{CancellationToken, XcpClient, XcpError};

mod recovery;
#[cfg(test)]
#[path = "capture/recovery_tests.rs"]
mod recovery_tests;
use recovery::{RecoveringReader, RecoverySettings};

pub const RAW_CAPTURE_FORMAT: &str = "i3rs.m1.raw-capture.v1";
const LOG_INFO_SELECTOR: u32 = 0x0001_0062;
const PERMISSIONS_SELECTOR: u32 = 0x0001_0040;
static DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct CaptureLimits {
    pub max_info_bytes: usize,
    pub max_config_bytes: usize,
    pub max_sector_bytes: usize,
    pub max_total_bytes: u64,
    pub max_sectors: u64,
    pub max_records: usize,
    /// Global reconnect budget for a capture; zero disables automatic recovery.
    pub max_reconnects: u8,
    /// Verify MoTeC Fast Upload against a standard configuration read before use.
    pub fast_upload: bool,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            max_info_bytes: 4 * 1024 * 1024,
            max_config_bytes: 64 * 1024 * 1024,
            max_sector_bytes: 16 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_sectors: 10_000,
            max_records: 4096,
            max_reconnects: 3,
            fast_upload: true,
        }
    }
}

impl CaptureLimits {
    fn validate(&self) -> Result<(), CaptureErrorKind> {
        if self.max_info_bytes < MO_HEADER_SIZE
            || self.max_config_bytes < MO_HEADER_SIZE
            || self.max_sector_bytes < MO_HEADER_SIZE
            || self.max_total_bytes < MO_HEADER_SIZE as u64
            || self.max_records == 0
            || self.max_reconnects > 8
        {
            return Err(CaptureErrorKind::InvalidInput(
                "record limits must allow a 12-byte header and a nonzero record budget; max reconnects is 8".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMapping {
    pub last_sector: Option<u32>,
    pub address_numerator: Option<u32>,
    pub address_denominator: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSummary {
    pub id: u8,
    pub boundary_a: u32,
    pub boundary_b: u32,
    pub sector_count: Option<u64>,
    pub plan_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInspection {
    pub endpoint: String,
    pub permissions_available: bool,
    pub download_allowed: [bool; 8],
    pub device: Option<DeviceMapping>,
    pub systems: Vec<SystemSummary>,
    pub disconnect_diagnostic: Option<String>,
}

/// Inspect metadata and active-user permissions without reading logging sectors.
pub fn inspect_device(
    endpoint: SocketAddr,
    timeout: Duration,
    cancellation: CancellationToken,
    limits: &CaptureLimits,
) -> Result<DeviceInspection, CaptureError> {
    let result = (|| {
        limits.validate()?;
        let mut budget = TransferBudget::new(limits.max_total_bytes);
        let mut client = XcpClient::connect(endpoint, timeout, cancellation)?;
        let prepared = read_inspection(&mut client, endpoint, limits, &mut budget)?;
        let mut summary = prepared.summary;
        if let Err(error) = client.disconnect() {
            summary.disconnect_diagnostic = Some(error.to_string());
        }
        Ok(summary)
    })();
    result.map_err(|kind| CaptureError {
        kind,
        bundle_directory: None,
        manifest_diagnostic: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Incomplete,
    Inconsistent,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedFile {
    pub file_name: String,
    pub expected_bytes: u64,
    pub bytes: u64,
    pub complete: bool,
    pub system: Option<u8>,
    pub sector: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureManifest {
    pub format: String,
    pub state: CaptureState,
    pub endpoint: String,
    pub started_unix_millis: u64,
    pub selected_systems: Vec<u8>,
    pub initial: DeviceInspection,
    pub files: Vec<CapturedFile>,
    pub total_bytes: u64,
    /// True means selected boundaries were equal before and after acquisition;
    /// it does not establish an atomic or checksum-validated ECU snapshot.
    pub boundaries_unchanged: Option<bool>,
    pub converter_validated: bool,
    pub produces_ld: bool,
    pub error: Option<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub directory: PathBuf,
    pub manifest: CaptureManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStage {
    Inspecting,
    ReadingSetups,
    ReadingMetadata,
    ReadingSector,
    CheckingConsistency,
    Reconnecting,
    Finished,
}

#[derive(Debug, Clone)]
pub struct CaptureProgress {
    pub stage: CaptureStage,
    pub system: Option<u8>,
    pub sector: Option<u32>,
    pub sectors_complete: u64,
    pub sectors_total: u64,
    pub bytes: u64,
}

/// Download selected logging systems into a fresh child of `destination_base`.
///
/// Permission and plan checks precede directory creation and all sector reads.
/// After creation, failure/cancellation retains an incomplete manifest and any
/// received files. All filenames are generated locally from validated integers.
/// Byte limits include initial/fallback/final info, configuration, recovery
/// verification and requested UPLOAD data whose reply may have been lost.
/// After initial inspection succeeds, transient reads can reconnect up to the
/// global max_reconnects limit. Each reconnect rechecks permissions, exact info
/// and every committed record before restarting the interrupted record.
/// A disconnect failure after acquisition is a diagnostic, not lost data.
pub fn download_raw(
    endpoint: SocketAddr,
    mut systems: Vec<u8>,
    destination_base: &Path,
    limits: &CaptureLimits,
    timeout: Duration,
    cancellation: CancellationToken,
    mut progress: impl FnMut(CaptureProgress),
) -> Result<CaptureResult, CaptureError> {
    let started = Instant::now();
    let mut directory = None;
    let mut manifest: Option<CaptureManifest> = None;
    let result = (|| -> Result<CaptureResult, CaptureErrorKind> {
        limits.validate()?;
        validate_systems(&mut systems)?;
        if !fs::metadata(destination_base)?.is_dir() {
            return Err(CaptureErrorKind::InvalidInput(
                "capture destination is not a directory".into(),
            ));
        }
        check_cancelled(&cancellation)?;
        progress(CaptureProgress {
            stage: CaptureStage::Inspecting,
            system: None,
            sector: None,
            sectors_complete: 0,
            sectors_total: 0,
            bytes: 0,
        });
        let mut budget = TransferBudget::new(limits.max_total_bytes);
        let mut client = XcpClient::connect(endpoint, timeout, cancellation.clone())?;
        let prepared = read_inspection(&mut client, endpoint, limits, &mut budget)?;
        let initial_info = LogInfo::parse_with_limit(&prepared.info_bytes, limits.max_records)?;
        let plans = selected_plans(
            &initial_info,
            &prepared.summary,
            &systems,
            limits.max_sectors,
        )?;
        let total_sectors = plans.iter().map(SectorPlan::len).sum();
        check_cancelled(&cancellation)?;
        let child = create_capture_directory(destination_base)?;
        directory = Some(child.clone());
        manifest = Some(CaptureManifest {
            format: RAW_CAPTURE_FORMAT.into(), state: CaptureState::Incomplete,
            endpoint: endpoint.to_string(), started_unix_millis: unix_millis(),
            selected_systems: systems.clone(), initial: prepared.summary.clone(),
            files: Vec::new(), total_bytes: 0, boundaries_unchanged: None,
            converter_validated: false, produces_ld: false, error: None,
            diagnostics: vec!["Raw Mo records; conversion, package completeness, signatures and checksums are unvalidated. Boundary comparison does not establish an atomic snapshot.".into()],
        });
        let current = manifest.as_mut().unwrap();
        let parameters = client.parameters();
        current.diagnostics.push(format!(
            "XCP negotiated max CTO {}, max DTO {}, address granularity {} bytes, communication flags 0x{:02x}.",
            parameters.max_cto, parameters.max_dto, parameters.address_granularity.bytes(),
            parameters.communication_mode
        ));
        create_manifest(&child, current)?;
        save_record(
            &child,
            current,
            "logging-info-before.mo",
            &prepared.info_bytes,
            None,
            None,
        )?;
        if let Some(bytes) = &prepared.permissions_bytes {
            save_record(
                &child,
                current,
                "active-user-permissions.mo",
                bytes,
                None,
                None,
            )?;
        }
        let mut reader = RecoveringReader::new(
            client,
            budget,
            prepared,
            limits,
            RecoverySettings {
                endpoint,
                timeout,
                cancellation: cancellation.clone(),
                systems: systems.clone(),
            },
        );
        let mut completed = 0;
        for (stage, extension, name) in [
            (CaptureStage::ReadingSetups, 0x27, "logging-setups.mo"),
            (CaptureStage::ReadingMetadata, 0x2a, "logging-metadata.mo"),
        ] {
            let position = CaptureProgress {
                stage,
                system: None,
                sector: None,
                sectors_complete: completed,
                sectors_total: total_sectors,
                bytes: reader.budget.used,
            };
            progress(position.clone());
            check_cancelled(&cancellation)?;
            let bytes = reader.read_record(
                extension,
                0,
                limits.max_config_bytes,
                &position,
                &mut current.diagnostics,
                &mut progress,
            )?;
            save_record(&child, current, name, &bytes, None, None)?;
            reader.commit(extension, 0, name, &bytes);
            reader.verify_fast_upload(
                (extension, 0),
                &bytes,
                &position,
                &mut current.diagnostics,
                &mut progress,
            )?;
        }
        for plan in &plans {
            for sector in plan.sectors() {
                let position = CaptureProgress {
                    stage: CaptureStage::ReadingSector,
                    system: Some(plan.system()),
                    sector: Some(sector),
                    sectors_complete: completed,
                    sectors_total: total_sectors,
                    bytes: reader.budget.used,
                };
                progress(position.clone());
                check_cancelled(&cancellation)?;
                let address = plan.address(sector)?;
                let bytes = reader.read_record(
                    address.extension,
                    address.address,
                    limits.max_sector_bytes,
                    &position,
                    &mut current.diagnostics,
                    &mut progress,
                ).inspect_err(|error| {
                    current.diagnostics.push(format!(
                        "Reading logging system {}, sector {sector} (extension 0x{:02x}, selector 0x{:08x}) stopped after {completed}/{total_sectors} sectors: {error}",
                        plan.system() + 1, address.extension, address.address,
                    ));
                })?;
                let name = format!("system-{:02}-sector-{sector:010}.mo", plan.system());
                save_record(
                    &child,
                    current,
                    &name,
                    &bytes,
                    Some(plan.system()),
                    Some(sector),
                )?;
                reader.commit(address.extension, address.address, &name, &bytes);
                completed += 1;
                reader.verify_fast_upload(
                    (address.extension, address.address),
                    &bytes,
                    &CaptureProgress {
                        sectors_complete: completed,
                        ..position
                    },
                    &mut current.diagnostics,
                    &mut progress,
                )?;
            }
        }
        let position = CaptureProgress {
            stage: CaptureStage::CheckingConsistency,
            system: None,
            sector: None,
            sectors_complete: completed,
            sectors_total: total_sectors,
            bytes: reader.budget.used,
        };
        progress(position.clone());
        check_cancelled(&cancellation)?;
        let final_bytes = reader.read_record(
            0x26,
            LOG_INFO_SELECTOR,
            limits.max_info_bytes,
            &position,
            &mut current.diagnostics,
            &mut progress,
        )?;
        save_record(
            &child,
            current,
            "logging-info-after.mo",
            &final_bytes,
            None,
            None,
        )?;
        let final_info = LogInfo::parse_with_limit(&final_bytes, limits.max_records)?;
        let initial_info = LogInfo::parse_with_limit(reader.initial_info(), limits.max_records)?;
        let changed = systems
            .iter()
            .copied()
            .filter(
                |id| match (initial_info.system(*id), final_info.system(*id)) {
                    (Some(before), Some(after)) => {
                        before.boundary_a != after.boundary_a
                            || before.boundary_b != after.boundary_b
                    }
                    _ => true,
                },
            )
            .collect::<Vec<_>>();
        current.boundaries_unchanged = Some(changed.is_empty());
        if !changed.is_empty() {
            current.state = CaptureState::Inconsistent;
            return Err(CaptureErrorKind::BoundariesChanged(changed));
        }
        check_cancelled(&cancellation)?;
        if let Err(error) = reader.client.disconnect() {
            current
                .diagnostics
                .push(format!("Acquisition finished; DISCONNECT failed: {error}"));
        }
        record_transfer_rate(current, started);
        current.state = CaptureState::Complete;
        update_manifest(&child, current)?;
        progress(CaptureProgress {
            stage: CaptureStage::Finished,
            system: None,
            sector: None,
            sectors_complete: completed,
            sectors_total: total_sectors,
            bytes: reader.budget.used,
        });
        Ok(CaptureResult {
            directory: child,
            manifest: current.clone(),
        })
    })();
    match result {
        Ok(result) => Ok(result),
        Err(kind) => {
            let mut manifest_diagnostic = None;
            if let (Some(path), Some(current)) = (&directory, &mut manifest) {
                if matches!(kind, CaptureErrorKind::SourceChanged(_)) {
                    current.state = CaptureState::Inconsistent;
                }
                if current.state == CaptureState::Complete {
                    current.state = CaptureState::Incomplete;
                }
                current.error = Some(kind.to_string());
                if !current
                    .diagnostics
                    .iter()
                    .any(|entry| entry.starts_with("Acquisition wrote "))
                {
                    record_transfer_rate(current, started);
                }
                if let Err(error) = update_manifest(path, current) {
                    manifest_diagnostic = Some(format!("Could not save failure state: {error}"));
                }
            }
            Err(CaptureError {
                kind,
                bundle_directory: directory,
                manifest_diagnostic,
            })
        }
    }
}

fn record_transfer_rate(manifest: &mut CaptureManifest, started: Instant) {
    let seconds = started.elapsed().as_secs_f64();
    let kb_per_second = manifest.total_bytes as f64 / (1000.0 * seconds.max(0.001));
    manifest.diagnostics.push(format!(
        "Acquisition wrote {} bytes in {seconds:.2} s ({kb_per_second:.0} kB/s average).",
        manifest.total_bytes
    ));
}

struct PreparedInspection {
    summary: DeviceInspection,
    info_bytes: Vec<u8>,
    permissions_bytes: Option<Vec<u8>>,
}

fn read_inspection(
    client: &mut XcpClient,
    endpoint: SocketAddr,
    limits: &CaptureLimits,
    budget: &mut TransferBudget,
) -> Result<PreparedInspection, CaptureErrorKind> {
    let info_bytes = budget.read_record(client, 0x26, LOG_INFO_SELECTOR, limits.max_info_bytes)?;
    let info = LogInfo::parse_with_limit(&info_bytes, limits.max_records)?;
    let permissions_bytes = if info.permissions.is_none() {
        Some(budget.read_record(client, 0x26, PERMISSIONS_SELECTOR, limits.max_info_bytes)?)
    } else {
        None
    };
    let fallback = permissions_bytes
        .as_ref()
        .map(|bytes| LogInfo::parse_with_limit(bytes, limits.max_records))
        .transpose()?;
    let permissions = info
        .permissions
        .or_else(|| fallback.as_ref().and_then(|info| info.permissions));
    let mut allowed = [false; 8];
    if let Some(permissions) = permissions {
        for id in 0..8 {
            allowed[usize::from(id)] = permissions.can_download(id);
        }
    }
    let device = info.device.as_ref().map(|device| DeviceMapping {
        last_sector: device.last_sector(),
        address_numerator: device.address_numerator(),
        address_denominator: device.address_denominator(),
    });
    let systems = info
        .systems
        .iter()
        .map(|system| {
            let plan = system.plan(info.device.as_ref(), limits.max_sectors);
            SystemSummary {
                id: system.id,
                boundary_a: system.boundary_a,
                boundary_b: system.boundary_b,
                sector_count: plan.as_ref().ok().map(SectorPlan::len),
                plan_error: plan.err().map(|error| error.to_string()),
            }
        })
        .collect();
    Ok(PreparedInspection {
        summary: DeviceInspection {
            endpoint: endpoint.to_string(),
            permissions_available: permissions.is_some(),
            download_allowed: allowed,
            device,
            systems,
            disconnect_diagnostic: None,
        },
        info_bytes,
        permissions_bytes,
    })
}

fn selected_plans(
    info: &LogInfo<'_>,
    summary: &DeviceInspection,
    systems: &[u8],
    limit: u64,
) -> Result<Vec<SectorPlan>, CaptureErrorKind> {
    let mut remaining = limit;
    let mut plans = Vec::with_capacity(systems.len());
    for id in systems {
        if !summary.download_allowed[usize::from(*id)] {
            return Err(CaptureErrorKind::PermissionDenied(*id));
        }
        let system = info
            .system(*id)
            .ok_or(CaptureErrorKind::MissingSystem(*id))?;
        let plan = system.plan(info.device.as_ref(), remaining)?;
        remaining -= plan.len();
        plans.push(plan);
    }
    Ok(plans)
}

fn validate_systems(systems: &mut [u8]) -> Result<(), CaptureErrorKind> {
    if systems.is_empty() || systems.len() > 8 || systems.iter().any(|id| *id >= 8) {
        return Err(CaptureErrorKind::InvalidInput(
            "select one to eight logging system IDs in 0..8".into(),
        ));
    }
    systems.sort_unstable();
    if systems.windows(2).any(|ids| ids[0] == ids[1]) {
        return Err(CaptureErrorKind::InvalidInput(
            "logging system IDs must be distinct".into(),
        ));
    }
    Ok(())
}

struct TransferBudget {
    limit: u64,
    used: u64,
}
impl TransferBudget {
    fn new(limit: u64) -> Self {
        Self { limit, used: 0 }
    }
    fn read_record(
        &mut self,
        client: &mut XcpClient,
        extension: u8,
        address: u32,
        record_limit: usize,
    ) -> Result<Vec<u8>, CaptureErrorKind> {
        self.read_record_with_upload(client, extension, address, record_limit, false)
    }

    fn read_record_with_upload(
        &mut self,
        client: &mut XcpClient,
        extension: u8,
        address: u32,
        record_limit: usize,
        fast: bool,
    ) -> Result<Vec<u8>, CaptureErrorKind> {
        let remaining = self.limit - self.used;
        let cap = record_limit.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if cap < MO_HEADER_SIZE {
            return Err(CaptureErrorKind::ByteLimit { remaining });
        }
        // XcpClient reads the fixed header, then checks declared total length
        // against cap before payload allocation or transfer.
        let before = client.requested_upload_bytes();
        let result = client.read_record_with_upload(extension, address, cap, fast);
        self.used += client.requested_upload_bytes() - before;
        result.map_err(Into::into)
    }
}

fn check_cancelled(token: &CancellationToken) -> Result<(), CaptureErrorKind> {
    if token.is_cancelled() {
        Err(CaptureErrorKind::Cancelled)
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
fn create_capture_directory(base: &Path) -> Result<PathBuf, CaptureErrorKind> {
    for _ in 0..32 {
        let sequence = DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "i3rs-m1-raw-{}-{}-{sequence}",
            unix_millis(),
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(CaptureErrorKind::InvalidInput(
        "could not create a unique capture directory".into(),
    ))
}
fn create_manifest(directory: &Path, manifest: &CaptureManifest) -> Result<(), CaptureErrorKind> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("manifest.json"))?;
    serde_json::to_writer_pretty(&mut file, manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
fn update_manifest(directory: &Path, manifest: &CaptureManifest) -> Result<(), CaptureErrorKind> {
    // Serialize into a fresh sibling first, preserving the previous readable
    // incomplete manifest if serialization, syncing or renaming fails.
    let next = directory.join("manifest.next.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&next)?;
    let written = (|| -> Result<(), CaptureErrorKind> {
        serde_json::to_writer_pretty(&mut file, manifest)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&next);
        return Err(error);
    }
    let current = directory.join("manifest.json");
    fs::rename(&next, current)?;
    Ok(())
}
fn save_record(
    directory: &Path,
    manifest: &mut CaptureManifest,
    name: &str,
    bytes: &[u8],
    system: Option<u8>,
    sector: Option<u32>,
) -> Result<(), CaptureErrorKind> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))?;
    manifest.files.push(CapturedFile {
        file_name: name.into(),
        expected_bytes: bytes.len() as u64,
        bytes: 0,
        complete: false,
        system,
        sector,
    });
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    let entry = manifest.files.last_mut().unwrap();
    entry.bytes = file.metadata()?.len();
    entry.complete = result.is_ok() && entry.bytes == entry.expected_bytes;
    manifest.total_bytes += entry.bytes;
    result?;
    update_manifest(directory, manifest)
}

#[derive(Debug)]
pub struct CaptureError {
    pub kind: CaptureErrorKind,
    pub bundle_directory: Option<PathBuf>,
    pub manifest_diagnostic: Option<String>,
}
impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)?;
        if let Some(path) = &self.bundle_directory {
            write!(f, "; incomplete raw bundle retained at {}", path.display())?;
        }
        if let Some(diagnostic) = &self.manifest_diagnostic {
            write!(f, "; {diagnostic}")?;
        }
        Ok(())
    }
}
impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}
#[derive(Debug)]
pub enum CaptureErrorKind {
    Transport(XcpError),
    Info(LogInfoError),
    Io(io::Error),
    Json(serde_json::Error),
    InvalidInput(String),
    PermissionDenied(u8),
    MissingSystem(u8),
    ByteLimit { remaining: u64 },
    Cancelled,
    BoundariesChanged(Vec<u8>),
    SourceChanged(String),
    RecoveryExhausted(u8),
}
impl fmt::Display for CaptureErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::Info(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::InvalidInput(message) => f.write_str(message),
            Self::PermissionDenied(id) => write!(
                f,
                "active user does not grant logging download for system {id}"
            ),
            Self::MissingSystem(id) => write!(f, "logging system {id} is absent from device info"),
            Self::ByteLimit { remaining } => write!(
                f,
                "raw capture byte budget has {remaining} bytes left; a Mo header needs 12"
            ),
            Self::Cancelled => f.write_str("raw capture was cancelled"),
            Self::SourceChanged(record) => write!(
                f,
                "source changed during recovery ({record}); capture cannot safely continue"
            ),
            Self::RecoveryExhausted(attempts) => write!(
                f,
                "transient communication failure; reconnect budget exhausted after {attempts} attempts"
            ),
            Self::BoundariesChanged(systems) => write!(
                f,
                "logging boundaries changed during acquisition for systems {systems:?}; raw capture is inconsistent"
            ),
        }
    }
}
impl std::error::Error for CaptureErrorKind {}
impl From<XcpError> for CaptureErrorKind {
    fn from(error: XcpError) -> Self {
        Self::Transport(error)
    }
}
impl From<LogInfoError> for CaptureErrorKind {
    fn from(error: LogInfoError) -> Self {
        Self::Info(error)
    }
}
impl From<io::Error> for CaptureErrorKind {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for CaptureErrorKind {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::super::log_info::{
        ACTIVE_USER_RECORD_TYPE, DEVICE_INFO_RECORD_TYPE, INFO_RECORD_TYPE, LOG_SYSTEM_RECORD_TYPE,
        PERMISSIONS_RECORD_TYPE,
    };
    use super::super::xcp::{decode_datagram, encode_frame};
    use super::*;
    use std::collections::HashMap;
    use std::net::UdpSocket;
    use std::sync::{Arc, Mutex};
    use std::thread;

    pub(super) fn record(kind: u32, index: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"Mo".to_vec();
        bytes.extend_from_slice(&[index, 0]);
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    pub(super) fn info(end: u32) -> Vec<u8> {
        let mut system = [0; 16];
        system[12..16].copy_from_slice(&end.to_be_bytes());
        let children = [
            record(DEVICE_INFO_RECORD_TYPE, 0, &[0; 56]),
            record(LOG_SYSTEM_RECORD_TYPE, 0, &system),
        ]
        .concat();
        record(INFO_RECORD_TYPE, 0, &children)
    }
    pub(super) fn permissions(allowed: bool) -> Vec<u8> {
        let mut payload = [0; 72];
        payload[64] = u8::from(allowed);
        let leaf = record(PERMISSIONS_RECORD_TYPE, 0, &payload);
        record(
            INFO_RECORD_TYPE,
            0,
            &record(ACTIVE_USER_RECORD_TYPE, 0, &leaf),
        )
    }
    const SECTOR_DATA: &[u8] = b"raw-packets";
    struct MockDevice {
        endpoint: SocketAddr,
        commands: Arc<Mutex<Vec<Vec<u8>>>>,
        handle: thread::JoinHandle<()>,
    }
    fn mock_device(allowed: bool, changed_boundaries: bool) -> MockDevice {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let endpoint = socket.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let observed = commands.clone();
        let handle = thread::spawn(move || {
            let records = HashMap::from([
                ((0x26, LOG_INFO_SELECTOR), info(1)),
                ((0x26, PERMISSIONS_SELECTOR), permissions(allowed)),
                ((0x27, 0), record(0x0200_0034, 0, b"set!")),
                ((0x2a, 0), record(0x0200_0035, 0, b"meta!")),
                ((0x25, 0), record(0x0200_004c, 0, SECTOR_DATA)),
            ]);
            let mut current = Vec::new();
            let mut offset = 0;
            let mut info_reads = 0;
            let mut input = [0; 1024];
            loop {
                let (len, peer) = socket.recv_from(&mut input).unwrap();
                let frames = decode_datagram(&input[..len]).unwrap();
                assert_eq!(frames.len(), 1);
                let request = frames[0];
                let command = request.payload;
                observed.lock().unwrap().push(command.to_vec());
                let response = match command[0] {
                    0xff => {
                        assert_eq!(command, [0xff, 0]);
                        vec![0xff, 0, 0, 64, 64, 0]
                    }
                    0xfe => {
                        assert_eq!(command, [0xfe]);
                        socket
                            .send_to(&encode_frame(request.counter, &[0xff]).unwrap(), peer)
                            .unwrap();
                        break;
                    }
                    0xf6 => {
                        assert_eq!(command.len(), 8);
                        let key = (
                            command[3],
                            u32::from_le_bytes(command[4..8].try_into().unwrap()),
                        );
                        if key == (0x26, LOG_INFO_SELECTOR) {
                            info_reads += 1;
                            current = if changed_boundaries && info_reads > 1 {
                                info(2)
                            } else {
                                records[&key].clone()
                            };
                        } else {
                            current = records
                                .get(&key)
                                .expect("unexpected memory selector")
                                .clone();
                        }
                        offset = 0;
                        vec![0xff]
                    }
                    0xf5 => {
                        assert_eq!(command.len(), 2);
                        assert!(command[1] > 0);
                        let end = offset + usize::from(command[1]);
                        let mut response = vec![0xff];
                        response.extend_from_slice(
                            current
                                .get(offset..end)
                                .expect("read beyond selected record"),
                        );
                        offset = end;
                        response
                    }
                    opcode => panic!("unexpected state-changing command 0x{opcode:02x}"),
                };
                socket
                    .send_to(&encode_frame(request.counter, &response).unwrap(), peer)
                    .unwrap();
            }
        });
        MockDevice {
            endpoint,
            commands,
            handle,
        }
    }
    impl MockDevice {
        fn finish(self) -> Vec<Vec<u8>> {
            self.handle.join().unwrap();
            Arc::try_unwrap(self.commands)
                .unwrap()
                .into_inner()
                .unwrap()
        }
    }
    pub(super) struct TestDirectory(pub PathBuf);
    impl TestDirectory {
        pub(super) fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "i3rs-capture-test-{}-{}-{}",
                std::process::id(),
                unix_millis(),
                DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&base).unwrap();
            Self(base)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    pub(super) fn manifest(path: &Path) -> CaptureManifest {
        serde_json::from_reader(fs::File::open(path.join("manifest.json")).unwrap()).unwrap()
    }
    pub(super) fn assert_read_only(commands: &[Vec<u8>]) {
        assert!(
            commands
                .iter()
                .all(|command| matches!(command[0], 0xff | 0xfe | 0xf6 | 0xf5))
        );
    }
    fn has_selector(commands: &[Vec<u8>], extension: u8, address: u32) -> bool {
        commands.iter().any(|command| {
            command.len() == 8
                && command[0] == 0xf6
                && command[3] == extension
                && command[4..8] == address.to_le_bytes()
        })
    }

    #[test]
    fn loopback_download_preserves_raw_records_and_reopens_complete_manifest() {
        let device = mock_device(true, false);
        let base = TestDirectory::new();
        // An unrelated existing file in the selected base is never overwritten.
        fs::write(base.0.join("manifest.json"), b"existing").unwrap();
        let mut stages = Vec::new();
        let result = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &CaptureLimits::default(),
            Duration::from_secs(1),
            CancellationToken::default(),
            |progress| stages.push(progress.stage),
        )
        .unwrap();
        let saved = manifest(&result.directory);
        assert_eq!(saved.format, RAW_CAPTURE_FORMAT);
        assert_eq!(saved.state, CaptureState::Complete);
        assert_eq!(saved.boundaries_unchanged, Some(true));
        assert!(!saved.converter_validated);
        assert!(!saved.produces_ld);
        assert_eq!(saved.files.len(), 6);
        assert_eq!(
            saved.total_bytes,
            saved.files.iter().map(|file| file.bytes).sum::<u64>()
        );
        for file in &saved.files {
            assert!(file.complete);
            assert_eq!(file.bytes, file.expected_bytes);
            assert_eq!(
                fs::metadata(result.directory.join(&file.file_name))
                    .unwrap()
                    .len(),
                file.bytes
            );
            assert!(file.file_name.ends_with(".mo"));
        }
        let sector = saved
            .files
            .iter()
            .find(|file| file.sector == Some(0))
            .unwrap();
        assert_eq!(
            fs::read(result.directory.join(&sector.file_name)).unwrap(),
            record(0x0200_004c, 0, SECTOR_DATA)
        );
        assert_eq!(fs::read(base.0.join("manifest.json")).unwrap(), b"existing");
        assert_eq!(stages.last(), Some(&CaptureStage::Finished));
        let commands = device.finish();
        assert_read_only(&commands);
        assert!(has_selector(&commands, 0x26, PERMISSIONS_SELECTOR));
        assert!(has_selector(&commands, 0x25, 0));
    }

    #[test]
    fn inspection_fetches_active_user_permissions_without_sector_reads() {
        let device = mock_device(true, false);
        let summary = inspect_device(
            device.endpoint,
            Duration::from_secs(1),
            CancellationToken::default(),
            &CaptureLimits::default(),
        )
        .unwrap();
        assert!(summary.permissions_available);
        assert!(summary.download_allowed[0]);
        assert!(!summary.download_allowed[1]);
        assert_eq!(summary.systems[0].sector_count, Some(1));
        let commands = device.finish();
        assert_read_only(&commands);
        assert!(!has_selector(&commands, 0x25, 0));
        assert!(!has_selector(&commands, 0x27, 0));
    }

    #[test]
    fn denied_permissions_prevent_capture_directory_and_sector_commands() {
        let device = mock_device(false, false);
        let base = TestDirectory::new();
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &CaptureLimits::default(),
            Duration::from_secs(1),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error.kind, CaptureErrorKind::PermissionDenied(0)));
        assert!(error.bundle_directory.is_none());
        assert_eq!(fs::read_dir(&base.0).unwrap().count(), 0);
        let commands = device.finish();
        assert_read_only(&commands);
        assert!(!has_selector(&commands, 0x25, 0));
        assert!(!has_selector(&commands, 0x27, 0));
    }

    #[test]
    fn cancellation_retains_received_sector_in_incomplete_bundle() {
        let device = mock_device(true, false);
        let base = TestDirectory::new();
        let cancellation = CancellationToken::default();
        let token = cancellation.clone();
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &CaptureLimits::default(),
            Duration::from_secs(1),
            cancellation,
            |progress| {
                if progress.stage == CaptureStage::CheckingConsistency {
                    token.cancel();
                }
            },
        )
        .unwrap_err();
        assert!(matches!(error.kind, CaptureErrorKind::Cancelled));
        let path = error.bundle_directory.unwrap();
        let saved = manifest(&path);
        assert_eq!(saved.state, CaptureState::Incomplete);
        assert!(saved.error.unwrap().contains("cancelled"));
        assert!(
            saved
                .files
                .iter()
                .any(|file| file.sector == Some(0) && file.complete)
        );
        assert_eq!(saved.boundaries_unchanged, None);
        assert_read_only(&device.finish());
    }

    #[test]
    fn aggregate_byte_cap_stops_before_next_record_and_retains_info() {
        let device = mock_device(true, false);
        let base = TestDirectory::new();
        let limits = CaptureLimits {
            max_total_bytes: (info(1).len() + permissions(true).len() + 11) as u64,
            ..CaptureLimits::default()
        };
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &limits,
            Duration::from_secs(1),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CaptureErrorKind::ByteLimit { remaining: 11 }
        ));
        let saved = manifest(&error.bundle_directory.unwrap());
        assert_eq!(saved.state, CaptureState::Incomplete);
        assert_eq!(saved.files.len(), 2);
        assert!(saved.total_bytes <= limits.max_total_bytes);
        let commands = device.finish();
        assert_read_only(&commands);
        assert!(!has_selector(&commands, 0x27, 0));
        assert!(!has_selector(&commands, 0x25, 0));
    }

    #[test]
    fn changed_boundaries_mark_persisted_bundle_inconsistent() {
        let device = mock_device(true, true);
        let base = TestDirectory::new();
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &CaptureLimits::default(),
            Duration::from_secs(1),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(error.kind, CaptureErrorKind::BoundariesChanged(ref systems) if systems == &[0])
        );
        let saved = manifest(&error.bundle_directory.unwrap());
        assert_eq!(saved.state, CaptureState::Inconsistent);
        assert_eq!(saved.boundaries_unchanged, Some(false));
        assert_eq!(saved.files.len(), 6);
        assert_read_only(&device.finish());
    }

    #[test]
    fn invalid_and_duplicate_system_ids_fail_before_connecting() {
        let base = TestDirectory::new();
        for systems in [vec![], vec![8], vec![0, 0]] {
            let error = download_raw(
                "127.0.0.1:9".parse().unwrap(),
                systems,
                &base.0,
                &CaptureLimits::default(),
                Duration::from_secs(1),
                CancellationToken::default(),
                |_| {},
            )
            .unwrap_err();
            assert!(matches!(error.kind, CaptureErrorKind::InvalidInput(_)));
            assert!(error.bundle_directory.is_none());
        }
        assert_eq!(fs::read_dir(&base.0).unwrap().count(), 0);
    }
    #[test]
    fn declared_record_limit_stops_after_header_before_payload_upload() {
        let device = mock_device(true, false);
        let base = TestDirectory::new();
        let limits = CaptureLimits {
            max_config_bytes: 16, // Setups fit; metadata declares 17 bytes.
            ..CaptureLimits::default()
        };
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &limits,
            Duration::from_secs(1),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CaptureErrorKind::Transport(XcpError::ReadLimitExceeded {
                requested: 17,
                limit: 16
            })
        ));
        let saved = manifest(&error.bundle_directory.unwrap());
        assert_eq!(saved.state, CaptureState::Incomplete);
        assert_eq!(saved.files.len(), 3);
        let commands = device.finish();
        let metadata_start = commands
            .iter()
            .position(|command| command.len() == 8 && command[0] == 0xf6 && command[3] == 0x2a)
            .unwrap();
        assert_eq!(
            &commands[metadata_start + 1..],
            &[vec![0xf5, 12], vec![0xfe]]
        );
        assert_read_only(&commands);
    }

    #[test]
    fn selected_sector_count_is_checked_before_creating_a_bundle() {
        let device = mock_device(true, false);
        let base = TestDirectory::new();
        let limits = CaptureLimits {
            max_sectors: 0,
            ..CaptureLimits::default()
        };
        let error = download_raw(
            device.endpoint,
            vec![0],
            &base.0,
            &limits,
            Duration::from_secs(1),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CaptureErrorKind::Info(LogInfoError::SectorLimitExceeded { count: 1, limit: 0 })
        ));
        assert!(error.bundle_directory.is_none());
        assert_eq!(fs::read_dir(&base.0).unwrap().count(), 0);
        let commands = device.finish();
        assert!(!has_selector(&commands, 0x27, 0));
        assert!(!has_selector(&commands, 0x25, 0));
        assert_read_only(&commands);
    }
}

//! Read-only native decoding of an i3rs raw capture directory.
//!
//! This consumes exact saved records and never invents an official MoTeC image
//! envelope. Manifest names are matched to the capture format's fixed names;
//! record contents and the original sector plan are validated independently.
//! The packet sector identifier is unique within each system; its relationship
//! to the physical memory sector after ring-buffer wrapping is not established.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;

use super::capture::{CaptureManifest, CaptureState, RAW_CAPTURE_FORMAT};
use super::image_decode::{DecodedImage, ImageDecodeLimits, decode_parts_with_capture_start};
use super::log_image::{MO_HEADER_SIZE, MoRecord, MoRecordHeader, MoRecordIter};
use super::log_info::{INFO_RECORD_TYPE, LogInfo};
use super::transport::CancellationToken;
use crate::Session;

#[derive(Debug, Clone, Copy)]
pub struct CaptureDecodeLimits {
    pub image: ImageDecodeLimits,
    pub max_manifest_bytes: usize,
    pub max_info_bytes: usize,
    pub max_config_bytes: usize,
    pub max_sector_bytes: usize,
    pub max_total_bytes: u64,
    pub max_files: usize,
}
impl Default for CaptureDecodeLimits {
    fn default() -> Self {
        Self {
            image: ImageDecodeLimits::default(),
            max_manifest_bytes: 1024 * 1024,
            max_info_bytes: 4 * 1024 * 1024,
            max_config_bytes: 64 * 1024 * 1024,
            max_sector_bytes: 16 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_files: 10_016,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Before,
    After,
    Permissions,
    Setups,
    Metadata,
    Sector(u8, u32),
}
impl Role {
    fn cap(self, limits: &CaptureDecodeLimits) -> usize {
        match self {
            Self::Before | Self::After | Self::Permissions => limits.max_info_bytes,
            Self::Setups | Self::Metadata => limits.max_config_bytes,
            Self::Sector(..) => limits.max_sector_bytes,
        }
    }
    fn record_type(self) -> u32 {
        match self {
            Self::Before | Self::After | Self::Permissions => INFO_RECORD_TYPE,
            Self::Setups => 0x0200_0020,
            Self::Metadata => 0x0200_0046,
            Self::Sector(..) => 0x0200_004c,
        }
    }
}

/// Decode a complete, unchanged-boundary capture into native timestamped data.
/// Files are bounded before reading; incomplete or edited-size manifests fail.
/// Acquisition time is deliberately not used as the recording calendar date.
pub fn decode_capture(
    directory: &Path,
    limits: &ImageDecodeLimits,
) -> Result<DecodedImage, String> {
    decode_capture_with_limits(
        directory,
        &CaptureDecodeLimits {
            image: *limits,
            ..CaptureDecodeLimits::default()
        },
        None,
    )
}

pub fn decode_capture_with_cancel(
    directory: &Path,
    limits: &ImageDecodeLimits,
    cancellation: &CancellationToken,
) -> Result<DecodedImage, String> {
    decode_capture_with_limits(
        directory,
        &CaptureDecodeLimits {
            image: *limits,
            ..CaptureDecodeLimits::default()
        },
        Some(cancellation),
    )
}

pub fn decode_capture_with_limits(
    directory: &Path,
    limits: &CaptureDecodeLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<DecodedImage, String> {
    decode_bundle(directory, limits, cancellation, false)
}

/// Decode only the saved, contiguous part of an interrupted acquisition.
pub(super) fn decode_partial_capture(
    directory: &Path,
    limits: &ImageDecodeLimits,
    cancellation: &CancellationToken,
) -> Result<DecodedImage, String> {
    decode_bundle(
        directory,
        &CaptureDecodeLimits {
            image: *limits,
            ..CaptureDecodeLimits::default()
        },
        Some(cancellation),
        true,
    )
}

fn decode_bundle(
    directory: &Path,
    limits: &CaptureDecodeLimits,
    cancellation: Option<&CancellationToken>,
    partial: bool,
) -> Result<DecodedImage, String> {
    check_cancelled(cancellation)?;
    if limits.max_manifest_bytes == 0
        || limits.max_files == 0
        || limits.max_info_bytes < MO_HEADER_SIZE
        || limits.max_config_bytes < MO_HEADER_SIZE
        || limits.max_sector_bytes < MO_HEADER_SIZE
    {
        return Err("invalid raw capture decode limits".into());
    }
    let directory = BundleDirectory::open(directory)?;
    let manifest_bytes = directory.read("manifest.json", limits.max_manifest_bytes, None)?;
    let manifest: CaptureManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("invalid raw capture manifest: {e}"))?;
    if manifest.format != RAW_CAPTURE_FORMAT {
        return Err("unsupported raw capture format".into());
    }
    if partial {
        if manifest.state != CaptureState::Incomplete || manifest.boundaries_unchanged.is_some() {
            return Err(
                "partial recovery requires an interrupted capture without a final boundary check"
                    .into(),
            );
        }
    } else if manifest.state != CaptureState::Complete
        || manifest.boundaries_unchanged != Some(true)
        || manifest.error.is_some()
    {
        return Err("raw capture is incomplete or has inconsistent logging boundaries".into());
    }
    if manifest.selected_systems.is_empty()
        || manifest.selected_systems.iter().any(|id| *id >= 8)
        || manifest
            .selected_systems
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err("raw capture systems must be unique, sorted identifiers in 0..8".into());
    }
    if manifest.files.len() > limits.max_files {
        return Err("raw capture file count exceeds limit".into());
    }
    let selected = manifest
        .selected_systems
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    let mut roles = Vec::new();
    let mut total = 0u64;
    let mut config_bytes = 0usize;
    let mut stream_bytes = 0usize;
    let mut sectors = 0usize;
    let mut before_index = None;
    let mut after_index = None;
    let mut setups_index = None;
    let mut metadata_index = None;
    for (index, entry) in manifest.files.iter().enumerate() {
        check_cancelled(cancellation)?;
        if !entry.complete
            || entry.bytes != entry.expected_bytes
            || entry.bytes < MO_HEADER_SIZE as u64
        {
            return Err("raw capture contains an incomplete record".into());
        }
        if !names.insert(entry.file_name.as_str()) {
            return Err("raw capture has duplicate filenames".into());
        }
        let role = match (entry.system, entry.sector, entry.file_name.as_str()) {
            (None, None, "logging-info-before.mo") => {
                set_unique(&mut before_index, index)?;
                Role::Before
            }
            (None, None, "logging-info-after.mo") => {
                set_unique(&mut after_index, index)?;
                Role::After
            }
            (None, None, "active-user-permissions.mo") => Role::Permissions,
            (None, None, "logging-setups.mo") => {
                set_unique(&mut setups_index, index)?;
                Role::Setups
            }
            (None, None, "logging-metadata.mo") => {
                set_unique(&mut metadata_index, index)?;
                Role::Metadata
            }
            (Some(system), Some(sector), name)
                if selected.contains(&system)
                    && name == format!("system-{system:02}-sector-{sector:010}.mo") =>
            {
                Role::Sector(system, sector)
            }
            _ => return Err("raw capture contains an unexpected filename or file identity".into()),
        };
        let size = usize::try_from(entry.bytes).map_err(|_| "raw capture record size overflow")?;
        if size > role.cap(limits) {
            return Err("raw capture record exceeds its byte limit".into());
        }
        total = total
            .checked_add(entry.bytes)
            .ok_or("raw capture total size overflow")?;
        if total > limits.max_total_bytes {
            return Err("raw capture total exceeds byte limit".into());
        }
        match role {
            Role::Sector(..) => {
                sectors = sectors
                    .checked_add(1)
                    .ok_or("raw capture sector count overflow")?;
                if sectors > limits.image.telemetry.max_sectors {
                    return Err("raw capture sector count exceeds limit".into());
                }
                let padding = (4 - stream_bytes % 4) % 4;
                stream_bytes = stream_bytes
                    .checked_add(padding)
                    .and_then(|n| n.checked_add(size))
                    .ok_or("raw capture stream size overflow")?;
            }
            Role::Setups | Role::Metadata => {
                config_bytes = config_bytes
                    .checked_add(size)
                    .ok_or("raw capture config size overflow")?;
            }
            _ => {}
        }
        if config_bytes
            .checked_add(stream_bytes)
            .ok_or("raw capture decode size overflow")?
            > limits.image.max_image_bytes
        {
            return Err("raw capture decoded parts exceed input byte limit".into());
        }
        directory.check_size(&entry.file_name, entry.bytes)?;
        roles.push(role);
    }
    if total != manifest.total_bytes {
        return Err("raw capture manifest total does not match its files".into());
    }
    let before_index = before_index.ok_or("raw capture has no initial logging info")?;
    if partial && after_index.is_some() {
        return Err("partial recovery requires a capture without final logging info".into());
    }
    if !partial && after_index.is_none() {
        return Err("raw capture has no final logging info".into());
    }
    let setups_index = setups_index.ok_or("raw capture has no logging setups")?;
    let metadata_index = metadata_index.ok_or("raw capture has no logging metadata")?;
    let read_record = |index: usize| -> Result<Vec<u8>, String> {
        check_cancelled(cancellation)?;
        let entry = &manifest.files[index];
        let bytes = directory.read(
            &entry.file_name,
            roles[index].cap(limits),
            Some(entry.bytes),
        )?;
        validate_record(&bytes, roles[index])?;
        Ok(bytes)
    };
    let before_bytes = read_record(before_index)?;
    let after_bytes = after_index.map(read_record).transpose()?;
    let before = LogInfo::parse_with_limit(&before_bytes, limits.image.telemetry.max_records)
        .map_err(|e| e.to_string())?;
    let after = after_bytes
        .as_deref()
        .map(|bytes| LogInfo::parse_with_limit(bytes, limits.image.telemetry.max_records))
        .transpose()
        .map_err(|e| e.to_string())?;
    let mut expected = Vec::new();
    for &system in &manifest.selected_systems {
        let initial = before
            .system(system)
            .ok_or("selected system missing from initial logging info")?;
        if let Some(after) = &after {
            let final_system = after
                .system(system)
                .ok_or("selected system missing from final logging info")?;
            if initial.boundary_a != final_system.boundary_a
                || initial.boundary_b != final_system.boundary_b
            {
                return Err("captured logging boundaries changed".into());
            }
        }
        let plan = initial
            .plan(
                before.device.as_ref(),
                limits.image.telemetry.max_sectors as u64,
            )
            .map_err(|e| e.to_string())?;
        if plan.len()
            > limits
                .image
                .telemetry
                .max_sectors
                .saturating_sub(expected.len()) as u64
        {
            return Err("raw capture sector plan exceeds limit".into());
        }
        expected.extend(plan.sectors().map(|sector| (system, sector)));
        let summary = manifest
            .initial
            .systems
            .iter()
            .filter(|summary| summary.id == system)
            .collect::<Vec<_>>();
        if summary.len() != 1
            || summary[0].boundary_a != initial.boundary_a
            || summary[0].boundary_b != initial.boundary_b
            || summary[0].sector_count != Some(plan.len())
            || summary[0].plan_error.is_some()
        {
            return Err("raw capture inspection summary differs from logging records".into());
        }
    }
    let actual = roles
        .iter()
        .filter_map(|role| match role {
            Role::Sector(system, sector) => Some((*system, *sector)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if partial {
        if actual.is_empty() || !expected.starts_with(&actual) {
            return Err("partial capture must contain a nonempty, contiguous prefix of the original read plan".into());
        }
    } else if actual != expected {
        return Err("raw capture sectors do not match the original read plan and order".into());
    }
    let setup_bytes = read_record(setups_index)?;
    let metadata_bytes = read_record(metadata_index)?;
    let mut joined = Vec::new();
    let mut logical_ids = BTreeSet::new();
    for (index, role) in roles.iter().enumerate() {
        match role {
            Role::Sector(system, _) => {
                let bytes = read_record(index)?;
                let id = sector_identity(&bytes)?;
                if !logical_ids.insert((*system, id)) {
                    return Err("duplicate captured logical sector identifier".into());
                }
                let padding = (4 - joined.len() % 4) % 4;
                let new_len = joined
                    .len()
                    .checked_add(padding)
                    .and_then(|n| n.checked_add(bytes.len()))
                    .ok_or("raw capture stream size overflow")?;
                if new_len > limits.image.max_image_bytes || new_len > u32::MAX as usize {
                    return Err("raw capture stream exceeds byte limit".into());
                }
                joined
                    .try_reserve(padding + bytes.len())
                    .map_err(|_| "could not allocate bounded capture stream")?;
                joined.resize(joined.len() + padding, 0);
                joined.extend_from_slice(&bytes);
            }
            Role::Permissions => {
                let bytes = read_record(index)?;
                LogInfo::parse_with_limit(&bytes, limits.image.telemetry.max_records)
                    .map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    let session = Session {
        date: String::new(),
        time: String::new(),
        driver: String::new(),
        vehicle_id: String::new(),
        venue: String::new(),
        short_comment: String::new(),
        device_serial: 0,
        device_type: "M1".into(),
        device_version: 0,
        num_channels_header: 0,
    };
    let stream = MoRecord {
        offset: 0,
        header: MoRecordHeader {
            flags: 0,
            encoding: 0,
            record_type: 0x0101_0002,
            payload_len: u32::try_from(joined.len())
                .map_err(|_| "raw capture stream size overflow")?,
        },
        payload: &joined,
    };
    check_cancelled(cancellation)?;
    let mut decoded = decode_parts_with_capture_start(
        exact_record(&setup_bytes)?,
        exact_record(&metadata_bytes)?,
        stream,
        session,
        &limits.image,
        true,
    )?;
    check_cancelled(cancellation)?;
    if directory.read("manifest.json", limits.max_manifest_bytes, None)? != manifest_bytes {
        return Err("raw capture manifest changed during decoding".into());
    }
    decoded.warnings.push("Decoded an i3rs raw capture bundle, not an official MoTeC logged-data image. Acquisition boundary checks do not establish an atomic ECU snapshot.".into());
    if partial {
        decoded.warnings.push(format!(
            "Partial capture: saved {} of {} planned sectors. Final ECU boundaries were not verified.",
            actual.len(), expected.len(),
        ));
    }
    decoded.warnings.push("Recording identity and calendar metadata were unavailable in the raw capture; those session fields are blank.".into());
    Ok(decoded)
}

fn check_cancelled(cancellation: Option<&CancellationToken>) -> Result<(), String> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err("capture decoding cancelled".into());
    }
    Ok(())
}

fn set_unique(slot: &mut Option<usize>, index: usize) -> Result<(), String> {
    if slot.replace(index).is_some() {
        return Err("duplicate raw capture record role".into());
    }
    Ok(())
}
fn exact_record(bytes: &[u8]) -> Result<MoRecord<'_>, String> {
    let record = MoRecordIter::new(bytes, 1)
        .next()
        .ok_or("missing captured record")?
        .map_err(|e| e.to_string())?;
    if record.payload.len().checked_add(MO_HEADER_SIZE) != Some(bytes.len()) {
        return Err("captured record has trailing data".into());
    }
    Ok(record)
}
fn validate_record(bytes: &[u8], role: Role) -> Result<(), String> {
    let record = exact_record(bytes)?;
    if record.header.record_type != role.record_type() || record.header.encoding != 0 {
        return Err("captured record has an unexpected type or encoding".into());
    }
    let expected_system = match role {
        Role::Sector(system, _) => system,
        _ => 0,
    };
    if record.header.flags != expected_system {
        return Err("captured record system header differs from manifest".into());
    }
    if let Role::Sector(..) = role {
        let first = MoRecordIter::new(record.payload, 1)
            .next()
            .ok_or("captured sector has no identifier")?
            .map_err(|e| e.to_string())?;
        if first.header.record_type != 0x0201_004d
            || first.header.encoding != 0
            || first.header.flags != 0
            || first.payload.len() != 4
        {
            return Err("captured sector has an invalid logical identifier".into());
        }
    }
    Ok(())
}

fn sector_identity(bytes: &[u8]) -> Result<u32, String> {
    let sector = exact_record(bytes)?;
    let first = MoRecordIter::new(sector.payload, 1)
        .next()
        .ok_or("missing sector identifier")?
        .map_err(|e| e.to_string())?;
    Ok(u32::from_be_bytes(
        first
            .payload
            .try_into()
            .map_err(|_| "invalid logical sector identifier")?,
    ))
}

struct BundleDirectory {
    #[cfg(not(unix))]
    root: PathBuf,
    #[cfg(unix)]
    handle: File,
}
impl BundleDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| format!("could not inspect capture directory: {e}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("capture source must be a directory, not a symlink".into());
        }
        let root = fs::canonicalize(path)
            .map_err(|e| format!("could not resolve capture directory: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let handle = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(&root)
                .map_err(|e| format!("could not open capture directory: {e}"))?;
            Ok(Self { handle })
        }
        #[cfg(not(unix))]
        {
            Ok(Self { root })
        }
    }
    fn open_file(&self, name: &str) -> Result<File, String> {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || name.contains(['/', '\\'])
            || name == "."
            || name == ".."
        {
            return Err("capture filename is not a plain basename".into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name).map_err(|_| "capture filename contains NUL")?;
            // A single validated basename relative to a pinned directory cannot
            // escape through a replaced parent directory or a file symlink.
            let fd = unsafe {
                libc::openat(
                    self.handle.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "could not open capture record: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: openat returned a fresh owned descriptor on success.
            let file = unsafe { File::from_raw_fd(fd) };
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("captured record is not a regular file".into());
            }
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let path = self.root.join(name);
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err("captured record is not a regular file".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(0x00200000)
                    .open(path)
                    .map_err(|e| e.to_string())?;
                let meta = file.metadata().map_err(|e| e.to_string())?;
                if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
                    return Err("captured record is a reparse point".into());
                }
                Ok(file)
            }
            #[cfg(not(windows))]
            {
                File::open(path).map_err(|e| e.to_string())
            }
        }
    }
    fn check_size(&self, name: &str, expected: u64) -> Result<(), String> {
        let file = self.open_file(name)?;
        if file.metadata().map_err(|e| e.to_string())?.len() != expected {
            return Err("captured file size differs from manifest".into());
        }
        Ok(())
    }
    fn read(&self, name: &str, cap: usize, expected: Option<u64>) -> Result<Vec<u8>, String> {
        let mut file = self.open_file(name)?;
        let before = file.metadata().map_err(|e| e.to_string())?.len();
        if before > cap as u64 || expected.is_some_and(|n| n != before) {
            return Err("capture file exceeds limit or differs from manifest size".into());
        }
        let count = usize::try_from(before).map_err(|_| "capture file size overflow")?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(count)
            .map_err(|_| "could not allocate bounded capture record")?;
        let bound = before.checked_add(1).ok_or("capture read size overflow")?;
        (&mut file)
            .take(bound)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() != count || file.metadata().map_err(|e| e.to_string())?.len() != before {
            return Err("captured file changed during reading".into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::m1::capture::{CapturedFile, DeviceInspection, SystemSummary};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn record(kind: u32, index: u8, encoding: u8, payload: &[u8]) -> Vec<u8> {
        let mut b = b"Mo".to_vec();
        b.extend([index, encoding]);
        b.extend(kind.to_be_bytes());
        b.extend((payload.len() as u32).to_be_bytes());
        b.extend(payload);
        b
    }
    fn joined(records: Vec<Vec<u8>>) -> Vec<u8> {
        let mut b = Vec::new();
        for r in records {
            b.resize(b.len().next_multiple_of(4), 0);
            b.extend(r);
        }
        b
    }
    fn varint(mut value: usize, out: &mut Vec<u8>) {
        while value >= 128 {
            out.push((value as u8 & 127) | 128);
            value >>= 7;
        }
        out.push(value as u8);
    }
    // Independent synthetic XZ using an uncompressed LZMA2 block, not fixture bytes.
    fn xz(data: &[u8]) -> Vec<u8> {
        let flags = [0, 1];
        let mut x = b"\xfd7zXZ\0".to_vec();
        x.extend(flags);
        x.extend(crc32fast::hash(&flags).to_le_bytes());
        let header = [2, 0, 0x21, 1, 0, 0, 0, 0];
        x.extend(header);
        x.extend(crc32fast::hash(&header).to_le_bytes());
        x.push(1);
        x.extend(((data.len() - 1) as u16).to_be_bytes());
        x.extend(data);
        x.push(0);
        x.resize(x.len().next_multiple_of(4), 0);
        x.extend(crc32fast::hash(data).to_le_bytes());
        let mut index = vec![0, 1];
        varint(12 + data.len() + 4 + 4, &mut index);
        varint(data.len(), &mut index);
        index.resize(index.len().next_multiple_of(4), 0);
        index.extend(crc32fast::hash(&index).to_le_bytes());
        let back = (index.len() / 4 - 1) as u32;
        x.extend(index);
        let mut footer = back.to_le_bytes().to_vec();
        footer.extend(flags);
        x.extend(crc32fast::hash(&footer).to_le_bytes());
        x.extend(footer);
        x.extend(b"YZ");
        x
    }
    fn info(end: u32) -> Vec<u8> {
        let mut p = vec![0; 16];
        p[12..16].copy_from_slice(&end.to_be_bytes());
        record(INFO_RECORD_TYPE, 0, 0, &record(0x0201_002f, 0, 0, &p))
    }
    fn setups() -> Vec<u8> {
        let mut p = vec![0; 16];
        p.extend([1; 16]);
        let h = [1u32, 7816, 0, 1, 4]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        let e = [1u32, 0x100, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        record(
            0x0200_0020,
            0,
            0,
            &record(
                0x0200_0022,
                0,
                0,
                &joined(vec![
                    record(0x0201_0023, 0, 0, &p),
                    record(0x0201_0044, 0, 0, &[2; 16]),
                    record(
                        0x0200_0024,
                        0,
                        0,
                        &joined(vec![
                            record(0x0201_0025, 0, 0, &h),
                            record(0x0201_0026, 0, 0, &e),
                        ]),
                    ),
                ]),
            ),
        )
    }
    fn metadata() -> Vec<u8> {
        let xml=br#"<TuneConfig><System><TargetModel><Events><Event Index="0" Period="1"/></Events></TargetModel></System><Device><Items><BuiltIn.Channel Name="Speed"><Properties><Value Type="f32" Unit="rpm"><Address Base="1" Offset="256"/></Value></Properties></BuiltIn.Channel></Items></Device></TuneConfig>"#;
        let plain = record(0x0201_002c, 0, 0, xml);
        let mut payload = (plain.len() as u32).to_be_bytes().to_vec();
        payload.extend(xz(&plain));
        record(
            0x0200_0046,
            0,
            0,
            &record(
                0x0200_0047,
                0,
                0,
                &joined(vec![
                    record(0x0201_0044, 0, 0, &[2; 16]),
                    record(0x0201_002c, 0, 31, &payload),
                ]),
            ),
        )
    }
    fn sector(id: u32) -> Vec<u8> {
        let mut packets = vec![record(0x0201_004d, 0, 0, &id.to_be_bytes())];
        if id == 0 {
            packets.push(record(0x0201_0039, 0, 0, &[1; 16]));
            packets.push(record(0x0201_0029, 0, 0, &0u32.to_be_bytes()));
        }
        let mut sample = (id * 125_000_000).to_be_bytes().to_vec();
        sample.extend((1000.0f32 + id as f32).to_bits().to_be_bytes());
        sample.extend(crc32fast::hash(&sample).to_be_bytes());
        packets.push(record(0x0201_002b, 0, 0, &sample));
        record(0x0200_004c, 0, 0, &joined(packets))
    }
    fn fixture() -> (Fixture, CaptureManifest) {
        let path = std::env::temp_dir().join(format!(
            "i3rs-capture-decode-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let files = vec![
            ("logging-info-before.mo".to_string(), info(2), None, None),
            ("logging-setups.mo".to_string(), setups(), None, None),
            ("logging-metadata.mo".to_string(), metadata(), None, None),
            (
                "system-00-sector-0000000001.mo".into(),
                sector(1),
                Some(0),
                Some(1),
            ),
            (
                "system-00-sector-0000000000.mo".into(),
                sector(0),
                Some(0),
                Some(0),
            ),
            ("logging-info-after.mo".to_string(), info(2), None, None),
        ];
        let mut entries = Vec::new();
        let mut total = 0;
        for (name, bytes, system, sector) in files {
            fs::write(path.join(&name), &bytes).unwrap();
            total += bytes.len() as u64;
            entries.push(CapturedFile {
                file_name: name,
                expected_bytes: bytes.len() as u64,
                bytes: bytes.len() as u64,
                complete: true,
                system,
                sector,
            });
        }
        let initial = DeviceInspection {
            endpoint: "127.0.0.1:5555".into(),
            permissions_available: true,
            download_allowed: [true; 8],
            device: None,
            systems: vec![SystemSummary {
                id: 0,
                boundary_a: 0,
                boundary_b: 2,
                sector_count: Some(2),
                plan_error: None,
            }],
            disconnect_diagnostic: None,
        };
        let manifest = CaptureManifest {
            format: RAW_CAPTURE_FORMAT.into(),
            state: CaptureState::Complete,
            endpoint: initial.endpoint.clone(),
            started_unix_millis: 0,
            selected_systems: vec![0],
            initial,
            files: entries,
            total_bytes: total,
            boundaries_unchanged: Some(true),
            converter_validated: false,
            produces_ld: false,
            error: None,
            diagnostics: Vec::new(),
        };
        save_manifest(&path, &manifest);
        (Fixture(path), manifest)
    }
    fn save_manifest(path: &Path, manifest: &CaptureManifest) {
        fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(manifest).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn complete_capture_decodes_direct_parts_and_cross_sector_clock() {
        let (f, _) = fixture();
        let d = decode_capture(&f.0, &ImageDecodeLimits::default()).unwrap();
        let c = &d.telemetry.systems[0].segments[0].channels[0];
        assert_eq!(c.samples.len(), 2);
        assert_eq!(c.samples[0].value, 1000.);
        assert_eq!(c.samples[1].value, 1001.);
        assert_eq!(c.samples[1].time_seconds, 1.);
        assert!(d.session.date.is_empty());
        assert!(d.warnings.iter().any(|w| w.contains("raw capture bundle")));
    }
    #[test]
    fn complete_capture_can_begin_mid_recording_but_still_checks_crc_and_reset_context() {
        let (f, mut manifest) = fixture();
        let oldest = sector(0);
        let first_value = sector(1);
        let mut packets = oldest[12..28].to_vec();
        packets.extend_from_slice(&first_value[28..]);
        packets.extend_from_slice(&oldest[28..]);

        let mut save_oldest = |packets: &[u8]| {
            let bytes = record(0x0200_004c, 0, 0, packets);
            let entry = &mut manifest.files[4];
            manifest.total_bytes = manifest.total_bytes - entry.bytes + bytes.len() as u64;
            entry.bytes = bytes.len() as u64;
            entry.expected_bytes = entry.bytes;
            fs::write(f.0.join(&entry.file_name), bytes).unwrap();
            save_manifest(&f.0, &manifest);
        };

        let limits = ImageDecodeLimits::default();
        save_oldest(&packets);
        let decoded = decode_capture(&f.0, &limits).unwrap();
        let samples = &decoded.telemetry.systems[0].segments[0].channels[0].samples;
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].value, 1000.);
        assert_eq!(samples[1].time_seconds, 1.);
        assert!(
            decoded
                .warnings
                .iter()
                .any(|w| w.contains("omitted 1 leading"))
        );

        packets[32] ^= 1; // Corrupt the omitted packet's data.
        save_oldest(&packets);
        assert!(
            decode_capture(&f.0, &limits)
                .unwrap_err()
                .contains("CRC mismatch")
        );

        packets[32] ^= 1;
        packets.extend(record(0x0201_002a, 0, 0, &[]));
        save_oldest(&packets);
        assert!(
            decode_capture(&f.0, &limits)
                .unwrap_err()
                .contains("precedes logging configuration")
        );
    }

    #[test]
    fn partial_recovery_requires_a_contiguous_prefix_and_synchronizes_the_oldest_edge() {
        let (f, mut manifest) = fixture();
        let oldest = manifest.files.remove(4);
        let after = manifest.files.pop().unwrap();
        manifest.total_bytes -= oldest.bytes + after.bytes;
        manifest.state = CaptureState::Incomplete;
        manifest.boundaries_unchanged = None;
        manifest.error = Some("ECU rejected XCP UPLOAD with error 0x31".into());

        // The saved sector starts with a numeric packet from before the setup
        // and timing markers. Its next numeric packet has both.
        let original = sector(1);
        let values = &original[28..];
        let mut packets = original[12..].to_vec();
        packets.extend(record(0x0201_0039, 0, 0, &[1; 16]));
        packets.extend(record(0x0201_0029, 0, 0, &0u32.to_be_bytes()));
        packets.extend(values);
        let bytes = record(0x0200_004c, 0, 0, &packets);
        let entry = &mut manifest.files[3];
        manifest.total_bytes += bytes.len() as u64 - entry.bytes;
        entry.bytes = bytes.len() as u64;
        entry.expected_bytes = entry.bytes;
        fs::write(f.0.join(&entry.file_name), bytes).unwrap();
        save_manifest(&f.0, &manifest);

        let limits = ImageDecodeLimits::default();
        let cancellation = CancellationToken::default();
        assert!(decode_capture(&f.0, &limits).is_err());
        let decoded = decode_partial_capture(&f.0, &limits, &cancellation).unwrap();
        let channel = &decoded.telemetry.systems[0].segments[0].channels[0];
        assert_eq!(channel.samples.len(), 1);
        assert_eq!(channel.samples[0].value, 1001.);
        assert!(decoded.warnings.iter().any(|w| w.contains("1 of 2")));

        // A missing newer sector is not an interrupted newest-first prefix.
        manifest.total_bytes = manifest.total_bytes - manifest.files[3].bytes + oldest.bytes;
        manifest.files[3] = oldest;
        save_manifest(&f.0, &manifest);
        assert!(
            decode_partial_capture(&f.0, &limits, &cancellation)
                .unwrap_err()
                .contains("contiguous prefix")
        );
    }

    #[test]
    fn manifest_paths_status_totals_and_limits_fail_closed() {
        let (f, mut m) = fixture();
        m.state = CaptureState::Incomplete;
        save_manifest(&f.0, &m);
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("incomplete")
        );
        m.state = CaptureState::Complete;
        m.files[1].file_name = "../logging-setups.mo".into();
        save_manifest(&f.0, &m);
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("filename")
        );
        let (f, mut m) = fixture();
        m.total_bytes += 1;
        save_manifest(&f.0, &m);
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("total")
        );
        let (f, _) = fixture();
        for limits in [
            CaptureDecodeLimits {
                max_manifest_bytes: 8,
                ..CaptureDecodeLimits::default()
            },
            CaptureDecodeLimits {
                max_total_bytes: 20,
                ..CaptureDecodeLimits::default()
            },
            CaptureDecodeLimits {
                max_sector_bytes: 12,
                ..CaptureDecodeLimits::default()
            },
            CaptureDecodeLimits {
                max_files: 2,
                ..CaptureDecodeLimits::default()
            },
        ] {
            assert!(decode_capture_with_limits(&f.0, &limits, None).is_err());
        }
        let cancelled = CancellationToken::default();
        cancelled.cancel();
        assert!(
            decode_capture_with_cancel(&f.0, &ImageDecodeLimits::default(), &cancelled)
                .unwrap_err()
                .contains("cancelled")
        );
    }
    #[test]
    fn actual_boundaries_sector_identity_and_order_are_checked() {
        let (f, _) = fixture();
        fs::write(f.0.join("logging-info-after.mo"), info(3)).unwrap();
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("boundaries")
        );
        let (f, mut m) = fixture();
        m.files.swap(3, 4);
        save_manifest(&f.0, &m);
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("plan")
        );
        let (f, _) = fixture();
        let mut s = sector(1);
        s[2] = 1;
        fs::write(f.0.join("system-00-sector-0000000001.mo"), s).unwrap();
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("system header")
        );
        let (f, _) = fixture();
        let mut s = sector(1);
        s[19] = 0x4e;
        fs::write(f.0.join("system-00-sector-0000000001.mo"), s).unwrap();
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("identifier")
        );
    }
    #[test]
    fn logical_sector_ids_are_unique_but_not_assumed_physical_addresses() {
        let (f, _) = fixture();
        let path = f.0.join("system-00-sector-0000000001.mo");
        let mut bytes = sector(1);
        bytes[24..28].copy_from_slice(&777u32.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(decode_capture(&f.0, &ImageDecodeLimits::default()).is_ok());
        bytes[24..28].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("duplicate captured logical")
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_records_are_never_followed() {
        use std::os::unix::fs::symlink;
        let (f, _) = fixture();
        let outside = f.0.with_extension("outside");
        fs::write(&outside, setups()).unwrap();
        let record = f.0.join("logging-setups.mo");
        fs::remove_file(&record).unwrap();
        symlink(&outside, &record).unwrap();
        assert!(
            decode_capture(&f.0, &ImageDecodeLimits::default())
                .unwrap_err()
                .contains("open capture record")
        );
        assert_eq!(fs::read(&outside).unwrap(), setups());
        fs::remove_file(outside).unwrap();
    }
}

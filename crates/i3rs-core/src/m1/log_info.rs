//! Bounded parsing and read planning for M1 Tune's logging-info Mo stream.
//!
//! Offsets come from Tune 1.4.0.0302: `6a3580`/`6a33d0` copy typed
//! payloads to object+0x2c, `88f040` constructs circular ranges, and `88df00`
//! maps sectors to memory addresses. Header byte 2 is an item index only for
//! the classes interpreted here (`ac01e0`); other records remain opaque.

use std::{fmt, ops::Range};

use super::log_image::{MO_HEADER_SIZE, MoRecord, MoRecordIter, RecordError};

pub const INFO_RECORD_TYPE: u32 = 0x0200_002e;
pub const LOG_SYSTEM_RECORD_TYPE: u32 = 0x0201_002f;
pub const DEVICE_INFO_RECORD_TYPE: u32 = 0x0201_0033;
pub const ACTIVE_USER_RECORD_TYPE: u32 = 0x0200_004b;
pub const PERMISSIONS_RECORD_TYPE: u32 = 0x0201_0010;
pub const DEFAULT_INFO_RECORD_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogInfo<'a> {
    pub root: MoRecord<'a>,
    pub device: Option<DeviceLogInfo<'a>>,
    pub systems: Vec<LogSystemInfo<'a>>,
    pub permissions: Option<LogPermissions<'a>>,
    /// Uninterpreted direct children, including unknown active-user fields.
    pub other_records: Vec<MoRecord<'a>>,
}

impl<'a> LogInfo<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, LogInfoError> {
        Self::parse_with_limit(bytes, DEFAULT_INFO_RECORD_LIMIT)
    }

    /// Parse one exact, untransformed info root. Unknown payloads are never
    /// traversed. The shared budget includes the root and active-user children.
    pub fn parse_with_limit(bytes: &'a [u8], limit: usize) -> Result<Self, LogInfoError> {
        let mut budget = Budget {
            remaining: limit,
            limit,
        };
        budget.consume()?;
        let root = MoRecordIter::new(bytes, 1)
            .next()
            .ok_or(LogInfoError::MissingRoot)??;
        if root.header.record_type != INFO_RECORD_TYPE {
            return Err(LogInfoError::UnexpectedRoot(root.header.record_type));
        }
        if root.payload.len().checked_add(MO_HEADER_SIZE) != Some(bytes.len()) {
            return Err(LogInfoError::TrailingData);
        }
        plain(root)?;
        let mut info = Self {
            root,
            device: None,
            systems: Vec::new(),
            permissions: None,
            other_records: Vec::new(),
        };
        let mut active_user_seen = false;
        for record in MoRecordIter::new(root.payload, usize::MAX) {
            budget.consume()?;
            let record = at_offset(record, MO_HEADER_SIZE)?;
            match record.header.record_type {
                DEVICE_INFO_RECORD_TYPE if record.header.flags == 0 => {
                    plain(record)?;
                    minimum(record, 16)?;
                    if info.device.is_some() {
                        return Err(LogInfoError::DuplicateRecord {
                            record_type: DEVICE_INFO_RECORD_TYPE,
                            index: 0,
                        });
                    }
                    info.device = Some(DeviceLogInfo { record });
                }
                LOG_SYSTEM_RECORD_TYPE => {
                    plain(record)?;
                    minimum(record, 16)?;
                    let id = record.header.flags;
                    if id >= 8 {
                        return Err(LogInfoError::InvalidSystemId(id));
                    }
                    if info.systems.iter().any(|system| system.id == id) {
                        return Err(LogInfoError::DuplicateRecord {
                            record_type: LOG_SYSTEM_RECORD_TYPE,
                            index: id,
                        });
                    }
                    info.systems.push(LogSystemInfo {
                        id,
                        boundary_a: be32(record.payload, 8).unwrap(),
                        boundary_b: be32(record.payload, 12).unwrap(),
                        record,
                    });
                }
                ACTIVE_USER_RECORD_TYPE if record.header.flags == 0 => {
                    plain(record)?;
                    if active_user_seen {
                        return Err(LogInfoError::DuplicateRecord {
                            record_type: ACTIVE_USER_RECORD_TYPE,
                            index: 0,
                        });
                    }
                    active_user_seen = true;
                    for child in MoRecordIter::new(record.payload, usize::MAX) {
                        budget.consume()?;
                        let child = at_offset(child, record.offset + MO_HEADER_SIZE)?;
                        if child.header.record_type == PERMISSIONS_RECORD_TYPE
                            && child.header.flags == 0
                        {
                            plain(child)?;
                            // Only bytes 64..72 are needed for download rights.
                            // Tune zero-pads shorter old records; do not infer a
                            // permission when the complete array is absent.
                            minimum(child, 72)?;
                            if info.permissions.is_some() {
                                return Err(LogInfoError::DuplicateRecord {
                                    record_type: PERMISSIONS_RECORD_TYPE,
                                    index: 0,
                                });
                            }
                            info.permissions = Some(LogPermissions { record: child });
                        } else {
                            info.other_records.push(child);
                        }
                    }
                }
                _ => info.other_records.push(record),
            }
        }
        Ok(info)
    }

    pub fn system(&self, id: u8) -> Option<&LogSystemInfo<'a>> {
        self.systems.iter().find(|system| system.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLogInfo<'a> {
    pub record: MoRecord<'a>,
}

impl DeviceLogInfo<'_> {
    pub fn address_numerator(&self) -> Option<u32> {
        be32(self.record.payload, 16)
    }
    pub fn last_sector(&self) -> Option<u32> {
        be32(self.record.payload, 40)
    }
    pub fn address_denominator(&self) -> Option<u32> {
        be32(self.record.payload, 44)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPermissions<'a> {
    pub record: MoRecord<'a>,
}

impl LogPermissions<'_> {
    /// The active user's per-system download bit observed in `88bea0`.
    pub fn can_download(&self, system: u8) -> bool {
        system < 8
            && self
                .record
                .payload
                .get(64 + usize::from(system))
                .is_some_and(|flags| flags & 1 != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogSystemInfo<'a> {
    pub id: u8,
    pub boundary_a: u32,
    pub boundary_b: u32,
    pub record: MoRecord<'a>,
}

impl LogSystemInfo<'_> {
    /// Construct a bounded lazy plan. A == B is empty; B > A is [A,B).
    /// The wrapped case needs an explicit last-sector field from device info.
    pub fn plan(
        &self,
        device: Option<&DeviceLogInfo<'_>>,
        max_sectors: u64,
    ) -> Result<SectorPlan, LogInfoError> {
        if self.id >= 8 {
            return Err(LogInfoError::InvalidSystemId(self.id));
        }
        // Equal boundaries produce no reads, regardless of unused mapping
        // fields or out-of-range boundary values in an empty system.
        if self.boundary_a == self.boundary_b {
            return Ok(SectorPlan {
                system: self.id,
                ranges: Vec::new(),
                count: 0,
                addressing: Addressing::Legacy,
            });
        }
        let last = device.and_then(DeviceLogInfo::last_sector);
        let addressing = if last.is_some_and(|last| last > 0) {
            let device = device.unwrap();
            let numerator = device
                .address_numerator()
                .ok_or(LogInfoError::MissingAddressMapping)?;
            let denominator = device
                .address_denominator()
                .ok_or(LogInfoError::MissingAddressMapping)?;
            if numerator == 0 || denominator == 0 {
                return Err(LogInfoError::InvalidAddressMapping);
            }
            Addressing::Scaled {
                numerator,
                denominator,
            }
        } else {
            Addressing::Legacy
        };
        let mut ranges = Vec::with_capacity(2);
        if self.boundary_a != self.boundary_b {
            if let Some(last) = last.filter(|last| *last > 0) {
                // B is exclusive in a linear range, so N+1 includes sector
                // N. Use u64 for the comparison even when N is u32::MAX.
                let out_of_bounds = if self.boundary_b > self.boundary_a {
                    u64::from(self.boundary_b) > u64::from(last) + 1
                } else {
                    self.boundary_a > last
                };
                if out_of_bounds {
                    return Err(LogInfoError::SectorOutOfBounds);
                }
            }
            if self.boundary_b > self.boundary_a {
                ranges.push(self.boundary_a..self.boundary_b);
            } else {
                let last = last
                    .filter(|last| *last > 0)
                    .ok_or(LogInfoError::MissingSectorBound)?;
                let end = last.checked_add(1).ok_or(LogInfoError::SectorOutOfBounds)?;
                if self.boundary_b > 0 {
                    ranges.push(0..self.boundary_b);
                }
                ranges.push(self.boundary_a..end);
            }
        }
        let count = ranges
            .iter()
            .map(|range| u64::from(range.end - range.start))
            .sum();
        if count > max_sectors {
            return Err(LogInfoError::SectorLimitExceeded {
                count,
                limit: max_sectors,
            });
        }
        if matches!(addressing, Addressing::Legacy)
            && ranges.iter().any(|range| range.end > 0x1_0000)
        {
            return Err(LogInfoError::SectorOutOfBounds);
        }
        let plan = SectorPlan {
            system: self.id,
            ranges,
            count,
            addressing,
        };
        // Validate extremes before a caller starts a transfer. Scaled addresses
        // are monotonic for the admitted positive signed MulDiv operands.
        for range in &plan.ranges {
            plan.address(range.start)?;
            plan.address(range.end - 1)?;
        }
        Ok(plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Addressing {
    Legacy,
    Scaled { numerator: u32, denominator: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectorPlan {
    system: u8,
    ranges: Vec<Range<u32>>,
    count: u64,
    addressing: Addressing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorAddress {
    pub extension: u8,
    pub address: u32,
}

impl SectorPlan {
    pub fn system(&self) -> u8 {
        self.system
    }
    pub fn len(&self) -> u64 {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn ranges(&self) -> &[Range<u32>] {
        &self.ranges
    }
    /// Match Tune's order: reverse each range, in original range order.
    pub fn sectors(&self) -> impl Iterator<Item = u32> + '_ {
        self.ranges.iter().flat_map(|range| range.clone().rev())
    }
    pub fn address(&self, sector: u32) -> Result<SectorAddress, LogInfoError> {
        if !self.ranges.iter().any(|range| range.contains(&sector)) {
            return Err(LogInfoError::SectorOutOfBounds);
        }
        match self.addressing {
            Addressing::Legacy => {
                if sector > 0xffff {
                    return Err(LogInfoError::SectorOutOfBounds);
                }
                Ok(SectorAddress {
                    extension: 0x25,
                    address: (u32::from(self.system) << 24) | sector,
                })
            }
            Addressing::Scaled {
                numerator,
                denominator,
            } => {
                if sector > i32::MAX as u32
                    || numerator > i32::MAX as u32
                    || denominator > i32::MAX as u32
                {
                    return Err(LogInfoError::UnsupportedSignedAddress);
                }
                // Windows MulDiv rounds to nearest integer. With positive
                // operands, adding denominator/2 implements ties upward.
                let value = (u64::from(sector) * u64::from(numerator) + u64::from(denominator / 2))
                    / u64::from(denominator);
                if value > i32::MAX as u64 {
                    return Err(LogInfoError::UnsupportedSignedAddress);
                }
                Ok(SectorAddress {
                    extension: 0x80 + self.system,
                    address: value as u32,
                })
            }
        }
    }
}

fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn at_offset<'a>(
    record: Result<MoRecord<'a>, RecordError>,
    offset: usize,
) -> Result<MoRecord<'a>, LogInfoError> {
    record
        .map(|mut record| {
            record.offset += offset;
            record
        })
        .map_err(|mut error| {
            error.offset += offset;
            LogInfoError::Record(error)
        })
}
fn plain(record: MoRecord<'_>) -> Result<(), LogInfoError> {
    if record.header.encoding == 0 {
        Ok(())
    } else {
        Err(LogInfoError::UnsupportedEncoding {
            record_type: record.header.record_type,
            encoding: record.header.encoding,
        })
    }
}
fn minimum(record: MoRecord<'_>, needed: usize) -> Result<(), LogInfoError> {
    if record.payload.len() >= needed {
        Ok(())
    } else {
        Err(LogInfoError::ShortPayload {
            record_type: record.header.record_type,
            needed,
            available: record.payload.len(),
        })
    }
}
struct Budget {
    remaining: usize,
    limit: usize,
}
impl Budget {
    fn consume(&mut self) -> Result<(), LogInfoError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(LogInfoError::RecordLimitExceeded(self.limit))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogInfoError {
    Record(RecordError),
    MissingRoot,
    UnexpectedRoot(u32),
    TrailingData,
    UnsupportedEncoding {
        record_type: u32,
        encoding: u8,
    },
    ShortPayload {
        record_type: u32,
        needed: usize,
        available: usize,
    },
    DuplicateRecord {
        record_type: u32,
        index: u8,
    },
    InvalidSystemId(u8),
    RecordLimitExceeded(usize),
    MissingSectorBound,
    MissingAddressMapping,
    InvalidAddressMapping,
    SectorOutOfBounds,
    UnsupportedSignedAddress,
    SectorLimitExceeded {
        count: u64,
        limit: u64,
    },
}
impl From<RecordError> for LogInfoError {
    fn from(error: RecordError) -> Self {
        Self::Record(error)
    }
}
impl fmt::Display for LogInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Record(error) => error.fmt(f),
            Self::MissingRoot => f.write_str("logging info has no root record"),
            Self::UnexpectedRoot(kind) => write!(f, "expected M1 info record, found 0x{kind:08x}"),
            Self::TrailingData => f.write_str("logging info has bytes after its root"),
            Self::UnsupportedEncoding {
                record_type,
                encoding,
            } => write!(
                f,
                "M1 info record 0x{record_type:08x} has unsupported encoding {encoding}"
            ),
            Self::ShortPayload {
                record_type,
                needed,
                available,
            } => write!(
                f,
                "M1 info record 0x{record_type:08x} needs {needed} payload bytes, found {available}"
            ),
            Self::DuplicateRecord { record_type, index } => write!(
                f,
                "duplicate M1 info record 0x{record_type:08x}, index {index}"
            ),
            Self::InvalidSystemId(id) => write!(f, "M1 logging system {id} is outside 0..8"),
            Self::RecordLimitExceeded(limit) => {
                write!(f, "M1 logging info exceeds {limit} records")
            }
            Self::MissingSectorBound => {
                f.write_str("wrapped logging data has no usable last-sector bound")
            }
            Self::MissingAddressMapping => {
                f.write_str("logging device is missing sector address mapping fields")
            }
            Self::InvalidAddressMapping => {
                f.write_str("logging device has a zero sector mapping operand")
            }
            Self::SectorOutOfBounds => f.write_str("logging sector is outside the admitted range"),
            Self::UnsupportedSignedAddress => {
                f.write_str("sector address exceeds the supported positive signed MulDiv range")
            }
            Self::SectorLimitExceeded { count, limit } => write!(
                f,
                "logging plan has {count} sectors, exceeding limit {limit}"
            ),
        }
    }
}
impl std::error::Error for LogInfoError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: u32, index: u8, encoding: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"Mo".to_vec();
        bytes.extend_from_slice(&[index, encoding]);
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    fn system(id: u8, a: u32, b: u32) -> Vec<u8> {
        let mut payload = [0; 16];
        payload[8..12].copy_from_slice(&a.to_be_bytes());
        payload[12..16].copy_from_slice(&b.to_be_bytes());
        record(LOG_SYSTEM_RECORD_TYPE, id, 0, &payload)
    }
    fn device(last: u32, numerator: u32, denominator: u32) -> Vec<u8> {
        let mut payload = [0; 56];
        payload[16..20].copy_from_slice(&numerator.to_be_bytes());
        payload[40..44].copy_from_slice(&last.to_be_bytes());
        payload[44..48].copy_from_slice(&denominator.to_be_bytes());
        record(DEVICE_INFO_RECORD_TYPE, 0, 0, &payload)
    }
    fn root(children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        for child in children {
            payload.resize(payload.len().next_multiple_of(4), 0);
            payload.extend_from_slice(child);
        }
        record(INFO_RECORD_TYPE, 0, 0, &payload)
    }

    #[test]
    fn parses_only_direct_known_children_and_preserves_unknown_bytes() {
        let nested_system = system(7, 100, 200);
        let unknown = record(0x1234, 90, 20, &nested_system);
        let bytes = root(&[device(7, 3, 2), system(3, 5, 2), unknown]);
        let info = LogInfo::parse(&bytes).unwrap();
        assert_eq!(info.systems.len(), 1);
        assert_eq!(info.system(3).unwrap().boundary_a, 5);
        assert!(info.system(7).is_none());
        assert_eq!(info.other_records[0].header.encoding, 20);
        assert_eq!(info.other_records[0].payload, nested_system);
        assert_eq!(info.device.unwrap().address_numerator(), Some(3));
        assert!(info.permissions.is_none());
    }

    #[test]
    fn reads_nested_active_user_download_bits_and_shares_record_budget() {
        let mut payload = [0; 102];
        payload[64] = 1;
        payload[65] = 8; // An unrelated permission bit cannot grant download.
        payload[71] = 0x81;
        let permissions = record(PERMISSIONS_RECORD_TYPE, 0, 0, &payload);
        let user = record(ACTIVE_USER_RECORD_TYPE, 0, 0, &permissions);
        let bytes = root(&[user, system(0, 0, 0)]);
        let info = LogInfo::parse_with_limit(&bytes, 4).unwrap();
        let rights = info.permissions.unwrap();
        assert!(rights.can_download(0));
        assert!(!rights.can_download(1));
        assert!(rights.can_download(7));
        assert!(!rights.can_download(8));
        assert_eq!(
            LogInfo::parse_with_limit(&bytes, 3),
            Err(LogInfoError::RecordLimitExceeded(3))
        );
    }

    #[test]
    fn rejects_ambiguous_truncated_or_transformed_known_records() {
        let bytes = root(&[system(2, 1, 3), system(2, 1, 3)]);
        assert!(matches!(
            LogInfo::parse(&bytes),
            Err(LogInfoError::DuplicateRecord { .. })
        ));
        let bytes = root(&[record(LOG_SYSTEM_RECORD_TYPE, 0, 0, &[0; 15])]);
        assert!(matches!(
            LogInfo::parse(&bytes),
            Err(LogInfoError::ShortPayload { .. })
        ));
        let bytes = root(&[record(LOG_SYSTEM_RECORD_TYPE, 0, 20, &[0; 16])]);
        assert!(matches!(
            LogInfo::parse(&bytes),
            Err(LogInfoError::UnsupportedEncoding { .. })
        ));
        let bytes = root(&[system(8, 0, 0)]);
        assert_eq!(
            LogInfo::parse(&bytes),
            Err(LogInfoError::InvalidSystemId(8))
        );
        let bytes = root(&[system(0, 1, 3)]);
        for end in 0..bytes.len() {
            assert!(LogInfo::parse(&bytes[..end]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(LogInfo::parse(&trailing), Err(LogInfoError::TrailingData));
    }

    #[test]
    fn plans_wrapped_sectors_lazily_in_vendor_order_and_rounds_scaled_addresses() {
        let bytes = root(&[device(7, 3, 2), system(3, 5, 2)]);
        let info = LogInfo::parse(&bytes).unwrap();
        let plan = info
            .system(3)
            .unwrap()
            .plan(info.device.as_ref(), 5)
            .unwrap();
        assert_eq!(plan.ranges(), &[0..2, 5..8]);
        assert_eq!(plan.sectors().collect::<Vec<_>>(), [1, 0, 7, 6, 5]);
        assert_eq!(plan.len(), 5);
        assert_eq!(
            plan.address(5).unwrap(),
            SectorAddress {
                extension: 0x83,
                address: 8
            }
        );
        assert_eq!(plan.address(4), Err(LogInfoError::SectorOutOfBounds));
        assert_eq!(
            info.system(3).unwrap().plan(info.device.as_ref(), 4),
            Err(LogInfoError::SectorLimitExceeded { count: 5, limit: 4 })
        );
    }

    #[test]
    fn handles_linear_legacy_and_empty_plans_without_materializing_sectors() {
        let bytes = root(&[system(1, 0xfffe, 0x1_0000), system(2, 4, 4)]);
        let info = LogInfo::parse(&bytes).unwrap();
        let plan = info.system(1).unwrap().plan(None, 2).unwrap();
        assert_eq!(plan.sectors().collect::<Vec<_>>(), [0xffff, 0xfffe]);
        assert_eq!(
            plan.address(0xffff).unwrap(),
            SectorAddress {
                extension: 0x25,
                address: 0x0100_ffff
            }
        );
        assert!(info.system(2).unwrap().plan(None, 0).unwrap().is_empty());
        let bytes = root(&[system(0, 0, 0x1_0001)]);
        let info = LogInfo::parse(&bytes).unwrap();
        assert_eq!(
            info.system(0).unwrap().plan(None, u64::MAX),
            Err(LogInfoError::SectorOutOfBounds)
        );
    }

    #[test]
    fn rejects_missing_bounds_bad_divisors_and_signed_muldiv_overflow() {
        let bytes = root(&[system(0, 5, 2)]);
        let info = LogInfo::parse(&bytes).unwrap();
        assert_eq!(
            info.system(0).unwrap().plan(None, 10),
            Err(LogInfoError::MissingSectorBound)
        );
        for (numerator, denominator, expected) in [
            (3, 0, LogInfoError::InvalidAddressMapping),
            (u32::MAX, 1, LogInfoError::UnsupportedSignedAddress),
            (i32::MAX as u32, 1, LogInfoError::UnsupportedSignedAddress),
        ] {
            let bytes = root(&[device(7, numerator, denominator), system(0, 2, 3)]);
            let info = LogInfo::parse(&bytes).unwrap();
            assert_eq!(
                info.system(0).unwrap().plan(info.device.as_ref(), 10),
                Err(expected)
            );
        }
        let bytes = root(&[device(u32::MAX, 1, 1), system(0, 5, 2)]);
        let info = LogInfo::parse(&bytes).unwrap();
        assert_eq!(
            info.system(0).unwrap().plan(info.device.as_ref(), u64::MAX),
            Err(LogInfoError::SectorOutOfBounds)
        );
    }
    #[test]
    fn linear_exclusive_end_may_be_one_past_last_sector() {
        let bytes = root(&[device(9, 3, 2), system(3, 8, 10)]);
        let info = LogInfo::parse(&bytes).unwrap();
        let plan = info
            .system(3)
            .unwrap()
            .plan(info.device.as_ref(), 2)
            .unwrap();
        assert_eq!(plan.sectors().collect::<Vec<_>>(), [9, 8]);
        assert_eq!(
            plan.address(9).unwrap(),
            SectorAddress {
                extension: 0x83,
                address: 14
            }
        );
        assert_eq!(plan.address(10), Err(LogInfoError::SectorOutOfBounds));

        let bytes = root(&[device(9, 1, 1), system(3, 8, 11)]);
        let info = LogInfo::parse(&bytes).unwrap();
        assert_eq!(
            info.system(3).unwrap().plan(info.device.as_ref(), 3),
            Err(LogInfoError::SectorOutOfBounds)
        );
    }

    #[test]
    fn empty_boundaries_ignore_unused_invalid_mapping_and_bounds() {
        let bytes = root(&[device(9, 0, 0), system(3, u32::MAX, u32::MAX)]);
        let info = LogInfo::parse(&bytes).unwrap();
        let plan = info
            .system(3)
            .unwrap()
            .plan(info.device.as_ref(), 0)
            .unwrap();
        assert!(plan.is_empty());
        assert_eq!(plan.sectors().next(), None);
    }
}

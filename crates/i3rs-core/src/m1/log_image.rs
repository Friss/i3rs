//! Structural preflight for an uncompressed M1 logged-data image (`.m1dli`).
//!
//! This recognizes the outer Mo record observed in M1 Tune 1.4.0.0302. It does
//! not decode logging packets, validate ECU permissions or checksums, convert
//! samples to an `.ld` file, or unpack a `.m1dli-archive`. Successful preflight
//! establishes only that the supplied bytes contain one complete image record.

use std::fmt;

/// Size of a Mo record header, including its two-byte magic.
pub const MO_HEADER_SIZE: usize = 12;
/// Root record type checked by M1 Tune's raw logged-data-image reader.
pub const LOG_IMAGE_RECORD_TYPE: u32 = 0x0100_0005;
pub const PROPERTIES_RECORD_TYPE: u32 = 0x0100_0009;
pub const PROPERTY_RECORD_TYPE: u32 = 0x0100_000a;
pub const STRING_RECORD_TYPE: u32 = 0x0001_0000;
/// Total header budget for the shallow metadata inspector, including the root.
pub const DEFAULT_METADATA_RECORD_LIMIT: usize = 4096;

/// The observed fixed-size Mo header. The two control bytes remain opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoRecordHeader {
    pub flags: u8,
    pub encoding: u8,
    pub record_type: u32,
    pub payload_len: u32,
}

impl MoRecordHeader {
    /// Read the leading header without interpreting its payload or control bytes.
    /// All multi-byte header fields use network (big-endian) byte order.
    pub fn parse(bytes: &[u8]) -> Result<Self, LogImageError> {
        let header = bytes
            .get(..MO_HEADER_SIZE)
            .ok_or(LogImageError::TruncatedHeader {
                available: bytes.len(),
            })?;
        if &header[..2] != b"Mo" {
            return Err(LogImageError::InvalidMagic);
        }
        Ok(Self {
            flags: header[2],
            encoding: header[3],
            record_type: u32::from_be_bytes(header[4..8].try_into().unwrap()),
            payload_len: u32::from_be_bytes(header[8..12].try_into().unwrap()),
        })
    }
}

/// One structurally complete raw logged-data image, borrowing its opaque payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLogImage<'a> {
    pub header: MoRecordHeader,
    pub payload: &'a [u8],
}

impl<'a> RawLogImage<'a> {
    /// Require exactly one complete root record, rejecting truncated payloads and
    /// trailing bytes. No allocation is based on the untrusted declared length.
    /// Nested payload records are deliberately not interpreted by this preflight.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, LogImageError> {
        let header = MoRecordHeader::parse(bytes)?;
        if header.record_type != LOG_IMAGE_RECORD_TYPE {
            return Err(LogImageError::UnexpectedRecordType {
                actual: header.record_type,
            });
        }
        let payload_len =
            usize::try_from(header.payload_len).map_err(|_| LogImageError::LengthOverflow)?;
        let end = MO_HEADER_SIZE
            .checked_add(payload_len)
            .ok_or(LogImageError::LengthOverflow)?;
        if end > bytes.len() {
            return Err(LogImageError::TruncatedPayload {
                declared: header.payload_len,
                available: bytes.len() - MO_HEADER_SIZE,
            });
        }
        if end < bytes.len() {
            return Err(LogImageError::TrailingData {
                bytes: bytes.len() - end,
            });
        }
        Ok(Self {
            header,
            payload: &bytes[MO_HEADER_SIZE..end],
        })
    }

    /// Inspect only known, untransformed properties containers. Unknown records
    /// and property fields remain borrowed bytes; duplicate properties retain
    /// their original order. This never decrypts or decompresses a record.
    pub fn inspect_metadata(&self) -> Result<MetadataInspection<'a>, RecordError> {
        self.inspect_metadata_with_limit(DEFAULT_METADATA_RECORD_LIMIT)
    }

    /// As [`Self::inspect_metadata`], with a shared budget for all examined
    /// headers. Traversal has a fixed maximum of three child levels: root,
    /// properties group, property fields. No unknown record is traversed.
    pub fn inspect_metadata_with_limit(
        &self,
        max_records: usize,
    ) -> Result<MetadataInspection<'a>, RecordError> {
        let mut budget = RecordBudget::new(max_records);
        budget.consume(0)?;
        let mut inspection = MetadataInspection {
            opaque_root: self.header.encoding != 0,
            groups: Vec::new(),
            other_root_records: Vec::new(),
            records_examined: 1,
        };
        if inspection.opaque_root {
            return Ok(inspection);
        }
        for item in MoRecordIter::at(self.payload, MO_HEADER_SIZE, max_records) {
            let record = item?;
            budget.consume(record.offset)?;
            if record.header.record_type == PROPERTIES_RECORD_TYPE {
                inspection.groups.push(inspect_group(record, &mut budget)?);
            } else {
                inspection.other_root_records.push(record);
            }
        }
        inspection.records_examined = max_records - budget.remaining;
        Ok(inspection)
    }
}

/// A complete borrowed record. `offset` is relative to the input given to the
/// iterator, or relative to the raw image when returned by metadata inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoRecord<'a> {
    pub offset: usize,
    pub header: MoRecordHeader,
    pub payload: &'a [u8],
}

/// Bounded direct-record iterator. It consumes only the supplied span, never
/// searches for a later signature, and stops permanently after its first error.
/// Child headers start at four-byte stream boundaries. Inter-record padding is
/// skipped without interpreting its bytes and is not part of either payload.
/// A final record may end at the exact container end without trailing padding;
/// a partial padding span before that end is rejected.
/// It does not infer whether those bytes are a container: callers must not feed
/// it encrypted/compressed or otherwise unknown payloads as plain records.
pub struct MoRecordIter<'a> {
    remaining: &'a [u8],
    offset: usize,
    records_left: usize,
    limit: usize,
    failed: bool,
}

impl<'a> MoRecordIter<'a> {
    pub fn new(bytes: &'a [u8], max_records: usize) -> Self {
        Self::at(bytes, 0, max_records)
    }

    fn at(bytes: &'a [u8], offset: usize, max_records: usize) -> Self {
        Self {
            remaining: bytes,
            offset,
            records_left: max_records,
            limit: max_records,
            failed: false,
        }
    }

    fn next_record(&mut self) -> Result<MoRecord<'a>, LogImageError> {
        if self.records_left == 0 {
            return Err(LogImageError::RecordLimitExceeded { limit: self.limit });
        }
        let header = MoRecordHeader::parse(self.remaining)?;
        let payload_len =
            usize::try_from(header.payload_len).map_err(|_| LogImageError::LengthOverflow)?;
        let end = MO_HEADER_SIZE
            .checked_add(payload_len)
            .ok_or(LogImageError::LengthOverflow)?;
        if end > self.remaining.len() {
            return Err(LogImageError::TruncatedPayload {
                declared: header.payload_len,
                available: self.remaining.len() - MO_HEADER_SIZE,
            });
        }
        let next_offset = self
            .offset
            .checked_add(end)
            .ok_or(LogImageError::LengthOverflow)?;
        let record = MoRecord {
            offset: self.offset,
            header,
            payload: &self.remaining[MO_HEADER_SIZE..end],
        };
        self.remaining = &self.remaining[end..];
        self.offset = next_offset;
        self.records_left -= 1;
        Ok(record)
    }
}

impl<'a> Iterator for MoRecordIter<'a> {
    type Item = Result<MoRecord<'a>, RecordError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining.is_empty() {
            return None;
        }
        // Tune's CBaseStream::Align (0xac4fe0) seeks to the next absolute
        // four-byte boundary before the generic reader requests another header.
        // The header's declared payload length excludes this alignment gap.
        let padding = (4 - self.offset % 4) % 4;
        if self.remaining.len() < padding {
            self.failed = true;
            return Some(Err(RecordError {
                offset: self.offset,
                kind: LogImageError::TruncatedPadding {
                    expected: padding,
                    available: self.remaining.len(),
                },
            }));
        }
        let Some(next_offset) = self.offset.checked_add(padding) else {
            self.failed = true;
            return Some(Err(RecordError {
                offset: self.offset,
                kind: LogImageError::LengthOverflow,
            }));
        };
        self.remaining = &self.remaining[padding..];
        self.offset = next_offset;
        if self.remaining.is_empty() {
            return None;
        }
        let offset = self.offset;
        Some(self.next_record().map_err(|kind| {
            self.failed = true;
            RecordError { offset, kind }
        }))
    }
}

impl std::iter::FusedIterator for MoRecordIter<'_> {}

/// Shallow metadata inspection. A transformed root has `opaque_root = true`
/// and no interpreted children. Non-metadata root records are retained intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataInspection<'a> {
    pub opaque_root: bool,
    pub groups: Vec<MetadataGroup<'a>>,
    pub other_root_records: Vec<MoRecord<'a>>,
    pub records_examined: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGroup<'a> {
    /// The group's header control bytes are preserved without assigning names.
    pub record: MoRecord<'a>,
    pub contents: MetadataGroupContents<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataGroupContents<'a> {
    /// A transformation encoding prevents examination of this container.
    Opaque,
    Plain {
        properties: Vec<MetadataProperty<'a>>,
        other_records: Vec<MoRecord<'a>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataProperty<'a> {
    pub record: MoRecord<'a>,
    pub fields: PropertyFields<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyFields<'a> {
    Opaque,
    /// All fields are retained, including unknown/duplicate field indices.
    Plain(Vec<MoRecord<'a>>),
}

impl<'a> MetadataProperty<'a> {
    /// Tune reads the first child as the property name. Expose it only when it
    /// is a plain string, preserving NULs and invalid UTF-8. Field positions are
    /// distinct from the opaque byte-2 header control.
    pub fn name(&self) -> Option<&'a [u8]> {
        let field = self.field_records().next()?;
        (field.header.record_type == STRING_RECORD_TYPE && field.header.encoding == 0)
            .then_some(field.payload)
    }

    /// Tune reads the second child as the value. Its type, encoding, and bytes
    /// remain raw; string/numeric interpretation is left to the caller.
    pub fn value(&self) -> Option<&MoRecord<'a>> {
        self.field_records().nth(1)
    }

    pub fn field_records(&self) -> impl Iterator<Item = &MoRecord<'a>> + '_ {
        let fields = match &self.fields {
            PropertyFields::Opaque => &[][..],
            PropertyFields::Plain(fields) => fields.as_slice(),
        };
        fields.iter()
    }
}

fn inspect_group<'a>(
    record: MoRecord<'a>,
    budget: &mut RecordBudget,
) -> Result<MetadataGroup<'a>, RecordError> {
    if record.header.encoding != 0 {
        return Ok(MetadataGroup {
            record,
            contents: MetadataGroupContents::Opaque,
        });
    }
    let mut properties = Vec::new();
    let mut other_records = Vec::new();
    for item in MoRecordIter::at(record.payload, record.offset + MO_HEADER_SIZE, budget.limit) {
        let child = item?;
        budget.consume(child.offset)?;
        if child.header.record_type == PROPERTY_RECORD_TYPE {
            let fields = if child.header.encoding != 0 {
                PropertyFields::Opaque
            } else {
                let mut fields = Vec::new();
                for field in
                    MoRecordIter::at(child.payload, child.offset + MO_HEADER_SIZE, budget.limit)
                {
                    let field = field?;
                    budget.consume(field.offset)?;
                    fields.push(field);
                }
                PropertyFields::Plain(fields)
            };
            properties.push(MetadataProperty {
                record: child,
                fields,
            });
        } else {
            other_records.push(child);
        }
    }
    Ok(MetadataGroup {
        record,
        contents: MetadataGroupContents::Plain {
            properties,
            other_records,
        },
    })
}

struct RecordBudget {
    limit: usize,
    remaining: usize,
}

impl RecordBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: limit,
        }
    }

    fn consume(&mut self, offset: usize) -> Result<(), RecordError> {
        self.remaining = self.remaining.checked_sub(1).ok_or(RecordError {
            offset,
            kind: LogImageError::RecordLimitExceeded { limit: self.limit },
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordError {
    pub offset: usize,
    pub kind: LogImageError,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mo record at byte {}: {}", self.offset, self.kind)
    }
}

impl std::error::Error for RecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogImageError {
    TruncatedHeader { available: usize },
    InvalidMagic,
    UnexpectedRecordType { actual: u32 },
    LengthOverflow,
    TruncatedPayload { declared: u32, available: usize },
    TruncatedPadding { expected: usize, available: usize },
    TrailingData { bytes: usize },
    RecordLimitExceeded { limit: usize },
}

impl fmt::Display for LogImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { available } => write!(
                f,
                "M1 image header is truncated: expected {MO_HEADER_SIZE} bytes, found {available}"
            ),
            Self::InvalidMagic => {
                f.write_str("M1 image does not begin with the Mo record signature")
            }
            Self::UnexpectedRecordType { actual } => write!(
                f,
                "Mo record type 0x{actual:08x} is not an M1 logged-data image"
            ),
            Self::LengthOverflow => {
                f.write_str("M1 image declared length exceeds the addressable size")
            }
            Self::TruncatedPayload {
                declared,
                available,
            } => write!(
                f,
                "M1 image payload is truncated: declared {declared} bytes, found {available}"
            ),
            Self::TrailingData { bytes } => {
                write!(f, "M1 image contains {bytes} bytes after its root record")
            }
            Self::TruncatedPadding {
                expected,
                available,
            } => write!(
                f,
                "Mo alignment padding is truncated: expected {expected} bytes, found {available}"
            ),
            Self::RecordLimitExceeded { limit } => {
                write!(f, "metadata inspection exceeds its {limit}-record limit")
            }
        }
    }
}

impl std::error::Error for LogImageError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic bytes encode the header layout observed in Tune's reader. These
    // are structural fixtures, not captured ECU images or conversion fixtures.
    const IMAGE: &[u8] = &[
        b'M', b'o', 0x12, 0x34, 0x01, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x04, 0xde, 0xad, 0xbe,
        0xef,
    ];

    #[test]
    fn reads_network_order_fields_and_preserves_opaque_bytes() {
        let image = RawLogImage::parse(IMAGE).unwrap();
        assert_eq!(image.header.flags, 0x12);
        assert_eq!(image.header.encoding, 0x34);
        assert_eq!(image.header.record_type, LOG_IMAGE_RECORD_TYPE);
        assert_eq!(image.header.payload_len, 4);
        assert_eq!(image.payload, &IMAGE[MO_HEADER_SIZE..]);
        assert_eq!(image.payload.as_ptr(), IMAGE[MO_HEADER_SIZE..].as_ptr());
    }

    #[test]
    fn rejects_every_truncated_header_and_payload_prefix() {
        for length in 0..MO_HEADER_SIZE {
            assert_eq!(
                RawLogImage::parse(&IMAGE[..length]),
                Err(LogImageError::TruncatedHeader { available: length })
            );
        }
        for length in MO_HEADER_SIZE..IMAGE.len() {
            assert_eq!(
                RawLogImage::parse(&IMAGE[..length]),
                Err(LogImageError::TruncatedPayload {
                    declared: 4,
                    available: length - MO_HEADER_SIZE,
                })
            );
        }
    }

    #[test]
    fn rejects_wrong_magic_record_type_and_trailing_data() {
        let mut bytes = IMAGE.to_vec();
        bytes[0] = b'm';
        assert_eq!(RawLogImage::parse(&bytes), Err(LogImageError::InvalidMagic));
        bytes[0] = b'M';
        bytes[7] = 6;
        assert_eq!(
            RawLogImage::parse(&bytes),
            Err(LogImageError::UnexpectedRecordType {
                actual: 0x0100_0006,
            })
        );
        bytes[7] = 5;
        bytes.push(0);
        assert_eq!(
            RawLogImage::parse(&bytes),
            Err(LogImageError::TrailingData { bytes: 1 })
        );
    }

    #[test]
    fn rejects_maximum_declared_size_without_allocating_or_wrapping() {
        let mut bytes = IMAGE[..MO_HEADER_SIZE].to_vec();
        bytes[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        let result = RawLogImage::parse(&bytes);
        if usize::BITS == 32 {
            assert_eq!(result, Err(LogImageError::LengthOverflow));
        } else {
            assert_eq!(
                result,
                Err(LogImageError::TruncatedPayload {
                    declared: u32::MAX,
                    available: 0,
                })
            );
        }
    }

    #[test]
    fn empty_payload_only_establishes_outer_record_structure() {
        let bytes = [b'M', b'o', 0, 0, 1, 0, 0, 5, 0, 0, 0, 0];
        assert!(RawLogImage::parse(&bytes).unwrap().payload.is_empty());
    }

    fn record(record_type: u32, flags: u8, encoding: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"Mo".to_vec();
        bytes.extend_from_slice(&[flags, encoding]);
        bytes.extend_from_slice(&record_type.to_be_bytes());
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn sequence(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            bytes.extend_from_slice(record);
        }
        bytes
    }

    #[test]
    fn direct_iteration_stays_borrowed_and_fuses_after_malformed_tail() {
        let mut bytes = record(0x1234_5678, 8, 20, b"opaque");
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        let second_offset = bytes.len();
        bytes.extend_from_slice(&record(0x0102_0304, 7, 0, &[]));
        let invalid_offset = bytes.len();
        bytes.push(b'M');
        let mut records = MoRecordIter::new(&bytes, 10);
        let first = records.next().unwrap().unwrap();
        assert_eq!(first.offset, 0);
        assert_eq!(first.header.encoding, 20);
        assert_eq!(first.payload, b"opaque");
        assert_eq!(first.payload.as_ptr(), bytes[12..].as_ptr());
        assert_eq!(records.next().unwrap().unwrap().offset, second_offset);
        assert_eq!(
            records.next().unwrap(),
            Err(RecordError {
                offset: invalid_offset,
                kind: LogImageError::TruncatedHeader { available: 1 },
            })
        );
        assert!(records.next().is_none());
        assert!(records.next().is_none());
    }

    #[test]
    fn direct_iteration_rejects_bad_magic_without_resynchronizing() {
        let mut bytes = b"not a record".to_vec();
        bytes.extend_from_slice(&record(STRING_RECORD_TYPE, 0, 0, b"later"));
        let mut records = MoRecordIter::new(&bytes, 2);
        assert_eq!(
            records.next().unwrap(),
            Err(RecordError {
                offset: 0,
                kind: LogImageError::InvalidMagic,
            })
        );
        assert!(records.next().is_none());
    }

    #[test]
    fn direct_iteration_enforces_record_limit_without_allocating() {
        let one = record(STRING_RECORD_TYPE, 0, 0, &[]);
        let bytes = [one.clone(), one].concat();
        let mut records = MoRecordIter::new(&bytes, 1);
        assert!(records.next().unwrap().is_ok());
        assert_eq!(
            records.next().unwrap(),
            Err(RecordError {
                offset: MO_HEADER_SIZE,
                kind: LogImageError::RecordLimitExceeded { limit: 1 },
            })
        );
        assert!(records.next().is_none());
        assert!(MoRecordIter::new(&[], 0).next().is_none());
    }

    #[test]
    fn child_alignment_excludes_padding_from_borrowed_payloads() {
        for length in 0..8 {
            let payload = vec![0x51; length];
            let mut bytes = record(STRING_RECORD_TYPE, 0, 0, &payload);
            let unpadded_end = bytes.len();
            // The reader seeks over padding; zero content is not required.
            bytes.resize(unpadded_end.next_multiple_of(4), 0xa5);
            let second_offset = bytes.len();
            bytes.extend_from_slice(&record(STRING_RECORD_TYPE, 0, 0, b"last"));
            let mut records = MoRecordIter::new(&bytes, 2);
            let first = records.next().unwrap().unwrap();
            assert_eq!(first.payload, payload);
            assert_eq!(first.payload.as_ptr(), bytes[12..].as_ptr());
            let second = records.next().unwrap().unwrap();
            assert_eq!(second.offset, second_offset);
            assert_eq!(second.payload, b"last");
            assert!(records.next().is_none());

            // Parent lengths can end immediately after the final payload, or
            // include its complete alignment gap. Neither creates a record.
            assert_eq!(MoRecordIter::new(&bytes[..unpadded_end], 1).count(), 1);
            let mut padded = MoRecordIter::new(&bytes[..second_offset], 1);
            assert!(padded.next().unwrap().is_ok());
            assert!(padded.next().is_none());
        }
    }

    #[test]
    fn incomplete_alignment_fails_without_reading_a_later_header() {
        let first = record(STRING_RECORD_TYPE, 0, 0, b"x");
        for available in 1..3 {
            let mut bytes = first.clone();
            bytes.resize(bytes.len() + available, 0);
            let mut records = MoRecordIter::new(&bytes, 2);
            assert!(records.next().unwrap().is_ok());
            assert_eq!(
                records.next().unwrap(),
                Err(RecordError {
                    offset: 13,
                    kind: LogImageError::TruncatedPadding {
                        expected: 3,
                        available,
                    },
                })
            );
            assert!(records.next().is_none());
        }
        let bytes = [first, record(STRING_RECORD_TYPE, 0, 0, b"later")].concat();
        let mut records = MoRecordIter::new(&bytes, 2);
        assert!(records.next().unwrap().is_ok());
        assert_eq!(
            records.next().unwrap(),
            Err(RecordError {
                offset: 16,
                kind: LogImageError::InvalidMagic,
            })
        );
        assert!(records.next().is_none());
    }

    #[test]
    fn nested_alignment_cannot_consume_a_sibling_containers_bytes() {
        let mut fields = record(STRING_RECORD_TYPE, 0, 0, b"x");
        fields.extend_from_slice(&[0, 0]); // Only two of the required three.
        let property = record(PROPERTY_RECORD_TYPE, 0, 0, &fields);
        let group = record(PROPERTIES_RECORD_TYPE, 0, 0, &property);
        let sibling = record(0x9988_7766, 0, 0, b"outside the property");
        let image = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &sequence(&[group, sibling]));
        assert_eq!(
            RawLogImage::parse(&image).unwrap().inspect_metadata(),
            Err(RecordError {
                offset: 49,
                kind: LogImageError::TruncatedPadding {
                    expected: 3,
                    available: 2,
                },
            })
        );
    }

    #[test]
    #[ignore = "requires local M1 logged-data image; set I3RS_M1_IMAGE"]
    fn captured_image_preserves_exact_aligned_record_spans() {
        // The private ECU image is kept outside the checked-in test fixtures.
        let path = std::env::var_os("I3RS_M1_IMAGE")
            .expect("set I3RS_M1_IMAGE to the local M1 logged-data image");
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len(), 3_259_648);
        let image = RawLogImage::parse(&bytes).unwrap();
        let root_children = MoRecordIter::at(image.payload, MO_HEADER_SIZE, 8)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            root_children
                .iter()
                .map(|r| (r.offset, r.header.record_type, r.header.payload_len))
                .collect::<Vec<_>>(),
            [
                (0x00000c, PROPERTIES_RECORD_TYPE, 180),
                (0x0000cc, PROPERTIES_RECORD_TYPE, 352),
                (0x000238, 0x0100_000e, 3_012_684),
                (0x2dfa90, 0x0200_0020, 5_236),
                (0x2e0f10, 0x0200_0046, 178_144),
                (0x30c6fc, 0x0201_0012, 252),
                (0x30c804, 0x0101_0002, 62_704),
            ]
        );
        let metadata = image.inspect_metadata().unwrap();
        assert_eq!(metadata.groups.len(), 2);
        let MetadataGroupContents::Plain { properties, .. } = &metadata.groups[0].contents else {
            panic!("expected plain properties");
        };
        assert_eq!(
            properties
                .iter()
                .map(|p| p.record.offset)
                .collect::<Vec<_>>(),
            [24, 76, 136]
        );
        assert_eq!(properties[0].name(), Some(b"version".as_slice()));
        assert_eq!(properties[0].value().unwrap().offset, 56);
        assert_eq!(properties[0].value().unwrap().payload.len(), 5);
        assert_eq!(properties[1].name(), Some(b"device.serial".as_slice()));
        assert_eq!(properties[1].value().unwrap().offset, 116);
        assert_eq!(properties[2].name(), Some(b"retrieved.utc".as_slice()));
        assert_eq!(properties[2].value().unwrap().offset, 176);
        assert_eq!(properties[2].value().unwrap().payload.len(), 16);
        assert_eq!(metadata.records_examined, 38);
    }

    #[test]
    fn metadata_preserves_duplicate_properties_raw_values_and_name_bytes() {
        let name = record(STRING_RECORD_TYPE, 42, 0, &[0xff, 0, 0x80]);
        let value = record(0xdead_beef, 7, 11, b"transformed value");
        let extra = record(0x1111_2222, 9, 0, b"unknown field contents");
        let fields = sequence(&[name, value, extra]);
        let property = record(PROPERTY_RECORD_TYPE, 0, 0, &fields);
        let group = record(
            PROPERTIES_RECORD_TYPE,
            1,
            0,
            &sequence(&[property.clone(), property]),
        );
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &group);
        let image = RawLogImage::parse(&bytes).unwrap();
        let metadata = image.inspect_metadata().unwrap();
        assert!(!metadata.opaque_root);
        assert_eq!(metadata.groups[0].record.header.flags, 1);
        let MetadataGroupContents::Plain { properties, .. } = &metadata.groups[0].contents else {
            panic!("expected plain group");
        };
        assert_eq!(properties.len(), 2);
        for property in properties {
            // Field position, not header byte 2, selects name and value.
            assert_eq!(property.name(), Some(&[0xff, 0, 0x80][..]));
            let value = property.value().unwrap();
            assert_eq!(value.header.record_type, 0xdead_beef);
            assert_eq!(value.header.flags, 7);
            assert_eq!(value.header.encoding, 11);
            assert_eq!(value.payload, b"transformed value");
            assert_eq!(property.field_records().count(), 3);
            assert_eq!(
                property.field_records().nth(2).unwrap().payload,
                b"unknown field contents"
            );
        }
        assert_eq!(metadata.records_examined, 10);
    }

    #[test]
    fn transformed_root_group_and_property_are_explicitly_opaque() {
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 30, b"not plain Mo records");
        let metadata = RawLogImage::parse(&bytes)
            .unwrap()
            .inspect_metadata()
            .unwrap();
        assert!(metadata.opaque_root);
        assert_eq!(metadata.records_examined, 1);
        assert!(metadata.groups.is_empty());

        let encoded_group = record(PROPERTIES_RECORD_TYPE, 0, 10, b"encrypted");
        let encoded_property = record(PROPERTY_RECORD_TYPE, 0, 50, b"encrypted property");
        let plain_group = record(PROPERTIES_RECORD_TYPE, 1, 0, &encoded_property);
        let bytes = record(
            LOG_IMAGE_RECORD_TYPE,
            0,
            0,
            &sequence(&[encoded_group, plain_group]),
        );
        let metadata = RawLogImage::parse(&bytes)
            .unwrap()
            .inspect_metadata()
            .unwrap();
        assert_eq!(metadata.groups[0].contents, MetadataGroupContents::Opaque);
        let MetadataGroupContents::Plain { properties, .. } = &metadata.groups[1].contents else {
            panic!("expected plain group");
        };
        assert_eq!(properties[0].fields, PropertyFields::Opaque);
        assert!(properties[0].name().is_none());
        assert!(properties[0].value().is_none());
        assert_eq!(metadata.records_examined, 4);
    }

    #[test]
    fn unknown_root_and_group_records_are_retained_without_traversal() {
        let unknown = record(0x9988_7766, 3, 0, b"not a nested record");
        let group = record(PROPERTIES_RECORD_TYPE, 0, 0, &unknown);
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &sequence(&[unknown, group]));
        let metadata = RawLogImage::parse(&bytes)
            .unwrap()
            .inspect_metadata()
            .unwrap();
        assert_eq!(metadata.other_root_records.len(), 1);
        assert_eq!(
            metadata.other_root_records[0].payload,
            b"not a nested record"
        );
        let MetadataGroupContents::Plain {
            properties,
            other_records,
        } = &metadata.groups[0].contents
        else {
            panic!("expected plain group");
        };
        assert!(properties.is_empty());
        assert_eq!(other_records.len(), 1);
        assert_eq!(other_records[0].header.record_type, 0x9988_7766);
        assert_eq!(metadata.records_examined, 4);
    }

    #[test]
    fn nested_record_cannot_consume_bytes_from_a_sibling_container() {
        let mut malformed_field = record(STRING_RECORD_TYPE, 0, 0, &[]);
        malformed_field[8..12].copy_from_slice(&8u32.to_be_bytes());
        let property = record(PROPERTY_RECORD_TYPE, 0, 0, &malformed_field);
        let group = record(PROPERTIES_RECORD_TYPE, 0, 0, &property);
        let sibling = record(0x9988_7766, 0, 0, b"bytes outside the property");
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &sequence(&[group, sibling]));
        assert_eq!(
            RawLogImage::parse(&bytes).unwrap().inspect_metadata(),
            Err(RecordError {
                offset: MO_HEADER_SIZE * 3,
                kind: LogImageError::TruncatedPayload {
                    declared: 8,
                    available: 0,
                },
            })
        );
    }

    #[test]
    fn metadata_budget_is_shared_across_nested_containers() {
        let name = record(STRING_RECORD_TYPE, 0, 0, b"name");
        let value = record(STRING_RECORD_TYPE, 0, 0, b"value");
        let property = record(PROPERTY_RECORD_TYPE, 0, 0, &sequence(&[name, value]));
        let group = record(PROPERTIES_RECORD_TYPE, 0, 0, &property);
        let bytes = record(LOG_IMAGE_RECORD_TYPE, 0, 0, &group);
        let image = RawLogImage::parse(&bytes).unwrap();
        assert_eq!(
            image.inspect_metadata_with_limit(4).unwrap_err().kind,
            LogImageError::RecordLimitExceeded { limit: 4 }
        );
        assert_eq!(
            image
                .inspect_metadata_with_limit(5)
                .unwrap()
                .records_examined,
            5
        );
        assert_eq!(
            image.inspect_metadata_with_limit(0),
            Err(RecordError {
                offset: 0,
                kind: LogImageError::RecordLimitExceeded { limit: 0 },
            })
        );
    }
}

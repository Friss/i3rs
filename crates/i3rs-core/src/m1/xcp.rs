//! XCP/UDP transport framing used by M1 Tune.
//!
//! The four-byte envelope contains a little-endian payload length and counter.
//! The decoder can inspect concatenated envelopes and validates all supplied
//! bytes. CONNECT fields are decoded separately; neither framing nor advertised
//! resources establish permission to retrieve or convert a logging system.

use std::fmt;

pub const HEADER_LENGTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    EmptyDatagram,
    TruncatedHeader {
        offset: usize,
        remaining: usize,
    },
    TruncatedPayload {
        offset: usize,
        declared: usize,
        remaining: usize,
    },
    PayloadTooLarge {
        length: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDatagram => write!(f, "Empty XCP datagram"),
            Self::TruncatedHeader { offset, remaining } => write!(
                f,
                "Truncated XCP header at byte {offset}: {remaining} of {HEADER_LENGTH} bytes"
            ),
            Self::TruncatedPayload {
                offset,
                declared,
                remaining,
            } => write!(
                f,
                "Truncated XCP payload at byte {offset}: {remaining} of {declared} bytes"
            ),
            Self::PayloadTooLarge { length } => write!(
                f,
                "XCP payload has {length} bytes; the transport length is limited to {}",
                u16::MAX
            ),
        }
    }
}

impl std::error::Error for FrameError {}

/// A borrowed frame. The counter is transport sequence information, not an
/// indication that a command succeeded or that this is a reply to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub counter: u16,
    pub payload: &'a [u8],
}

/// Encode one frame, rejecting lengths that would otherwise wrap a u16.
pub fn encode_frame(counter: u16, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    let length = u16::try_from(payload.len()).map_err(|_| FrameError::PayloadTooLarge {
        length: payload.len(),
    })?;
    let mut bytes = Vec::with_capacity(HEADER_LENGTH + payload.len());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&counter.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Decode all frames in one datagram without copying payloads.
///
/// A malformed final frame fails the entire datagram. In particular, a valid
/// prefix cannot hide a truncated response or trailing partial header.
pub fn decode_datagram(bytes: &[u8]) -> Result<Vec<Frame<'_>>, FrameError> {
    if bytes.is_empty() {
        return Err(FrameError::EmptyDatagram);
    }
    let mut frames = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < HEADER_LENGTH {
            return Err(FrameError::TruncatedHeader { offset, remaining });
        }
        let length = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        let counter = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
        offset += HEADER_LENGTH;
        let remaining = bytes.len() - offset;
        if length > remaining {
            return Err(FrameError::TruncatedPayload {
                offset,
                declared: length,
                remaining,
            });
        }
        frames.push(Frame {
            counter,
            payload: &bytes[offset..offset + length],
        });
        offset += length;
    }
    Ok(frames)
}

/// Tune initializes its outgoing counter to zero, then increments before send.
#[derive(Debug, Default, Clone, Copy)]
pub struct RequestCounter(u16);

impl RequestCounter {
    pub fn advance(&mut self) -> u16 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    LittleEndian,
    BigEndian,
}

impl ByteOrder {
    pub fn read_u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::LittleEndian => u16::from_le_bytes(bytes),
            Self::BigEndian => u16::from_be_bytes(bytes),
        }
    }

    pub fn write_u32(self, value: u32) -> [u8; 4] {
        match self {
            Self::LittleEndian => value.to_le_bytes(),
            Self::BigEndian => value.to_be_bytes(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressGranularity {
    Byte,
    Word,
    Dword,
}

impl AddressGranularity {
    pub fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Dword => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyError {
    Truncated { minimum: usize, actual: usize },
    UnexpectedPacket { marker: u8 },
    DeviceError { code: u8 },
    UnsupportedGranularity,
}

impl fmt::Display for ReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { minimum, actual } => write!(
                f,
                "Truncated XCP reply: expected at least {minimum} bytes, found {actual}"
            ),
            Self::UnexpectedPacket { marker } => {
                write!(f, "Unexpected XCP packet marker 0x{marker:02x}")
            }
            Self::DeviceError { code } => write!(f, "XCP device returned error 0x{code:02x}"),
            Self::UnsupportedGranularity => {
                write!(f, "XCP address granularity is reserved or unsupported")
            }
        }
    }
}

impl std::error::Error for ReplyError {}

/// Check the command-result markers handled by Tune's shared reply classifier.
/// Unlike the observed reader, this checks that an error packet has its code.
pub fn check_reply(payload: &[u8]) -> Result<(), ReplyError> {
    match payload.first().copied() {
        Some(0xff) => Ok(()),
        Some(0xfe) => match payload.get(1).copied() {
            Some(0) => Ok(()),
            Some(code) => Err(ReplyError::DeviceError { code }),
            None => Err(ReplyError::Truncated {
                minimum: 2,
                actual: 1,
            }),
        },
        Some(marker) => Err(ReplyError::UnexpectedPacket { marker }),
        None => Err(ReplyError::Truncated {
            minimum: 1,
            actual: 0,
        }),
    }
}

/// The six CONNECT reply bytes consumed by the inspected Tune implementation.
/// Remaining bytes are retained without assigning unverified version fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectReply<'a> {
    pub resources: u8,
    pub communication_mode: u8,
    pub byte_order: ByteOrder,
    pub address_granularity: AddressGranularity,
    pub max_cto: u8,
    pub max_dto: u16,
    pub extension: &'a [u8],
}

impl<'a> ConnectReply<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, ReplyError> {
        check_reply(payload)?;
        if payload.len() < 6 {
            return Err(ReplyError::Truncated {
                minimum: 6,
                actual: payload.len(),
            });
        }
        let byte_order = if payload[2] & 1 == 0 {
            ByteOrder::LittleEndian
        } else {
            ByteOrder::BigEndian
        };
        let address_granularity = match (payload[2] >> 1) & 3 {
            0 => AddressGranularity::Byte,
            1 => AddressGranularity::Word,
            2 => AddressGranularity::Dword,
            _ => return Err(ReplyError::UnsupportedGranularity),
        };
        Ok(Self {
            resources: payload[1],
            communication_mode: payload[2],
            byte_order,
            address_granularity,
            max_cto: payload[3],
            max_dto: byte_order.read_u16([payload[4], payload[5]]),
            extension: &payload[6..],
        })
    }
}

/// Construct the two-byte CONNECT payload with an explicit caller-selected mode.
pub fn connect_request(mode: u8) -> [u8; 2] {
    [0xff, mode]
}

pub const DISCONNECT_REQUEST: [u8; 1] = [0xfe];

/// Set the read address using negotiated address byte order. Reserved bytes are
/// initialized to zero; the extension is a separate byte and is never swapped.
pub fn set_mta_request(byte_order: ByteOrder, extension: u8, address: u32) -> [u8; 8] {
    let mut request = [0xf6, 0, 0, extension, 0, 0, 0, 0];
    request[4..].copy_from_slice(&byte_order.write_u32(address));
    request
}

/// Construct an UPLOAD payload. The count is in negotiated address units.
pub fn upload_request(address_units: u8) -> [u8; 2] {
    [0xf5, address_units]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_little_endian_envelope_without_copying() {
        let bytes = [2, 0, 0x34, 0x12, 0xff, 0];
        let frames = decode_datagram(&bytes).unwrap();
        assert_eq!(
            frames,
            [Frame {
                counter: 0x1234,
                payload: &[0xff, 0]
            }]
        );
        assert_eq!(frames[0].payload.as_ptr(), bytes[4..].as_ptr());
        assert_eq!(encode_frame(0x1234, &[0xff, 0]).unwrap(), bytes);
    }

    #[test]
    fn validates_every_frame_in_a_batched_datagram() {
        let mut bytes = vec![1, 0, 1, 0, 0xff, 2, 0, 2, 0, 0xfe, 0x22];
        assert_eq!(decode_datagram(&bytes).unwrap().len(), 2);
        bytes.pop();
        assert!(matches!(
            decode_datagram(&bytes),
            Err(FrameError::TruncatedPayload {
                offset: 9,
                declared: 2,
                remaining: 1
            })
        ));
        bytes.truncate(6);
        assert!(matches!(
            decode_datagram(&bytes),
            Err(FrameError::TruncatedHeader {
                offset: 5,
                remaining: 1
            })
        ));
    }

    #[test]
    fn rejects_all_short_headers_and_oversized_claims() {
        assert_eq!(decode_datagram(&[]), Err(FrameError::EmptyDatagram));
        for len in 1..HEADER_LENGTH {
            assert!(matches!(
                decode_datagram(&[0; HEADER_LENGTH][..len]),
                Err(FrameError::TruncatedHeader { .. })
            ));
        }
        assert!(matches!(
            decode_datagram(&[0xff, 0xff, 0, 0]),
            Err(FrameError::TruncatedPayload {
                declared: 65_535,
                remaining: 0,
                ..
            })
        ));
        assert!(matches!(
            encode_frame(1, &vec![0; 65_536]),
            Err(FrameError::PayloadTooLarge { length: 65_536 })
        ));
    }

    #[test]
    fn zero_length_envelopes_make_progress_but_are_not_command_responses() {
        assert_eq!(
            decode_datagram(&[0, 0, 1, 0, 0, 0, 2, 0]).unwrap(),
            [
                Frame {
                    counter: 1,
                    payload: &[]
                },
                Frame {
                    counter: 2,
                    payload: &[]
                }
            ]
        );
    }

    #[test]
    fn outgoing_counter_starts_at_one_and_wraps() {
        let mut counter = RequestCounter::default();
        assert_eq!(counter.advance(), 1);
        for expected in 2..=u16::MAX {
            assert_eq!(counter.advance(), expected);
        }
        assert_eq!(counter.advance(), 0);
        assert_eq!(counter.advance(), 1);
    }

    #[test]
    fn reads_connect_fields_using_negotiated_byte_order() {
        let little = ConnectReply::parse(&[0xff, 5, 0, 64, 0x34, 0x12]).unwrap();
        assert_eq!(little.max_dto, 0x1234);
        assert_eq!(little.address_granularity, AddressGranularity::Byte);
        let bytes = [0xff, 5, 0x85, 64, 0x12, 0x34, 0xaa, 0xbb];
        let big = ConnectReply::parse(&bytes).unwrap();
        assert_eq!(big.max_dto, 0x1234);
        assert_eq!(big.address_granularity.bytes(), 4);
        assert_eq!(big.communication_mode, 0x85);
        assert_eq!(big.extension, &[0xaa, 0xbb]);
        assert_eq!(
            ConnectReply::parse(&[0xff, 5, 2, 64, 0, 0])
                .unwrap()
                .address_granularity
                .bytes(),
            2
        );
    }

    #[test]
    fn rejects_short_reserved_and_device_error_replies() {
        assert!(matches!(
            check_reply(&[]),
            Err(ReplyError::Truncated { .. })
        ));
        assert!(matches!(
            check_reply(&[0xfe]),
            Err(ReplyError::Truncated { .. })
        ));
        assert_eq!(
            check_reply(&[0xfe, 0x25]),
            Err(ReplyError::DeviceError { code: 0x25 })
        );
        assert_eq!(
            check_reply(&[0xfd, 1]),
            Err(ReplyError::UnexpectedPacket { marker: 0xfd })
        );
        assert!(check_reply(&[0xfe, 0]).is_ok());
        for len in 1..6 {
            assert!(matches!(
                ConnectReply::parse(&[0xff; 6][..len]),
                Err(ReplyError::Truncated { minimum: 6, .. })
            ));
        }
        assert_eq!(
            ConnectReply::parse(&[0xff, 0, 6, 64, 0, 0]),
            Err(ReplyError::UnsupportedGranularity)
        );
    }

    #[test]
    fn encodes_read_commands_without_swapping_extension_or_transport_header() {
        assert_eq!(connect_request(0), [0xff, 0]);
        assert_eq!(DISCONNECT_REQUEST, [0xfe]);
        assert_eq!(upload_request(31), [0xf5, 31]);
        let address = 0x1234_5678;
        assert_eq!(
            set_mta_request(ByteOrder::LittleEndian, 0x25, address),
            [0xf6, 0, 0, 0x25, 0x78, 0x56, 0x34, 0x12]
        );
        let big = set_mta_request(ByteOrder::BigEndian, 0x25, address);
        assert_eq!(big, [0xf6, 0, 0, 0x25, 0x12, 0x34, 0x56, 0x78]);
        assert_eq!(&encode_frame(1, &big).unwrap()[..4], &[8, 0, 1, 0]);
    }
}

//! Native, read-only XCP/UDP exchanges used as M1 acquisition building blocks.
//!
//! Emits only CONNECT mode zero, DISCONNECT, SET_MTA and read commands. There are
//! no automatic command retries: UPLOAD advances the device's MTA. A timeout,
//! cancellation or invalid reply poisons the client, so delayed replies cannot
//! accidentally continue an uncertain transfer. This is not a complete logging
//! downloader, permission implementation or `.ld` converter.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::log_image::{LogImageError, MO_HEADER_SIZE, MoRecordHeader};
use super::xcp::{
    AddressGranularity, ByteOrder, ConnectReply, DISCONNECT_REQUEST, FrameError, ReplyError,
    RequestCounter, check_reply, connect_request, decode_datagram, encode_frame, set_mta_request,
    upload_request,
};

#[path = "transport/fast_upload.rs"]
mod fast_upload;

const CANCELLATION_POLL: Duration = Duration::from_millis(25);
const DATAGRAM_CAPACITY: usize = 65_536;

/// Shareable cancellation flag. Cancelling does not send a device command.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Owned copy of the observed CONNECT fields, without inventing version fields
/// for the remaining bytes. Resource flags alone do not establish permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedParameters {
    pub resources: u8,
    pub communication_mode: u8,
    pub byte_order: ByteOrder,
    pub address_granularity: AddressGranularity,
    pub max_cto: u8,
    pub max_dto: u16,
    pub extension: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientState {
    Connected,
    Poisoned,
    Disconnected,
}

/// A single serial command stream to one explicitly selected UDP peer.
///
/// The timeout applies to each command exchange, including all ignored stale
/// packets. Cancellation is checked at most every 25 ms while awaiting a reply.
pub struct XcpClient {
    socket: UdpSocket,
    counter: RequestCounter,
    datagram: Box<[u8]>,
    read_timeout: Duration,
    timeout: Duration,
    cancellation: CancellationToken,
    parameters: Option<NegotiatedParameters>,
    state: ClientState,
    requested_upload_bytes: u64,
}

impl XcpClient {
    pub fn connect(
        endpoint: SocketAddr,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, XcpError> {
        let mut client = Self::open(endpoint, timeout, cancellation)?;
        client.negotiate()?;
        Ok(client)
    }

    // Recovery opens the socket before negotiation so even failed handshakes
    // can retain their port until the entire capture finishes.
    pub(crate) fn open(
        endpoint: SocketAddr,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, XcpError> {
        if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
            return Err(XcpError::InvalidTimeout);
        }
        if cancellation.is_cancelled() {
            return Err(XcpError::Cancelled);
        }
        let bind_address = match endpoint {
            SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        };
        let socket = UdpSocket::bind(bind_address)?;
        socket.connect(endpoint)?;
        let poll_timeout = timeout.min(CANCELLATION_POLL);
        socket.set_write_timeout(Some(poll_timeout))?;
        socket.set_read_timeout(Some(poll_timeout))?;
        Ok(Self {
            socket,
            counter: RequestCounter::default(),
            datagram: vec![0; DATAGRAM_CAPACITY].into_boxed_slice(),
            read_timeout: poll_timeout,
            timeout,
            cancellation,
            parameters: None,
            state: ClientState::Connected,
            requested_upload_bytes: 0,
        })
    }

    pub(crate) fn negotiate(&mut self) -> Result<(), XcpError> {
        let payload = self.exchange(&connect_request(0))?;
        let reply = match ConnectReply::parse(&payload) {
            Ok(reply) => reply,
            Err(error) => return self.poison(XcpError::Reply(error)),
        };
        let parameters = NegotiatedParameters {
            resources: reply.resources,
            communication_mode: reply.communication_mode,
            byte_order: reply.byte_order,
            address_granularity: reply.address_granularity,
            max_cto: reply.max_cto,
            max_dto: reply.max_dto,
            extension: reply.extension.to_vec(),
        };
        let granularity = parameters.address_granularity.bytes();
        if parameters.max_cto < 8 || upload_capacity(&parameters) == 0 {
            return self.poison(XcpError::InvalidMaxCto {
                max_cto: parameters.max_cto,
                granularity,
            });
        }
        if payload.len() > usize::from(parameters.max_cto) {
            return self.poison(XcpError::ReplyTooLarge {
                maximum: usize::from(parameters.max_cto),
                actual: payload.len(),
            });
        }
        self.parameters = Some(parameters);
        Ok(())
    }

    pub fn parameters(&self) -> &NegotiatedParameters {
        // Only a successfully negotiated client can escape `connect`.
        self.parameters.as_ref().unwrap()
    }

    pub fn is_poisoned(&self) -> bool {
        self.state == ClientState::Poisoned
    }

    pub(crate) fn requested_upload_bytes(&self) -> u64 {
        self.requested_upload_bytes
    }

    /// End this session before a reconnect, retaining the socket/port to
    /// quarantine late packets. Drop must not disconnect the replacement.
    pub(crate) fn retire(&mut self) {
        if self.state != ClientState::Disconnected && self.socket.set_nonblocking(true).is_ok() {
            let counter = self.counter.advance().to_le_bytes();
            let _ = self.socket.send(&[1, 0, counter[0], counter[1], 0xfe]);
        }
        self.state = ClientState::Disconnected;
    }

    /// Read exactly `length` bytes with an explicit allocation/transfer cap.
    /// Addresses are opaque extension-specific selectors, passed unchanged to
    /// SET_MTA. Only the byte count must align to AG; no read-padding policy is
    /// inferred. A zero-length read emits no command.
    pub fn read_memory(
        &mut self,
        extension: u8,
        address: u32,
        length: usize,
        limit: usize,
    ) -> Result<Vec<u8>, XcpError> {
        self.ensure_ready()?;
        self.validate_read(address, length, limit)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| XcpError::AllocationFailed)?;
        if length == 0 {
            return Ok(bytes);
        }
        self.set_mta(extension, address)?;
        self.upload_exact(length, &mut bytes)?;
        Ok(bytes)
    }

    /// Read a complete Mo record, first its 12-byte header and then its declared
    /// payload through the SAME MTA continuation. `max_bytes` includes the header.
    /// This checks framing/magic/length only; it does not validate a typed root,
    /// nested records, permissions or content checksums.
    pub fn read_record(
        &mut self,
        extension: u8,
        address: u32,
        max_bytes: usize,
    ) -> Result<Vec<u8>, XcpError> {
        self.read_record_with_upload(extension, address, max_bytes, false)
    }

    pub(crate) fn read_record_with_upload(
        &mut self,
        extension: u8,
        address: u32,
        max_bytes: usize,
        fast: bool,
    ) -> Result<Vec<u8>, XcpError> {
        let mut bytes = self.read_memory(extension, address, MO_HEADER_SIZE, max_bytes)?;
        let header = match MoRecordHeader::parse(&bytes) {
            Ok(header) => header,
            Err(error) => return self.poison(XcpError::InvalidRecord(error)),
        };
        let payload_length = match usize::try_from(header.payload_len) {
            Ok(length) => length,
            Err(_) => return self.poison(XcpError::InvalidRecord(LogImageError::LengthOverflow)),
        };
        let Some(total_length) = MO_HEADER_SIZE.checked_add(payload_length) else {
            return self.poison(XcpError::InvalidRecord(LogImageError::LengthOverflow));
        };
        if let Err(error) = self.validate_read(address, total_length, max_bytes) {
            return self.poison(error);
        }
        if bytes.try_reserve_exact(payload_length).is_err() {
            return self.poison(XcpError::AllocationFailed);
        }
        if fast && self.can_fast_upload(payload_length) {
            let result = self.fast_upload_exact(payload_length, &mut bytes);
            if let Err(error) = result {
                return self.poison(error);
            }
        } else {
            self.upload_exact(payload_length, &mut bytes)?;
        }
        Ok(bytes)
    }

    /// Complete a bounded DISCONNECT exchange. Drop separately attempts one
    /// nonblocking send, without waiting, if this was not completed.
    pub fn disconnect(&mut self) -> Result<(), XcpError> {
        if self.state == ClientState::Disconnected {
            return Ok(());
        }
        self.ensure_ready()?;
        let reply = self.exchange(&DISCONNECT_REQUEST)?;
        self.check_ack(&reply)?;
        self.state = ClientState::Disconnected;
        Ok(())
    }

    fn validate_read(&self, address: u32, length: usize, limit: usize) -> Result<(), XcpError> {
        if length > limit {
            return Err(XcpError::ReadLimitExceeded {
                requested: length,
                limit,
            });
        }
        let granularity = self.parameters().address_granularity.bytes();
        if !length.is_multiple_of(granularity) {
            return Err(XcpError::UnalignedRead {
                address,
                length,
                granularity,
            });
        }
        Ok(())
    }

    fn set_mta(&mut self, extension: u8, address: u32) -> Result<(), XcpError> {
        let reply = self.exchange(&set_mta_request(
            self.parameters().byte_order,
            extension,
            address,
        ))?;
        self.check_ack(&reply)
    }

    fn check_ack(&mut self, reply: &[u8]) -> Result<(), XcpError> {
        if reply == [0xff] || reply == [0xfe, 0] {
            Ok(())
        } else {
            self.poison(XcpError::UnexpectedReplyLength {
                expected: 1,
                actual: reply.len(),
            })
        }
    }

    fn upload_exact(&mut self, length: usize, bytes: &mut Vec<u8>) -> Result<(), XcpError> {
        let granularity = self.parameters().address_granularity.bytes();
        let max_units = upload_capacity(self.parameters());
        let mut remaining = length;
        while remaining > 0 {
            let units = max_units
                .min(remaining / granularity)
                .min(usize::from(u8::MAX));
            if units == 0 {
                return self.poison(XcpError::UnalignedRead {
                    address: 0,
                    length: remaining,
                    granularity,
                });
            }
            let chunk = units * granularity;
            // Charge requested data even if its reply is lost. A retry at a
            // higher level must not reset the capture's transfer budget.
            self.requested_upload_bytes = self.requested_upload_bytes.saturating_add(chunk as u64);
            let reply = self.exchange(&upload_request(units as u8))?;
            let expected = granularity + chunk;
            if reply.len() != expected {
                return self.poison(XcpError::UnexpectedReplyLength {
                    expected,
                    actual: reply.len(),
                });
            }
            bytes.extend_from_slice(&reply[granularity..]);
            remaining -= chunk;
        }
        Ok(())
    }

    fn ensure_ready(&mut self) -> Result<(), XcpError> {
        match self.state {
            ClientState::Poisoned => return Err(XcpError::Poisoned),
            ClientState::Disconnected => return Err(XcpError::Disconnected),
            ClientState::Connected => {}
        }
        if self.cancellation.is_cancelled() {
            return self.poison(XcpError::Cancelled);
        }
        Ok(())
    }

    fn poison<T>(&mut self, error: XcpError) -> Result<T, XcpError> {
        self.state = ClientState::Poisoned;
        Err(error)
    }

    fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>, XcpError> {
        self.ensure_ready()?;
        let result = self.exchange_inner(payload);
        if result.is_err() {
            self.state = ClientState::Poisoned;
        }
        result
    }

    fn exchange_inner(&mut self, payload: &[u8]) -> Result<Vec<u8>, XcpError> {
        let (counter, deadline) = self.send_command(payload)?;
        loop {
            let length = self.receive_datagram(deadline)?;
            let frames = decode_datagram(&self.datagram[..length])?;
            let mut matched = None;
            for frame in frames {
                let Some(marker) = frame.payload.first() else {
                    return Err(XcpError::Reply(ReplyError::Truncated {
                        minimum: 1,
                        actual: 0,
                    }));
                };
                // Tune routes only CTO FF/FE responses to pending commands.
                // Events and DAQ packets can reuse the same counter and are
                // irrelevant to this read-only client's awaited response.
                if !matches!(marker, 0xff | 0xfe) {
                    continue;
                }
                if frame.counter == counter {
                    if matched.is_some() {
                        return Err(XcpError::DuplicateReply { counter });
                    }
                    matched = Some(frame.payload);
                }
            }
            let Some(reply) = matched else {
                continue;
            };
            if let Some(parameters) = &self.parameters {
                let maximum = usize::from(parameters.max_cto);
                if reply.len() > maximum {
                    return Err(XcpError::ReplyTooLarge {
                        maximum,
                        actual: reply.len(),
                    });
                }
            }
            match check_reply(reply) {
                Ok(()) => {}
                Err(ReplyError::DeviceError { code }) => {
                    let command = match payload.first() {
                        Some(0xff) => "CONNECT",
                        Some(0xfe) => "DISCONNECT",
                        Some(0xf6) => "SET_MTA",
                        Some(0xf5) => "UPLOAD",
                        _ => "unknown command",
                    };
                    return Err(XcpError::CommandRejected { command, code });
                }
                Err(error) => return Err(error.into()),
            }
            return Ok(reply.to_vec());
        }
    }
    fn send_command(&mut self, payload: &[u8]) -> Result<(u16, Instant), XcpError> {
        self.ensure_ready()?;
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or(XcpError::InvalidTimeout)?;
        let counter = self.counter.advance();
        let request = encode_frame(counter, payload)?;
        if self.socket.send(&request).map_err(classify_io)? != request.len() {
            return Err(XcpError::PartialDatagramSend);
        }
        Ok((counter, deadline))
    }

    fn receive_datagram(&mut self, deadline: Instant) -> Result<usize, XcpError> {
        loop {
            if self.cancellation.is_cancelled() {
                return Err(XcpError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(XcpError::Timeout);
            }
            let read_timeout = remaining.min(CANCELLATION_POLL);
            if read_timeout != self.read_timeout {
                self.socket.set_read_timeout(Some(read_timeout))?;
                self.read_timeout = read_timeout;
            }
            let length = match self.socket.recv(&mut self.datagram) {
                Ok(length) => length,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(XcpError::Io(error)),
            };
            if self.cancellation.is_cancelled() {
                return Err(XcpError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(XcpError::Timeout);
            }
            return Ok(length);
        }
    }
}

impl Drop for XcpClient {
    fn drop(&mut self) {
        self.retire();
    }
}

#[derive(Debug)]
pub enum XcpError {
    Io(io::Error),
    Frame(FrameError),
    Reply(ReplyError),
    CommandRejected {
        command: &'static str,
        code: u8,
    },
    InvalidFastUpload(&'static str),
    InvalidRecord(LogImageError),
    InvalidTimeout,
    Cancelled,
    Timeout,
    Poisoned,
    Disconnected,
    InvalidMaxCto {
        max_cto: u8,
        granularity: usize,
    },
    UnexpectedReplyLength {
        expected: usize,
        actual: usize,
    },
    ReplyTooLarge {
        maximum: usize,
        actual: usize,
    },
    DuplicateReply {
        counter: u16,
    },
    UnalignedRead {
        address: u32,
        length: usize,
        granularity: usize,
    },
    ReadLimitExceeded {
        requested: usize,
        limit: usize,
    },
    AllocationFailed,
    PartialDatagramSend,
}

impl fmt::Display for XcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "XCP UDP I/O failed: {error}"),
            Self::Frame(error) => write!(f, "Invalid XCP frame: {error}"),
            Self::Reply(error) => write!(f, "Invalid XCP reply: {error}"),
            Self::CommandRejected { command, code } => {
                write!(f, "ECU rejected XCP {command} with error 0x{code:02x}")
            }
            Self::InvalidFastUpload(reason) => write!(f, "Invalid M1 Fast Upload reply: {reason}"),
            Self::InvalidRecord(error) => write!(f, "Invalid Mo record: {error}"),
            Self::InvalidTimeout => f.write_str("XCP timeout must be positive and representable"),
            Self::Cancelled => f.write_str("XCP operation cancelled"),
            Self::Timeout => f.write_str("XCP command timed out"),
            Self::Poisoned => f.write_str("XCP session has an uncertain transfer state; reconnect"),
            Self::Disconnected => f.write_str("XCP session is disconnected"),
            Self::InvalidMaxCto {
                max_cto,
                granularity,
            } => write!(
                f,
                "XCP max CTO {max_cto} cannot support the observed read commands with address granularity {granularity}"
            ),
            Self::UnexpectedReplyLength { expected, actual } => {
                write!(f, "XCP reply has {actual} bytes; expected {expected}")
            }
            Self::ReplyTooLarge { maximum, actual } => write!(
                f,
                "XCP reply has {actual} bytes; negotiated maximum is {maximum}"
            ),
            Self::DuplicateReply { counter } => write!(
                f,
                "XCP datagram contains multiple replies for counter {counter}"
            ),
            Self::UnalignedRead {
                address,
                length,
                granularity,
            } => write!(
                f,
                "XCP read length {length} for selector 0x{address:08x} is not a multiple of address granularity {granularity}"
            ),
            Self::ReadLimitExceeded { requested, limit } => write!(
                f,
                "XCP read requires {requested} bytes, exceeding its explicit {limit}-byte limit"
            ),
            Self::AllocationFailed => f.write_str("Could not allocate the bounded XCP read buffer"),
            Self::PartialDatagramSend => f.write_str("XCP UDP datagram was not sent completely"),
        }
    }
}

impl std::error::Error for XcpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::Reply(error) => Some(error),
            Self::InvalidRecord(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for XcpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FrameError> for XcpError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<ReplyError> for XcpError {
    fn from(error: ReplyError) -> Self {
        Self::Reply(error)
    }
}

fn classify_io(error: io::Error) -> XcpError {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        XcpError::Timeout
    } else {
        XcpError::Io(error)
    }
}

fn upload_capacity(parameters: &NegotiatedParameters) -> usize {
    let max_cto = usize::from(parameters.max_cto);
    let granularity = parameters.address_granularity.bytes();
    // Tune uses (maxCTO - 2) / AG. Also cap the payload plus AG-byte response
    // prefix to maxCTO: this conservative client choice handles odd negotiated
    // sizes without requesting an oversized reply or inventing padding reads.
    (max_cto.saturating_sub(2) / granularity)
        .min(max_cto.saturating_sub(granularity) / granularity)
        .min(usize::from(u8::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    fn receive(
        socket: &UdpSocket,
        expected_peer: Option<SocketAddr>,
    ) -> (u16, Vec<u8>, SocketAddr) {
        let mut bytes = [0u8; 1024];
        let (length, peer) = socket.recv_from(&mut bytes).unwrap();
        if let Some(expected) = expected_peer {
            assert_eq!(peer, expected);
        }
        let frames = decode_datagram(&bytes[..length]).unwrap();
        assert_eq!(frames.len(), 1);
        (frames[0].counter, frames[0].payload.to_vec(), peer)
    }

    fn respond(socket: &UdpSocket, peer: SocketAddr, counter: u16, payload: &[u8]) {
        socket
            .send_to(&encode_frame(counter, payload).unwrap(), peer)
            .unwrap();
    }

    fn mock(
        mode: u8,
        max_cto: u8,
        run: impl FnOnce(UdpSocket, SocketAddr) + Send + 'static,
    ) -> (SocketAddr, JoinHandle<()>) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let endpoint = socket.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (counter, command, peer) = receive(&socket, None);
            assert_eq!(counter, 1);
            assert_eq!(command, connect_request(0));
            respond(
                &socket,
                peer,
                counter,
                &[0xff, 5, mode, max_cto, 0, 1, 1, 1],
            );
            run(socket, peer);
        });
        (endpoint, handle)
    }

    fn client(endpoint: SocketAddr) -> XcpClient {
        XcpClient::connect(
            endpoint,
            Duration::from_secs(1),
            CancellationToken::default(),
        )
        .unwrap()
    }

    fn acknowledge(socket: &UdpSocket, peer: SocketAddr, expected: &[u8]) -> u16 {
        let (counter, command, _) = receive(socket, Some(peer));
        assert_eq!(command, expected);
        respond(socket, peer, counter, &[0xff]);
        counter
    }

    #[test]
    fn read_only_command_sequence_matches_echoed_counters_and_ignores_stale_frames() {
        let (endpoint, server) = mock(0, 8, |socket, peer| {
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(counter, 2);
            assert_eq!(command, [0xf6, 0, 0, 0x25, 0x78, 0x56, 0x34, 0x12]);
            let mut batched = encode_frame(1, &[0xfe, 0x22]).unwrap();
            batched.extend_from_slice(&encode_frame(counter, &[0xff]).unwrap());
            socket.send_to(&batched, peer).unwrap();

            let mut offset = 0;
            for (expected_counter, count) in [(3, 6), (4, 6), (5, 2)] {
                let (counter, command, _) = receive(&socket, Some(peer));
                assert_eq!(counter, expected_counter);
                assert_eq!(command, [0xf5, count]);
                let mut reply = vec![0xff];
                reply.extend(offset..offset + count);
                let mut batched = encode_frame(counter - 1, &[0xfe, 0x23]).unwrap();
                batched.extend_from_slice(&encode_frame(counter, &reply).unwrap());
                socket.send_to(&batched, peer).unwrap();
                offset += count;
            }
            assert_eq!(acknowledge(&socket, peer, &[0xfe]), 6);
        });
        let mut client = client(endpoint);
        assert_eq!(client.parameters().max_cto, 8);
        assert_eq!(
            client.read_memory(0x25, 0x1234_5678, 14, 14).unwrap(),
            (0..14).collect::<Vec<_>>()
        );
        client.disconnect().unwrap();
        assert!(matches!(
            client.read_memory(0, 0, 1, 1),
            Err(XcpError::Disconnected)
        ));
        server.join().unwrap();
    }

    #[test]
    fn negotiated_granularity_skips_padding_but_never_swaps_raw_data() {
        for (mode, max_cto, granularity) in [(2, 8, 2), (5, 12, 4), (5, 10, 4)] {
            let (endpoint, server) = mock(mode, max_cto, move |socket, peer| {
                let order = if mode & 1 == 0 {
                    ByteOrder::LittleEndian
                } else {
                    ByteOrder::BigEndian
                };
                // Logical selectors are not required to align to AG.
                acknowledge(&socket, peer, &set_mta_request(order, 0x27, 0x0001_0062));
                let mut remaining = 12usize;
                let mut offset = 0u8;
                while remaining > 0 {
                    let units = ((usize::from(max_cto) - 2) / granularity)
                        .min((usize::from(max_cto) - granularity) / granularity)
                        .min(remaining / granularity);
                    let (counter, command, _) = receive(&socket, Some(peer));
                    assert_eq!(command, [0xf5, units as u8]);
                    let chunk = units * granularity;
                    let mut reply = vec![0xa5; granularity];
                    reply[0] = 0xff;
                    reply.extend(offset..offset + chunk as u8);
                    respond(&socket, peer, counter, &reply);
                    remaining -= chunk;
                    offset += chunk as u8;
                }
                acknowledge(&socket, peer, &[0xfe]);
            });
            let mut client = client(endpoint);
            assert_eq!(
                client.read_memory(0x27, 0x0001_0062, 12, 12).unwrap(),
                (0..12).collect::<Vec<_>>()
            );
            client.disconnect().unwrap();
            server.join().unwrap();
        }
    }

    fn mo_record(payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"Mo\0\0".to_vec();
        bytes.extend_from_slice(&0x0100_0005u32.to_be_bytes());
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn record_payload_continues_mta_without_resetting_or_overreading() {
        let record = mo_record(&[0x91, 0x82, 0x73, 0x64, 0x55]);
        let expected = record.clone();
        let (endpoint, server) = mock(0, 64, move |socket, peer| {
            acknowledge(
                &socket,
                peer,
                &set_mta_request(ByteOrder::LittleEndian, 0x26, 0x10062),
            );
            for (count, data) in [(12, &record[..12]), (5, &record[12..])] {
                let (counter, command, _) = receive(&socket, Some(peer));
                assert_eq!(command, [0xf5, count]);
                let mut reply = vec![0xff];
                reply.extend_from_slice(data);
                respond(&socket, peer, counter, &reply);
            }
            acknowledge(&socket, peer, &[0xfe]);
        });
        let mut client = client(endpoint);
        assert_eq!(client.read_record(0x26, 0x10062, 17).unwrap(), expected);
        client.disconnect().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn limits_and_unaligned_lengths_are_rejected_before_commands() {
        let (endpoint, server) = mock(2, 8, |socket, peer| {
            // No rejected read may emit SET_MTA or UPLOAD.
            acknowledge(&socket, peer, &[0xfe]);
        });
        let mut client = client(endpoint);
        assert!(matches!(
            client.read_memory(0, 0, 18, 16),
            Err(XcpError::ReadLimitExceeded { .. })
        ));
        assert!(matches!(
            client.read_memory(0, 0, 3, 3),
            Err(XcpError::UnalignedRead { .. })
        ));
        assert!(matches!(
            client.read_record(0, 0, 11),
            Err(XcpError::ReadLimitExceeded { .. })
        ));
        assert!(client.read_memory(0, 0, 0, 0).unwrap().is_empty());
        assert!(!client.is_poisoned());
        client.disconnect().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn oversized_declared_record_does_not_upload_payload() {
        let (endpoint, server) = mock(0, 64, |socket, peer| {
            acknowledge(
                &socket,
                peer,
                &set_mta_request(ByteOrder::LittleEndian, 0x2a, 0),
            );
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, [0xf5, 12]);
            let mut reply = vec![0xff];
            let mut header = mo_record(&[]);
            header[8..12].copy_from_slice(&1_000_000u32.to_be_bytes());
            reply.extend_from_slice(&header);
            respond(&socket, peer, counter, &reply);
            let (_, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, [0xfe]); // Drop only, no payload command.
        });
        let mut client = client(endpoint);
        assert!(matches!(
            client.read_record(0x2a, 0, 128),
            Err(XcpError::ReadLimitExceeded {
                requested: 1_000_012,
                limit: 128
            })
        ));
        assert!(client.is_poisoned());
        assert!(matches!(
            client.read_memory(0, 0, 1, 1),
            Err(XcpError::Poisoned)
        ));
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn timeout_does_not_retry_upload_and_poisons_session() {
        let (endpoint, server) = mock(0, 64, |socket, peer| {
            acknowledge(
                &socket,
                peer,
                &set_mta_request(ByteOrder::LittleEndian, 0x25, 0),
            );
            let (_, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, [0xf5, 4]);
            // Do not reply. The next command must be Drop's DISCONNECT.
            let (_, next, _) = receive(&socket, Some(peer));
            assert_eq!(next, [0xfe]);
        });
        let mut client = XcpClient::connect(
            endpoint,
            Duration::from_millis(70),
            CancellationToken::default(),
        )
        .unwrap();
        assert!(matches!(
            client.read_memory(0x25, 0, 4, 4),
            Err(XcpError::Timeout)
        ));
        assert!(client.is_poisoned());
        assert!(matches!(
            client.read_memory(0, 0, 4, 4),
            Err(XcpError::Poisoned)
        ));
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn cancellation_interrupts_wait_and_poisons_session() {
        let cancellation = CancellationToken::default();
        let cancel_worker = cancellation.clone();
        let (sent, waiting) = mpsc::channel();
        let (endpoint, server) = mock(0, 64, move |socket, peer| {
            acknowledge(
                &socket,
                peer,
                &set_mta_request(ByteOrder::LittleEndian, 0x25, 0),
            );
            let (_, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, [0xf5, 4]);
            sent.send(()).unwrap();
            let (_, next, _) = receive(&socket, Some(peer));
            assert_eq!(next, [0xfe]);
        });
        let canceller = thread::spawn(move || {
            waiting.recv_timeout(Duration::from_secs(2)).unwrap();
            cancel_worker.cancel();
        });
        let mut client =
            XcpClient::connect(endpoint, Duration::from_secs(1), cancellation).unwrap();
        assert!(matches!(
            client.read_memory(0x25, 0, 4, 4),
            Err(XcpError::Cancelled)
        ));
        assert!(client.is_poisoned());
        drop(client);
        canceller.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn malformed_frame_and_short_upload_reply_poison_instead_of_returning_partial_data() {
        for malformed_frame in [false, true] {
            let (endpoint, server) = mock(0, 64, move |socket, peer| {
                acknowledge(
                    &socket,
                    peer,
                    &set_mta_request(ByteOrder::LittleEndian, 0, 0),
                );
                let (counter, command, _) = receive(&socket, Some(peer));
                assert_eq!(command, [0xf5, 4]);
                let mut reply = encode_frame(counter, &[0xff, 1]).unwrap();
                if malformed_frame {
                    reply[0..2].copy_from_slice(&20u16.to_le_bytes());
                }
                socket.send_to(&reply, peer).unwrap();
            });
            let mut client = client(endpoint);
            let error = client.read_memory(0, 0, 4, 4).unwrap_err();
            if malformed_frame {
                assert!(matches!(
                    error,
                    XcpError::Frame(FrameError::TruncatedPayload { .. })
                ));
            } else {
                assert!(matches!(
                    error,
                    XcpError::UnexpectedReplyLength {
                        expected: 5,
                        actual: 2
                    }
                ));
            }
            assert!(client.is_poisoned());
            server.join().unwrap();
        }
    }

    #[test]
    fn duplicate_matching_replies_are_rejected() {
        let (endpoint, server) = mock(0, 64, |socket, peer| {
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(command[0], 0xf6);
            let reply = encode_frame(counter, &[0xff]).unwrap();
            socket
                .send_to(&[reply.clone(), reply].concat(), peer)
                .unwrap();
        });
        let mut client = client(endpoint);
        assert!(matches!(
            client.read_memory(0, 0, 1, 1),
            Err(XcpError::DuplicateReply { counter: 2 })
        ));
        assert!(client.is_poisoned());
        server.join().unwrap();
    }

    #[test]
    fn invalid_negotiation_and_pre_cancelled_connect_are_rejected() {
        for (mode, max_cto) in [(0, 7), (5, 3)] {
            let (endpoint, server) = mock(mode, max_cto, |socket, peer| {
                let (_, command, _) = receive(&socket, Some(peer));
                assert_eq!(command, [0xfe]);
            });
            assert!(matches!(
                XcpClient::connect(
                    endpoint,
                    Duration::from_secs(1),
                    CancellationToken::default()
                ),
                Err(XcpError::InvalidMaxCto { .. })
            ));
            server.join().unwrap();
        }
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        assert!(matches!(
            XcpClient::connect(
                SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
                Duration::from_secs(1),
                cancellation
            ),
            Err(XcpError::Cancelled)
        ));
        assert!(matches!(
            XcpClient::connect(
                SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
                Duration::ZERO,
                CancellationToken::default()
            ),
            Err(XcpError::InvalidTimeout)
        ));
    }

    #[test]
    fn shared_zero_error_success_classifier_is_used_consistently() {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let endpoint = socket.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (counter, command, peer) = receive(&socket, None);
            assert_eq!(command, connect_request(0));
            respond(&socket, peer, counter, &[0xfe, 0, 0, 8, 0, 1]);
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(
                command,
                set_mta_request(ByteOrder::LittleEndian, 0x26, u32::MAX)
            );
            respond(&socket, peer, counter, &[0xfe, 0]);
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, upload_request(1));
            // The shared classifier treats FE 00 as success; the observed
            // UPLOAD handler then copies the single byte after the prefix.
            respond(&socket, peer, counter, &[0xfe, 0]);
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, [0xfe]);
            respond(&socket, peer, counter, &[0xfe, 0]);
        });
        let mut client = client(endpoint);
        assert_eq!(client.parameters().resources, 0);
        assert_eq!(client.read_memory(0x26, u32::MAX, 1, 1).unwrap(), [0]);
        client.disconnect().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn asynchronous_packet_with_matching_counter_is_ignored_before_reply_matching() {
        let (endpoint, server) = mock(0, 64, |socket, peer| {
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(
                command,
                set_mta_request(ByteOrder::LittleEndian, 0x26, 0x10062)
            );
            let mut mixed = encode_frame(counter, &[0xfd, 0x12, 0x34]).unwrap();
            mixed.extend_from_slice(&encode_frame(counter, &[0xff]).unwrap());
            socket.send_to(&mixed, peer).unwrap();
            let (counter, command, _) = receive(&socket, Some(peer));
            assert_eq!(command, upload_request(2));
            respond(&socket, peer, counter, &[0xfc, 0x55]);
            respond(&socket, peer, counter, &[0xff, 0xa1, 0xb2]);
            acknowledge(&socket, peer, &[0xfe]);
        });
        let mut client = client(endpoint);
        assert_eq!(
            client.read_memory(0x26, 0x10062, 2, 2).unwrap(),
            [0xa1, 0xb2]
        );
        assert!(!client.is_poisoned());
        client.disconnect().unwrap();
        server.join().unwrap();
    }
}

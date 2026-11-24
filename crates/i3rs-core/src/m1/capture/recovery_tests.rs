//! Fault-injection acceptance tests for whole-record recovery.
use super::tests::{TestDirectory, assert_read_only, info, manifest, permissions, record};
use super::*;
use crate::m1::xcp::{decode_datagram, encode_frame};
use std::collections::{HashMap, HashSet};
use std::net::UdpSocket;
use std::sync::{Arc, Mutex, atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    LostReply,
    Exhaust,
    Denied,
    Boundaries,
    Setups,
    Metadata,
    Spread,
    SavedSector,
    Malformed,
    EcuError,
    Handshake,
    Validation,
    FinalInfo,
    Fast,
    FastUnsupported,
    FastBadChunk,
    FastLoss,
}
#[derive(Debug, Clone)]
struct Event {
    peer: SocketAddr,
    command: Vec<u8>,
}
type MockSelection = ((u8, u32), Vec<u8>, usize);

struct Device {
    endpoint: SocketAddr,
    events: Arc<Mutex<Vec<Event>>>,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}
impl Device {
    fn new(fault: Fault) -> Self {
        let fast = matches!(
            fault,
            Fault::Fast | Fault::FastUnsupported | Fault::FastBadChunk | Fault::FastLoss
        );
        let big_endian = fault == Fault::Fast;
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(25)))
            .unwrap();
        let endpoint = socket.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut sessions = HashMap::new();
            let mut selected: HashMap<SocketAddr, MockSelection> = HashMap::new();
            let mut poisoned_peers = HashSet::new();
            let mut pending_late: Option<(SocketAddr, Vec<u8>)> = None;
            let mut reads: HashMap<(usize, u8, u32), usize> = HashMap::new();
            let mut input = [0; 1024];
            while !stopped.load(Ordering::SeqCst) {
                let (n, peer) = match socket.recv_from(&mut input) {
                    Ok(value) => value,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => panic!("mock socket: {error}"),
                };
                let frames = decode_datagram(&input[..n]).unwrap();
                assert_eq!(frames.len(), 1);
                let frame = frames[0];
                let command = frame.payload;
                observed.lock().unwrap().push(Event {
                    peer,
                    command: command.to_vec(),
                });
                let next_session = sessions.len() + 1;
                let session = *sessions.entry(peer).or_insert(next_session);
                let response = match command[0] {
                    0xff => {
                        assert_eq!(command, [0xff, 0]);
                        if let Some((old_peer, bytes)) = pending_late.take() {
                            assert_ne!(
                                peer, old_peer,
                                "fresh connection must have a distinct local UDP port"
                            );
                            socket.send_to(&bytes, old_peer).unwrap();
                        }
                        if fault == Fault::Handshake && session == 2 {
                            continue;
                        }
                        if fast {
                            if big_endian {
                                vec![0xff, 0, 1, 64, 4, 0]
                            } else {
                                vec![0xff, 0, 0, 64, 0, 4]
                            }
                        } else {
                            vec![0xff, 0, 0, 64, 64, 0]
                        }
                    }
                    0xfe => vec![0xff], // Retiring old peers must not end the mock.
                    0xf6 => {
                        assert!(
                            !poisoned_peers.contains(&peer),
                            "uncertain session must be retired, not reused"
                        );
                        let extension = command[3];
                        let address_bytes = command[4..8].try_into().unwrap();
                        let address = if big_endian {
                            u32::from_be_bytes(address_bytes)
                        } else {
                            u32::from_le_bytes(address_bytes)
                        };
                        let count = reads.entry((session, extension, address)).or_default();
                        *count += 1;
                        let bytes = match (extension, address) {
                            (0x26, LOG_INFO_SELECTOR) => {
                                info(if session > 1 && fault == Fault::Boundaries {
                                    3
                                } else {
                                    2
                                })
                            }
                            (0x26, PERMISSIONS_SELECTOR) => {
                                permissions(!(session > 1 && fault == Fault::Denied))
                            }
                            (0x27, 0) => record(
                                0x0200_0020,
                                0,
                                &vec![
                                    if session > 1 && fault == Fault::Setups {
                                        8
                                    } else {
                                        7
                                    };
                                    if fast { 2444 } else { 128 }
                                ],
                            ),
                            (0x2a, 0) => record(
                                0x0200_0046,
                                0,
                                &vec![
                                    if session > 1 && fault == Fault::Metadata {
                                        10
                                    } else {
                                        9
                                    };
                                    if fast { 2444 } else { 128 }
                                ],
                            ),
                            (0x25, sector @ 0..=1) => record(
                                0x0200_004c,
                                0,
                                &vec![
                                    if session > 1 && fault == Fault::SavedSector {
                                        99
                                    } else {
                                        sector as u8
                                    };
                                    if fast { 4000 } else { 256 }
                                ],
                            ),
                            selector => panic!("unexpected selector {selector:?}"),
                        };
                        selected.insert(peer, ((extension, address), bytes, 0));
                        vec![0xff]
                    }
                    0xf5 => {
                        assert!(
                            !poisoned_peers.contains(&peer),
                            "never repeat UPLOAD after a lost reply"
                        );
                        let (key, bytes, offset) = selected.get_mut(&peer).unwrap();
                        let count = usize::from(command[1]);
                        let previous = *offset;
                        let end = previous + count;
                        let mut response = vec![0xff];
                        response.extend_from_slice(
                            bytes
                                .get(previous..end)
                                .expect("record continuation overrun"),
                        );
                        *offset = end; // ECU consumes this request even when its reply is lost.
                        let fail = if fast {
                            false
                        } else if fault == Fault::Spread {
                            previous >= 75
                                && ((session == 1 && *key == (0x25, 1))
                                    || (session == 2 && *key == (0x25, 0)))
                        } else if fault == Fault::FinalInfo {
                            session == 1
                                && *key == (0x26, LOG_INFO_SELECTOR)
                                && reads[&(session, key.0, key.1)] == 2
                                && previous >= 12
                        } else {
                            *key == (0x25, 0)
                                && previous >= 75
                                && (session == 1 || fault == Fault::Exhaust)
                                || fault == Fault::Validation
                                    && session == 2
                                    && *key == (0x27, 0)
                                    && previous >= 12
                        };
                        if fail {
                            match fault {
                                Fault::Malformed => response = vec![0xff, 1],
                                Fault::EcuError => response = vec![0xfe, 0x31],
                                _ => {
                                    poisoned_peers.insert(peer);
                                    pending_late = Some((
                                        peer,
                                        encode_frame(frame.counter, &response).unwrap(),
                                    ));
                                    continue;
                                }
                            }
                        }
                        response
                    }
                    0xf1 if fast => {
                        assert_eq!(&command[..4], &[0xf1, 0xff, 0, 0]);
                        assert!(!poisoned_peers.contains(&peer));
                        if fault == Fault::FastUnsupported {
                            assert_eq!(session, 1, "unsupported mode must be disabled");
                            let response = encode_frame(frame.counter, &[0xfe, 0x20]).unwrap();
                            socket.send_to(&response, peer).unwrap();
                            continue;
                        }
                        let (key, bytes, offset) = selected.get_mut(&peer).unwrap();
                        let length_bytes = command[4..].try_into().unwrap();
                        let length = if big_endian {
                            u32::from_be_bytes(length_bytes)
                        } else {
                            u32::from_le_bytes(length_bytes)
                        } as usize;
                        assert_eq!(*offset, MO_HEADER_SIZE);
                        let end = *offset + length;
                        assert_eq!(end, bytes.len());
                        let mut frames = Vec::new();
                        for (index, chunk) in bytes[*offset..end].chunks(1020).enumerate().rev() {
                            let mut reply = vec![0xff, 0];
                            let index_bytes = if big_endian {
                                (index as u16).to_be_bytes()
                            } else {
                                (index as u16).to_le_bytes()
                            };
                            reply.extend_from_slice(&index_bytes);
                            reply.extend_from_slice(chunk);
                            let encoded = encode_frame(frame.counter, &reply).unwrap();
                            if *key == (0x25, 0) && session == 1 {
                                if fault == Fault::FastLoss && index == 0 {
                                    poisoned_peers.insert(peer);
                                    pending_late = Some((peer, encoded));
                                    continue;
                                }
                                if fault == Fault::FastBadChunk && index == 1 {
                                    frames.extend_from_slice(&encoded);
                                    reply[4] ^= 1;
                                    frames.extend_from_slice(
                                        &encode_frame(frame.counter, &reply).unwrap(),
                                    );
                                    continue;
                                }
                            }
                            frames.extend_from_slice(&encoded);
                            if index == 2 {
                                frames.extend_from_slice(&encoded);
                            }
                        }
                        *offset = end;
                        socket.send_to(&frames, peer).unwrap();
                        continue;
                    }
                    opcode => panic!("unexpected command 0x{opcode:02x}"),
                };
                socket
                    .send_to(&encode_frame(frame.counter, &response).unwrap(), peer)
                    .unwrap();
            }
        });
        Self {
            endpoint,
            events,
            stop,
            handle,
        }
    }
    fn finish(self) -> Vec<Event> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap();
        Arc::try_unwrap(self.events).unwrap().into_inner().unwrap()
    }
}
fn capture(
    device: &Device,
    base: &TestDirectory,
    limits: &CaptureLimits,
    token: CancellationToken,
    progress: impl FnMut(CaptureProgress),
) -> Result<CaptureResult, CaptureError> {
    download_raw(
        device.endpoint,
        vec![0],
        &base.0,
        limits,
        Duration::from_millis(80),
        token,
        progress,
    )
}
fn selector_count(events: &[Event], extension: u8, address: u32) -> usize {
    events
        .iter()
        .filter(|event| {
            event.command.len() == 8
                && event.command[0] == 0xf6
                && event.command[3] == extension
                && event.command[4..8] == address.to_le_bytes()
        })
        .count()
}
fn connections(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| event.command[0] == 0xff)
        .count()
}
#[test]
fn lost_upload_reply_restarts_record_on_new_port_and_preserves_exact_data() {
    for fault in [
        Fault::LostReply,
        Fault::Handshake,
        Fault::Validation,
        Fault::FinalInfo,
    ] {
        let device = Device::new(fault);
        let base = TestDirectory::new();
        let mut reports = Vec::new();
        let result = capture(
            &device,
            &base,
            &CaptureLimits::default(),
            CancellationToken::default(),
            |p| reports.push(p),
        )
        .unwrap();
        assert_eq!(result.manifest.state, CaptureState::Complete);
        let saved = manifest(&result.directory);
        assert_eq!(saved.files.len(), 7); // two info, fallback permissions, two config, two sectors
        assert_eq!(saved.files.iter().filter(|f| f.sector.is_some()).count(), 2);
        for sector in [0, 1] {
            let file = saved
                .files
                .iter()
                .find(|file| file.sector == Some(sector))
                .unwrap();
            assert_eq!(
                fs::read(result.directory.join(&file.file_name)).unwrap(),
                record(0x0200_004c, 0, &[sector as u8; 256])
            );
        }
        assert!(
            saved
                .diagnostics
                .iter()
                .any(|s| s.contains("verified unchanged source"))
        );
        assert!(reports.windows(2).all(|p| p[0].bytes <= p[1].bytes));
        assert!(
            reports
                .iter()
                .any(|p| p.stage == CaptureStage::Reconnecting)
        );
        assert!(reports.last().unwrap().bytes > saved.total_bytes);
        let events = device.finish();
        assert_eq!(
            connections(&events),
            if matches!(fault, Fault::Handshake | Fault::Validation) {
                3
            } else {
                2
            }
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.command[0] == 0xff)
                .map(|e| e.peer)
                .collect::<HashSet<_>>()
                .len(),
            connections(&events)
        );
        assert!(
            selector_count(&events, 0x25, 1) >= 2,
            "saved sectors must be revalidated"
        );
        assert_read_only(&events.iter().map(|e| e.command.clone()).collect::<Vec<_>>());
    }
}
#[test]
fn source_changes_or_revoked_rights_stop_recovery_without_restarting_failed_record() {
    for fault in [
        Fault::Denied,
        Fault::Boundaries,
        Fault::Setups,
        Fault::Metadata,
        Fault::SavedSector,
    ] {
        let device = Device::new(fault);
        let base = TestDirectory::new();
        let error = capture(
            &device,
            &base,
            &CaptureLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(
                error.kind,
                CaptureErrorKind::PermissionDenied(0) | CaptureErrorKind::SourceChanged(_)
            ),
            "{fault:?}: {error}"
        );
        let saved = manifest(&error.bundle_directory.unwrap());
        assert_ne!(saved.state, CaptureState::Complete);
        assert_eq!(saved.files.iter().filter(|f| f.sector.is_some()).count(), 1);
        let events = device.finish();
        assert_eq!(connections(&events), 2);
        assert_eq!(selector_count(&events, 0x25, 0), 1);
        if fault == Fault::Denied {
            assert_eq!(selector_count(&events, 0x25, 1), 1);
        }
    }
}
#[test]
fn reconnect_budget_is_global_and_zero_disables_recovery() {
    for attempts in [0, 2] {
        let device = Device::new(Fault::Exhaust);
        let base = TestDirectory::new();
        let error = capture(
            &device,
            &base,
            &CaptureLimits {
                max_reconnects: attempts,
                ..Default::default()
            },
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error.kind,CaptureErrorKind::RecoveryExhausted(n) if n==attempts));
        let saved = manifest(&error.bundle_directory.unwrap());
        assert_eq!(saved.state, CaptureState::Incomplete);
        assert_eq!(saved.files.iter().filter(|f| f.sector.is_some()).count(), 1);
        assert_eq!(connections(&device.finish()), usize::from(attempts) + 1);
    }
}
#[test]
fn invalid_or_explicit_negative_replies_are_not_automatically_retried() {
    for fault in [Fault::Malformed, Fault::EcuError] {
        let device = Device::new(fault);
        let base = TestDirectory::new();
        let error = capture(
            &device,
            &base,
            &CaptureLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap_err();
        if fault == Fault::EcuError {
            assert!(matches!(
                error.kind,
                CaptureErrorKind::Transport(XcpError::CommandRejected {
                    command: "UPLOAD",
                    code: 0x31,
                })
            ));
            let directory = error.bundle_directory.unwrap();
            let saved: CaptureManifest =
                serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap())
                    .unwrap();
            assert!(saved.diagnostics.iter().any(|message| {
                message.contains("sector 0") && message.contains("after 1/2 sectors")
            }));
        } else {
            assert!(matches!(error.kind, CaptureErrorKind::Transport(_)));
        }
        assert_eq!(connections(&device.finish()), 1);
    }
}
#[test]
fn cancellation_during_backoff_does_not_open_another_connection() {
    let device = Device::new(Fault::LostReply);
    let base = TestDirectory::new();
    let token = CancellationToken::default();
    let cancel = token.clone();
    let error = capture(&device, &base, &CaptureLimits::default(), token, |p| {
        if p.stage == CaptureStage::Reconnecting {
            cancel.cancel();
        }
    })
    .unwrap_err();
    assert!(matches!(error.kind, CaptureErrorKind::Cancelled));
    assert_eq!(
        manifest(&error.bundle_directory.unwrap()).state,
        CaptureState::Incomplete
    );
    assert_eq!(connections(&device.finish()), 1);
}
#[test]
fn retries_and_lost_replies_do_not_reset_transfer_byte_limit() {
    let device = Device::new(Fault::LostReply);
    let base = TestDirectory::new();
    let cap = 2 * info(2).len() + permissions(true).len() + 2 * (12 + 128) + 2 * (12 + 256);
    let error = capture(
        &device,
        &base,
        &CaptureLimits {
            max_total_bytes: cap as u64,
            ..Default::default()
        },
        CancellationToken::default(),
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        CaptureErrorKind::ByteLimit { .. }
            | CaptureErrorKind::Transport(XcpError::ReadLimitExceeded { .. })
    ));
    let saved = manifest(&error.bundle_directory.unwrap());
    assert_ne!(saved.state, CaptureState::Complete);
    let events = device.finish();
    let requested: usize = events
        .iter()
        .filter(|e| e.command[0] == 0xf5)
        .map(|e| usize::from(e.command[1]))
        .sum();
    assert!(requested <= cap, "{requested} > {cap}");
}

#[test]
fn separate_record_failures_share_one_global_reconnect_budget() {
    let device = Device::new(Fault::Spread);
    let base = TestDirectory::new();
    let error = capture(
        &device,
        &base,
        &CaptureLimits {
            max_reconnects: 1,
            ..Default::default()
        },
        CancellationToken::default(),
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(error.kind, CaptureErrorKind::RecoveryExhausted(1)));
    let saved = manifest(&error.bundle_directory.unwrap());
    assert_eq!(saved.files.iter().filter(|f| f.sector.is_some()).count(), 1);
    assert_eq!(connections(&device.finish()), 2);
}

#[test]
fn fast_upload_verifies_bytes_and_handles_out_of_order_and_duplicate_chunks() {
    let device = Device::new(Fault::Fast);
    let base = TestDirectory::new();
    let result = capture(
        &device,
        &base,
        &CaptureLimits::default(),
        CancellationToken::default(),
        |_| {},
    )
    .unwrap();
    let events = device.finish();
    assert_eq!(connections(&events), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.command[0] == 0xf1)
            .count(),
        4
    );
    assert!(
        result
            .manifest
            .diagnostics
            .iter()
            .any(|line| line.contains("Verified M1 Fast Upload"))
    );
    assert_fast_test_sectors(&result);
}

#[test]
fn fast_upload_falls_back_or_restarts_whole_records_after_failure() {
    for fault in [Fault::FastUnsupported, Fault::FastBadChunk, Fault::FastLoss] {
        let device = Device::new(fault);
        let base = TestDirectory::new();
        let result = capture(
            &device,
            &base,
            &CaptureLimits::default(),
            CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        let events = device.finish();
        assert_eq!(connections(&events), 2, "{fault:?}");
        let replacement = events
            .iter()
            .rev()
            .find(|event| event.command[0] == 0xff)
            .unwrap()
            .peer;
        assert_eq!(
            events
                .iter()
                .any(|event| event.peer == replacement && event.command[0] == 0xf1),
            fault == Fault::FastLoss,
            "{fault:?}",
        );
        assert_fast_test_sectors(&result);
    }
}

fn assert_fast_test_sectors(result: &CaptureResult) {
    assert_eq!(result.manifest.state, CaptureState::Complete);
    for sector in 0..2 {
        let path = result
            .directory
            .join(format!("system-00-sector-{sector:010}.mo"));
        assert_eq!(
            fs::read(path).unwrap(),
            record(0x0200_004c, 0, &[sector; 4000])
        );
    }
}

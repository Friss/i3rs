//! Native DNS-SD browsing for candidate M1 advertisements.
//!
//! Shipped M1 Tune registers both service strings with its Discovery client;
//! physical-device compatibility remains unverified. Discovery never connects
//! to a resolved endpoint or sends an ECU command. The host must permit local
//! multicast networking; interface/firewall/privacy configuration is left to
//! the operator. See the mdns-sd 0.21.4 `ResolvedService` and `ScopedIp` APIs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::{SocketAddr, SocketAddrV6};
use std::time::{Duration, Instant};

use mdns_sd::{DaemonEvent, DaemonStatus, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent};

use super::transport::CancellationToken;

/// Candidate service names observed in Tune, not a hardware support guarantee.
pub const CANDIDATE_SERVICE_TYPES: [&str; 2] =
    ["_motec-xcp-e7._udp.local.", "_motec-15._udp.local."];
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CANDIDATES: usize = 1024;
const MAX_WARNINGS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interface {
    pub name: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// An IPv6 scope is retained even when the address is not link-local.
    pub address: SocketAddr,
    pub interfaces: Vec<Interface>,
}

/// TXT values remain bytes; a flag (`None`) is distinct from an empty value.
/// mdns-sd has already parsed/deduplicated TXT keys, so this is not wire capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtProperty {
    pub key: String,
    pub value: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDevice {
    pub service_type: String,
    pub fullname: String,
    pub hostname: String,
    pub endpoints: Vec<Endpoint>,
    /// Advertised addresses that cannot select a usable peer, including an
    /// IPv6 link-local address without an interface scope.
    pub unusable_endpoints: Vec<SocketAddr>,
    pub txt: Vec<TxtProperty>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub devices: Vec<CandidateDevice>,
    /// Bounded daemon diagnostics, including host networking errors. An empty
    /// list of devices does not prove that no hardware is connected.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    InvalidWindow,
    Cancelled,
    Mdns {
        stage: &'static str,
        message: String,
    },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow => {
                f.write_str("Discovery window must be positive and representable")
            }
            Self::Cancelled => f.write_str("Device discovery cancelled"),
            Self::Mdns { stage, message } => {
                write!(f, "Device discovery {stage} failed: {message}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

fn mdns_error(stage: &'static str, error: impl fmt::Display) -> DiscoveryError {
    DiscoveryError::Mdns {
        stage,
        message: error.to_string(),
    }
}

/// Browse both candidate types on the host's available interfaces for `window`.
/// Call from a worker thread. Cancellation is polled every 25 ms; shutdown can
/// take an additional two seconds. All exit paths request daemon shutdown.
/// No discovery or XCP traffic is emitted if cancellation is already set.
pub fn discover(
    window: Duration,
    cancellation: &CancellationToken,
) -> Result<DiscoveryReport, DiscoveryError> {
    let deadline = Instant::now()
        .checked_add(window)
        .filter(|_| !window.is_zero())
        .ok_or(DiscoveryError::InvalidWindow)?;
    if cancellation.is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }
    let mut owner = OwnedDaemon {
        daemon: ServiceDaemon::new().map_err(|error| mdns_error("startup", error))?,
        exit_requested: false,
    };
    // Setup also runs inside the result boundary so a second browse/monitor
    // failure cannot leave the first browse's daemon running.
    let result = browse(&owner.daemon, deadline, cancellation);
    let shutdown = owner.shutdown();
    match (result, shutdown) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (result, Err(error)) => Err(mdns_error(
            "shutdown",
            match result {
                Err(original) => format!("{error}; earlier operation: {original}"),
                Ok(_) => error.to_string(),
            },
        )),
    }
}

struct OwnedDaemon {
    daemon: ServiceDaemon,
    exit_requested: bool,
}

impl OwnedDaemon {
    fn shutdown(&mut self) -> Result<(), DiscoveryError> {
        let status = self
            .daemon
            .shutdown()
            .map_err(|error| mdns_error("shutdown request", error))?;
        self.exit_requested = true;
        match status.recv_timeout(SHUTDOWN_TIMEOUT) {
            Ok(DaemonStatus::Shutdown) => Ok(()),
            Ok(status) => Err(mdns_error("shutdown status", format!("{status:?}"))),
            Err(error) => Err(mdns_error("shutdown acknowledgement", error)),
        }
    }
}

impl Drop for OwnedDaemon {
    fn drop(&mut self) {
        if !self.exit_requested {
            // Fallback covers unwinding and shutdown-request errors. No other
            // caller can fill this privately owned daemon's command queue.
            let _ = self.daemon.shutdown();
        }
    }
}

fn browse(
    daemon: &ServiceDaemon,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<DiscoveryReport, DiscoveryError> {
    let monitor = daemon
        .monitor()
        .map_err(|error| mdns_error("monitor", error))?;
    let mut receivers = Vec::new();
    for service_type in CANDIDATE_SERVICE_TYPES {
        receivers.push(
            daemon
                .browse(service_type)
                .map_err(|error| mdns_error("browse", error))?,
        );
    }
    let mut devices = BTreeMap::new();
    let mut warnings = BTreeSet::new();
    let mut next_receiver = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        // Rotate the blocking receiver. Bound draining work so advertisements
        // cannot starve cancellation or extend the browsing window.
        let received = receivers[next_receiver].recv_timeout(remaining.min(POLL_INTERVAL));
        if cancellation.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        match received {
            Ok(event) => apply_event(event, &mut devices, &mut warnings),
            Err(mdns_sd::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(mdns_error("event receive", error)),
        }
        next_receiver = (next_receiver + 1) % receivers.len();
        for receiver in &receivers {
            for _ in 0..32 {
                if cancellation.is_cancelled() {
                    return Err(DiscoveryError::Cancelled);
                }
                match receiver.try_recv() {
                    Ok(event) => apply_event(event, &mut devices, &mut warnings),
                    Err(mdns_sd::TryRecvError::Empty) => break,
                    Err(error) => return Err(mdns_error("event receive", error)),
                }
            }
        }
        for _ in 0..32 {
            if cancellation.is_cancelled() {
                return Err(DiscoveryError::Cancelled);
            }
            match monitor.try_recv() {
                Ok(DaemonEvent::Error(error)) => warn(&mut warnings, error.to_string()),
                Ok(_) => {}
                Err(mdns_sd::TryRecvError::Empty) => break,
                Err(error) => return Err(mdns_error("monitor receive", error)),
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    Ok(DiscoveryReport {
        devices: devices.into_values().collect(),
        warnings: warnings.into_iter().collect(),
    })
}

fn warn(warnings: &mut BTreeSet<String>, message: String) {
    if warnings.len() < MAX_WARNINGS {
        warnings.insert(message);
    }
}

fn apply_event(
    event: ServiceEvent,
    devices: &mut BTreeMap<String, CandidateDevice>,
    warnings: &mut BTreeSet<String>,
) {
    match event {
        ServiceEvent::ServiceResolved(service) => {
            if let Some(device) = candidate_from_service(&service) {
                let key = device.fullname.to_ascii_lowercase();
                if devices.contains_key(&key) || devices.len() < MAX_CANDIDATES {
                    // Resolution updates replace stale addresses/TXT values.
                    devices.insert(key, device);
                } else {
                    warn(
                        warnings,
                        format!("Candidate limit {MAX_CANDIDATES} reached"),
                    );
                }
            } else if !service.fullname.is_empty() {
                devices.remove(&service.fullname.to_ascii_lowercase());
            }
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            devices.remove(&fullname.to_ascii_lowercase());
        }
        _ => {}
    }
}

/// Pure mapping of an advertisement. Data remains untrusted and no TXT keys
/// are interpreted as ECU identity or permission information.
pub fn candidate_from_service(service: &ResolvedService) -> Option<CandidateDevice> {
    if !CANDIDATE_SERVICE_TYPES
        .iter()
        .any(|name| service.ty_domain.eq_ignore_ascii_case(name))
        || service.fullname.is_empty()
        || service.host.is_empty()
        || service.port == 0
    {
        return None;
    }
    let mut endpoints: BTreeMap<SocketAddr, BTreeSet<Interface>> = BTreeMap::new();
    let mut unusable = BTreeSet::new();
    for ip in &service.addresses {
        let (address, interfaces, usable) = match ip {
            ScopedIp::V4(ip) => (
                SocketAddr::from((*ip.addr(), service.port)),
                ip.interface_ids().iter().map(interface).collect::<Vec<_>>(),
                !ip.addr().is_unspecified()
                    && !ip.addr().is_multicast()
                    && !ip.addr().is_broadcast(),
            ),
            ScopedIp::V6(ip) => {
                let scope = ip.scope_id();
                let is_link_local = ip.addr().is_unicast_link_local();
                let scope_id = if is_link_local { scope.index } else { 0 };
                let interfaces = if scope.index != 0 || !scope.name.is_empty() {
                    vec![interface(scope)]
                } else {
                    Vec::new()
                };
                let mapped_unusable = ip.addr().to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast()
                });
                (
                    SocketAddr::V6(SocketAddrV6::new(*ip.addr(), service.port, 0, scope_id)),
                    interfaces,
                    !ip.addr().is_unspecified()
                        && !ip.addr().is_multicast()
                        && !mapped_unusable
                        && (!is_link_local || scope.index != 0),
                )
            }
            _ => continue,
        };
        if usable {
            endpoints.entry(address).or_default().extend(interfaces);
        } else {
            unusable.insert(address);
        }
    }
    Some(CandidateDevice {
        service_type: service.ty_domain.clone(),
        fullname: service.fullname.clone(),
        hostname: service.host.clone(),
        endpoints: endpoints
            .into_iter()
            .map(|(address, interfaces)| Endpoint {
                address,
                interfaces: interfaces.into_iter().collect(),
            })
            .collect(),
        unusable_endpoints: unusable.into_iter().collect(),
        txt: service
            .txt_properties
            .iter()
            .map(|property| TxtProperty {
                key: property.key().to_owned(),
                value: property.val().map(<[u8]>::to_vec),
            })
            .collect(),
    })
}

fn interface(id: &mdns_sd::InterfaceId) -> Interface {
    Interface {
        name: id.name.clone(),
        index: id.index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::{ServiceInfo, TxtProperties};
    use serde_json::json;

    fn service() -> ResolvedService {
        ServiceInfo::new(
            CANDIDATE_SERVICE_TYPES[0],
            "candidate",
            "ecu.local.",
            "192.0.2.1",
            5555,
            Vec::<mdns_sd::TxtProperty>::new(),
        )
        .unwrap()
        .as_resolved_service()
    }

    fn scoped_v6(address: &str, index: u32, name: &str) -> ScopedIp {
        serde_json::from_value(
            json!({"V6": {"addr": address, "scope_id": {"index": index, "name": name}}}),
        )
        .unwrap()
    }

    #[test]
    fn preserves_distinct_ipv6_scopes_and_rejects_missing_link_local_scope() {
        let mut service = service();
        service.addresses = [
            scoped_v6("fe80::1", 4, "en4"),
            scoped_v6("fe80::1", 7, "en7"),
            scoped_v6("fe80::1", 0, ""),
        ]
        .into();
        let device = candidate_from_service(&service).unwrap();
        assert_eq!(device.endpoints.len(), 2);
        assert_eq!(
            device.unusable_endpoints,
            ["[fe80::1]:5555".parse().unwrap()]
        );
        for (endpoint, index) in device.endpoints.iter().zip([4, 7]) {
            assert_eq!(
                endpoint.address,
                SocketAddr::V6(SocketAddrV6::new(
                    "fe80::1".parse().unwrap(),
                    5555,
                    0,
                    index
                ))
            );
            assert_eq!(
                endpoint.interfaces[0],
                Interface {
                    name: format!("en{index}"),
                    index
                }
            );
        }
    }

    #[test]
    fn deduplicates_socket_addresses_and_retains_all_ipv4_interfaces() {
        let mut service = service();
        service.addresses = [4, 7]
            .map(|index| {
                ScopedIp::V4(mdns_sd::ScopedIpV4::new(
                    "192.0.2.1".parse().unwrap(),
                    mdns_sd::InterfaceId {
                        name: format!("en{index}"),
                        index,
                    },
                ))
            })
            .into();
        let device = candidate_from_service(&service).unwrap();
        assert_eq!(device.endpoints.len(), 1);
        assert_eq!(device.endpoints[0].interfaces.len(), 2);
    }

    #[test]
    fn keeps_binary_txt_flag_and_empty_values_without_assuming_keys() {
        let mut service = service();
        service.txt_properties = TxtProperties::from(
            &[
                6, b'O', b'p', b'a', b'q', b'=', 0xff, 4, b'f', b'l', b'a', b'g', 6, b'e', b'm',
                b'p', b't', b'y', b'=',
            ][..],
        );
        assert_eq!(
            candidate_from_service(&service).unwrap().txt,
            [
                TxtProperty {
                    key: "Opaq".into(),
                    value: Some(vec![0xff])
                },
                TxtProperty {
                    key: "flag".into(),
                    value: None
                },
                TxtProperty {
                    key: "empty".into(),
                    value: Some(vec![])
                },
            ]
        );
    }

    #[test]
    fn resolution_updates_replace_stale_endpoints_and_removals_are_case_insensitive() {
        let first = service();
        let mut updated = first.clone();
        updated.fullname = first.fullname.to_ascii_uppercase();
        updated.addresses = [ScopedIp::from(
            "192.0.2.2".parse::<std::net::IpAddr>().unwrap(),
        )]
        .into();
        let mut devices = BTreeMap::new();
        let mut warnings = BTreeSet::new();
        apply_event(
            ServiceEvent::ServiceResolved(Box::new(first.clone())),
            &mut devices,
            &mut warnings,
        );
        apply_event(
            ServiceEvent::ServiceResolved(Box::new(updated)),
            &mut devices,
            &mut warnings,
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices.values().next().unwrap().endpoints[0].address,
            "192.0.2.2:5555".parse().unwrap()
        );
        apply_event(
            ServiceEvent::ServiceRemoved(first.ty_domain, first.fullname),
            &mut devices,
            &mut warnings,
        );
        assert!(devices.is_empty());
    }

    #[test]
    fn rejects_unrelated_services_and_zero_ports() {
        let mut service = service();
        service.ty_domain = "_unrelated._udp.local.".into();
        assert!(candidate_from_service(&service).is_none());
        service.ty_domain = CANDIDATE_SERVICE_TYPES[1].into();
        service.port = 0;
        assert!(candidate_from_service(&service).is_none());
    }

    #[test]
    fn pre_cancelled_scan_and_invalid_window_need_no_network() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        assert_eq!(
            discover(Duration::from_secs(1), &cancellation),
            Err(DiscoveryError::Cancelled)
        );
        assert_eq!(
            discover(Duration::ZERO, &CancellationToken::default()),
            Err(DiscoveryError::InvalidWindow)
        );
    }

    #[test]
    fn non_link_local_ipv6_clears_scope_id_and_rejects_mapped_multicast() {
        let mut service = service();
        service.addresses = [
            scoped_v6("fd00::1", 4, "en4"),
            scoped_v6("fd00::1", 7, "en7"),
            scoped_v6("::ffff:224.0.0.1", 4, "en4"),
        ]
        .into();
        let device = candidate_from_service(&service).unwrap();
        assert_eq!(device.endpoints.len(), 1);
        assert_eq!(
            device.endpoints[0].address,
            SocketAddr::V6(SocketAddrV6::new("fd00::1".parse().unwrap(), 5555, 0, 0))
        );
        assert_eq!(device.endpoints[0].interfaces.len(), 2);
        assert_eq!(device.unusable_endpoints.len(), 1);
    }
}

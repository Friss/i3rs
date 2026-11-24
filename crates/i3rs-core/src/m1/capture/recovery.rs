//! Restart whole records on fresh sessions; never replay a stateful UPLOAD.

use sha2::{Digest, Sha256};

use super::super::xcp::ByteOrder;
use super::*;

pub(super) struct RecoverySettings {
    pub endpoint: SocketAddr,
    pub timeout: Duration,
    pub cancellation: CancellationToken,
    pub systems: Vec<u8>,
}

struct CommittedRecord {
    extension: u8,
    address: u32,
    length: usize,
    digest: [u8; 32],
    name: String,
}

pub(super) struct RecoveringReader<'a> {
    pub client: XcpClient,
    pub budget: TransferBudget,
    initial: PreparedInspection,
    limits: &'a CaptureLimits,
    settings: RecoverySettings,
    committed: Vec<CommittedRecord>,
    attempts: u8,
    fast_upload: Option<bool>,
    fast_upload_params: Option<(ByteOrder, u16)>,
    // Retain all old local ports, including failed handshakes, for this capture.
    // retire() prevents their Drop from disconnecting the replacement session.
    retired: Vec<XcpClient>,
}

impl<'a> RecoveringReader<'a> {
    pub fn new(
        client: XcpClient,
        budget: TransferBudget,
        initial: PreparedInspection,
        limits: &'a CaptureLimits,
        settings: RecoverySettings,
    ) -> Self {
        Self {
            client,
            budget,
            initial,
            limits,
            settings,
            committed: Vec::new(),
            attempts: 0,
            fast_upload: if limits.fast_upload && limits.max_reconnects != 0 {
                None
            } else {
                Some(false)
            },
            fast_upload_params: None,
            retired: Vec::new(),
        }
    }

    pub fn initial_info(&self) -> &[u8] {
        &self.initial.info_bytes
    }

    pub fn commit(&mut self, extension: u8, address: u32, name: &str, bytes: &[u8]) {
        self.committed.push(CommittedRecord {
            extension,
            address,
            length: bytes.len(),
            digest: Sha256::digest(bytes).into(),
            name: name.into(),
        });
    }

    pub fn verify_fast_upload(
        &mut self,
        (extension, address): (u8, u32),
        standard: &[u8],
        position: &CaptureProgress,
        diagnostics: &mut Vec<String>,
        progress: &mut impl FnMut(CaptureProgress),
    ) -> Result<(), CaptureErrorKind> {
        let payload = standard.len() - MO_HEADER_SIZE;
        // Never spend the final reconnect slot or exhaust budget needed for
        // remaining metadata on an optional Fast Upload probe.
        let required_budget = (standard.len() as u64)
            .saturating_mul(2)
            .saturating_add(self.initial.info_bytes.len() as u64);
        if self.fast_upload.is_some()
            || self.attempts >= self.limits.max_reconnects
            || !self.client.can_fast_upload(payload)
            || payload <= usize::from(self.client.parameters().max_dto) - 4
            || self.budget.limit.saturating_sub(self.budget.used) < required_budget
        {
            return Ok(());
        }
        let failure = match self.budget.read_record_with_upload(
            &mut self.client,
            extension,
            address,
            standard.len(),
            true,
        ) {
            Ok(bytes) if bytes == standard => {
                self.fast_upload = Some(true);
                let params = self.client.parameters();
                self.fast_upload_params = Some((params.byte_order, params.max_dto));
                diagnostics.push("Verified M1 Fast Upload byte for byte against standard XCP. Using indexed blocks for eligible records.".into());
                return Ok(());
            }
            Ok(_) => "bytes differed from the standard read".into(),
            Err(error)
                if transient(&error)
                    || incompatible_fast_upload(&error)
                    || matches!(
                        error,
                        CaptureErrorKind::Transport(XcpError::CommandRejected {
                            command: "Fast Upload",
                            ..
                        })
                    ) =>
            {
                error.to_string()
            }
            Err(error) => return Err(error),
        };
        self.fast_upload = Some(false);
        diagnostics.push(format!(
            "M1 Fast Upload verification failed ({failure}); reconnecting for standard XCP reads."
        ));
        self.recover(position, diagnostics, progress)
    }

    pub fn read_record(
        &mut self,
        extension: u8,
        address: u32,
        cap: usize,
        position: &CaptureProgress,
        diagnostics: &mut Vec<String>,
        progress: &mut impl FnMut(CaptureProgress),
    ) -> Result<Vec<u8>, CaptureErrorKind> {
        loop {
            let fast = extension != 0x26 && self.fast_upload == Some(true);
            match self.budget.read_record_with_upload(
                &mut self.client,
                extension,
                address,
                cap,
                fast,
            ) {
                Ok(bytes) => return Ok(bytes),
                Err(error) if fast && incompatible_fast_upload(&error) => {
                    self.fast_upload = Some(false);
                    diagnostics.push(format!(
                        "M1 Fast Upload stopped ({error}); reconnecting for standard XCP reads."
                    ));
                    self.recover(position, diagnostics, progress)?;
                }
                Err(error) if transient(&error) => {
                    diagnostics.push(format!("Transient read failure at extension 0x{extension:02x}, selector 0x{address:08x}: {error}"));
                    self.recover(position, diagnostics, progress)?;
                    // Restart from SET_MTA and the record header, discarding the
                    // failed attempt's bytes. No offset arithmetic is inferred.
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn recover(
        &mut self,
        position: &CaptureProgress,
        diagnostics: &mut Vec<String>,
        progress: &mut impl FnMut(CaptureProgress),
    ) -> Result<(), CaptureErrorKind> {
        loop {
            check_cancelled(&self.settings.cancellation)?;
            if self.attempts >= self.limits.max_reconnects {
                return Err(CaptureErrorKind::RecoveryExhausted(self.attempts));
            }
            self.attempts += 1;
            diagnostics.push(format!("Recovery attempt {}/{}: reconnecting, rechecking permissions and comparing saved source records", self.attempts, self.limits.max_reconnects));
            let mut report = position.clone();
            report.stage = CaptureStage::Reconnecting;
            report.bytes = self.budget.used;
            progress(report.clone());
            self.client.retire();
            // Cancellable bounded exponential delay; no busy spin or long sleep.
            let delay = Duration::from_millis((100u64 << (self.attempts - 1)).min(1000));
            let deadline = std::time::Instant::now() + delay;
            loop {
                check_cancelled(&self.settings.cancellation)?;
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
            let result = self.reconnect_and_validate(&report, diagnostics, progress);
            match result {
                Ok(()) => {
                    diagnostics.push(format!("Recovery attempt {} verified unchanged source; restarting interrupted record", self.attempts));
                    let mut resumed = position.clone();
                    resumed.bytes = self.budget.used;
                    progress(resumed);
                    return Ok(());
                }
                Err(error)
                    if self.fast_upload == Some(true) && incompatible_fast_upload(&error) =>
                {
                    self.fast_upload = Some(false);
                    diagnostics.push(format!(
                        "M1 Fast Upload failed during recovery ({error}); falling back to standard XCP reads."
                    ));
                }
                Err(error) if transient(&error) => {
                    diagnostics.push(format!(
                        "Recovery attempt {} failed: {error}",
                        self.attempts
                    ));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn reconnect_and_validate(
        &mut self,
        report: &CaptureProgress,
        diagnostics: &mut Vec<String>,
        progress: &mut impl FnMut(CaptureProgress),
    ) -> Result<(), CaptureErrorKind> {
        // Open while the retired socket is still bound: even a repeated
        // counter cannot make an old reply a reply on the new local endpoint.
        let next = XcpClient::open(
            self.settings.endpoint,
            self.settings.timeout,
            self.settings.cancellation.clone(),
        )?;
        self.retired.push(std::mem::replace(&mut self.client, next));
        self.client.negotiate()?;
        if self.fast_upload == Some(true)
            && let Some(prev_fast_params) = self.fast_upload_params
            && (
                self.client.parameters().byte_order,
                self.client.parameters().max_dto,
            ) != prev_fast_params
        {
            self.fast_upload = Some(false);
            diagnostics.push(
                "Negotiated XCP byte order or max DTO changed on reconnect; disabling Fast Upload for standard XCP reads."
                    .into(),
            );
        }
        let prepared = read_inspection(
            &mut self.client,
            self.settings.endpoint,
            self.limits,
            &mut self.budget,
        )?;
        let info = LogInfo::parse_with_limit(&prepared.info_bytes, self.limits.max_records)?;
        // Recheck rights before touching any logging sectors on this connection.
        selected_plans(
            &info,
            &prepared.summary,
            &self.settings.systems,
            self.limits.max_sectors,
        )?;
        if prepared.info_bytes != self.initial.info_bytes
            || prepared.permissions_bytes != self.initial.permissions_bytes
        {
            return Err(CaptureErrorKind::SourceChanged(
                "logging information or active-user permissions".into(),
            ));
        }
        for record in &self.committed {
            check_cancelled(&self.settings.cancellation)?;
            let remaining_budget = self.budget.limit.saturating_sub(self.budget.used);
            let fast = record.extension != 0x26 && self.fast_upload == Some(true);
            let bytes = match self.budget.read_record_with_upload(
                &mut self.client,
                record.extension,
                record.address,
                record.length,
                fast,
            ) {
                Ok(bytes) => bytes,
                // If budget could cover the committed length, exceeding
                // record.length (or an invalid Mo header) means the source changed.
                Err(CaptureErrorKind::Transport(XcpError::ReadLimitExceeded {
                    requested,
                    limit,
                })) if remaining_budget >= record.length as u64
                    && limit == record.length
                    && requested > record.length =>
                {
                    return Err(CaptureErrorKind::SourceChanged(record.name.clone()));
                }
                Err(CaptureErrorKind::Transport(XcpError::InvalidRecord(_))) => {
                    return Err(CaptureErrorKind::SourceChanged(record.name.clone()));
                }
                Err(error) => return Err(error),
            };
            if bytes.len() != record.length
                || <[u8; 32]>::from(Sha256::digest(&bytes)) != record.digest
            {
                return Err(CaptureErrorKind::SourceChanged(record.name.clone()));
            }
            let mut report = report.clone();
            report.bytes = self.budget.used;
            progress(report);
        }
        Ok(())
    }
}

// Only transport failures that can plausibly be temporary qualify. Malformed
// protocol data, explicit ECU errors, security, budgets and cancellation do not.
fn transient(error: &CaptureErrorKind) -> bool {
    matches!(error, CaptureErrorKind::Transport(XcpError::Timeout))
        || matches!(error, CaptureErrorKind::Transport(XcpError::Io(error)) if matches!(error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::NetworkDown | io::ErrorKind::NetworkUnreachable | io::ErrorKind::HostUnreachable))
}

fn incompatible_fast_upload(error: &CaptureErrorKind) -> bool {
    matches!(
        error,
        CaptureErrorKind::Transport(
            XcpError::CommandRejected {
                command: "Fast Upload",
                code: 0x20 | 0x21,
            } | XcpError::InvalidFastUpload(_)
        )
    )
}

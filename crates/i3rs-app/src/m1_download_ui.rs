//! Native acquisition and conversion UI with bounded background workers.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use egui::Context;
use i3rs_core::m1::{
    capture::{
        CaptureError, CaptureLimits, CaptureProgress, CaptureResult, CaptureStage,
        DeviceInspection, SystemSummary, download_raw, inspect_device,
    },
    conversion::{
        ConversionError, ConversionLimits, ConversionProgress, ConversionResult,
        convert_capture_directory, convert_image_file,
    },
    discovery::{DiscoveryReport, discover},
    transport::CancellationToken,
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

enum Finished {
    Scan(Result<DiscoveryReport, String>),
    Inspect(SocketAddr, Result<DeviceInspection, CaptureError>),
    Download(Result<CaptureResult, CaptureError>),
    Converted(Option<PathBuf>, Result<ConversionResult, ConversionError>),
    WorkerFailed,
}

enum JobEvent {
    Progress(CaptureProgress),
    Converting(ConversionProgress),
    Finished(Box<Finished>),
}

struct Job {
    events: Receiver<JobEvent>,
    cancellation: CancellationToken,
    label: &'static str,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.cancel();
        // Dropping the receiver releases a worker blocked on its final send.
        // Worker handles are detached; the GUI never waits for network cleanup.
    }
}

struct InspectedDevice {
    endpoint: SocketAddr,
    summary: DeviceInspection,
}

struct DownloadSnapshot {
    endpoint: SocketAddr,
    systems: Vec<u8>,
    destination: PathBuf,
}

#[derive(Default)]
pub struct M1DownloadUi {
    pub open: bool,
    endpoint: String,
    discovery: Option<DiscoveryReport>,
    inspected: Option<InspectedDevice>,
    selected: [bool; 8],
    destination: Option<PathBuf>,
    job: Option<Job>,
    progress: Option<CaptureProgress>,
    status: String,
    result_directory: Option<PathBuf>,
    conversion_progress: Option<ConversionProgress>,
    recordings: Vec<PathBuf>,
    pending_open: Option<PathBuf>,
}

impl M1DownloadUi {
    /// Poll workers even while the window is closed, so completion and retained
    /// partial-bundle paths remain available when it is reopened.
    pub fn show(&mut self, ctx: &Context) -> Option<PathBuf> {
        self.poll(ctx);
        if !self.open {
            self.cancel();
            return self.pending_open.take();
        }
        let mut open = self.open;
        egui::Window::new("M1 log download")
            .open(&mut open)
            .default_size([680., 570.])
            .vscroll(true)
            .show(ctx, |ui| self.contents(ui));
        self.open = open;
        if !open {
            self.cancel();
        }
        self.pending_open.take()
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        ui.label("Download M1 logs or convert a saved M1 image into recordings.");
        ui.weak("Experimental: hardware compatibility is unverified. Original raw records are retained. Conversion resamples onto a shared clock; gaps remain missing.");
        ui.separator();
        let busy = self.job.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Scan for devices (5 s)"))
                .clicked()
            {
                self.start_scan(ui.ctx());
            }
        });
        let mut chosen_endpoint = None;
        if let Some(report) = &self.discovery {
            if report.devices.is_empty() {
                ui.label(
                    "No candidates found. Check the Ethernet interface and local network access.",
                );
            }
            for device in &report.devices {
                ui.label(&device.fullname);
                for endpoint in &device.endpoints {
                    let interfaces = endpoint
                        .interfaces
                        .iter()
                        .map(|interface| format!("{} (index {})", interface.name, interface.index))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let label = if interfaces.is_empty() {
                        endpoint.address.to_string()
                    } else {
                        format!("{} · {interfaces}", endpoint.address)
                    };
                    if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                        chosen_endpoint = Some(endpoint.address.to_string());
                    }
                }
                for address in &device.unusable_endpoints {
                    ui.weak(format!(
                        "Unusable address: {address} (missing scope or invalid unicast address)"
                    ));
                }
            }
            for warning in &report.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, warning);
            }
        }
        if let Some(endpoint) = chosen_endpoint {
            self.set_endpoint(endpoint);
        }
        ui.separator();
        ui.label("Device endpoint");
        let response = ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut self.endpoint)
                .hint_text("[IPv6%interface-index]:port")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            self.invalidate_inspection();
        }
        let endpoint = parse_endpoint(&self.endpoint);
        if !self.endpoint.trim().is_empty()
            && let Err(error) = &endpoint
        {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
        if ui
            .add_enabled(
                !busy && endpoint.is_ok(),
                egui::Button::new("Inspect device"),
            )
            .clicked()
            && let Ok(endpoint) = endpoint
        {
            self.start_inspect(endpoint, ui.ctx());
        }
        self.show_systems(ui, busy);
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Choose destination folder…"))
                .clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose a parent folder for M1 logs")
                    .pick_folder()
            {
                self.destination = Some(folder);
            }
            match &self.destination {
                Some(folder) => {
                    ui.label(folder.display().to_string());
                }
                None => {
                    ui.weak("Choose a folder before downloading.");
                }
            }
        });
        let snapshot = self.download_snapshot();
        let reason = snapshot.as_ref().err().cloned();
        if ui
            .add_enabled(
                !busy && snapshot.is_ok(),
                egui::Button::new("Download and convert logs"),
            )
            .clicked()
            && let Ok(snapshot) = snapshot
        {
            self.start_download(snapshot, ui.ctx());
        }
        if !busy && let Some(reason) = reason {
            ui.weak(reason);
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let ready = !busy && self.destination.is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Convert saved M1 image…"))
                .clicked()
                && let Some(source) = rfd::FileDialog::new()
                    .set_title("Choose a saved M1 logged data image")
                    .add_filter("M1 logged data image", &["m1dli"])
                    .pick_file()
            {
                self.start_conversion(source, false, ui.ctx());
            }
            if ui
                .add_enabled(ready, egui::Button::new("Convert raw bundle…"))
                .clicked()
                && let Some(source) = rfd::FileDialog::new()
                    .set_title("Choose a complete i3rs M1 raw bundle")
                    .pick_folder()
            {
                self.start_conversion(source, true, ui.ctx());
            }
        });
        if let Some(job) = &self.job {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(job.label);
                if ui.button("Cancel").clicked() {
                    job.cancellation.cancel();
                }
            });
        }
        if let Some(progress) = &self.progress {
            let stage = match progress.stage {
                CaptureStage::Inspecting => "Inspecting device",
                CaptureStage::ReadingSetups => "Reading logging setups",
                CaptureStage::ReadingMetadata => "Reading metadata",
                CaptureStage::ReadingSector => "Reading logging sectors",
                CaptureStage::Reconnecting => "Reconnecting and verifying saved data",
                CaptureStage::CheckingConsistency => "Checking logging boundaries",
                CaptureStage::Finished => "Raw acquisition finished",
            };
            ui.label(stage);
            ui.label(format!(
                "{} / {} sectors · {} bytes",
                progress.sectors_complete, progress.sectors_total, progress.bytes
            ));
            if let Some(system) = progress.system {
                let sector = progress
                    .sector
                    .map(|sector| format!(" · sector {sector}"))
                    .unwrap_or_default();
                ui.weak(format!("System {}{sector}", u16::from(system) + 1));
            }
        }
        if let Some(progress) = &self.conversion_progress {
            ui.label(format!(
                "{:?} · {} / {} recordings · {} bytes",
                progress.stage,
                progress.files_complete,
                progress.files_total,
                progress.bytes_written
            ));
        }
        if !self.status.is_empty() {
            ui.separator();
            ui.label(&self.status);
        }
        if let Some(directory) = &self.result_directory {
            ui.label(format!("Output folder: {}", directory.display()));
            if ui.button("Copy folder path").clicked() {
                ui.ctx().copy_text(directory.to_string_lossy().into_owned());
            }
        }
        for path in &self.recordings {
            if ui
                .button(format!(
                    "Open {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))
                .clicked()
            {
                self.pending_open = Some(path.clone());
            }
        }
    }

    fn show_systems(&mut self, ui: &mut egui::Ui, busy: bool) {
        let Some(inspected) = &self.inspected else {
            return;
        };
        ui.separator();
        ui.label("Logging systems");
        if !inspected.summary.permissions_available {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Download permissions are unavailable.",
            );
        }
        if inspected.summary.systems.is_empty() {
            ui.weak("The device reported no logging systems.");
        }
        for system in &inspected.summary.systems {
            let Some(selected) = self.selected.get_mut(usize::from(system.id)) else {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Device reported an invalid system ID.",
                );
                continue;
            };
            let eligible = can_download(&inspected.summary, system);
            let detail = system_reason(&inspected.summary, system);
            ui.add_enabled(
                !busy && eligible,
                egui::Checkbox::new(
                    selected,
                    format!("System {} · {detail}", u16::from(system.id) + 1),
                ),
            );
            if let Some(error) = &system.plan_error {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
        }
        if let Some(diagnostic) = &inspected.summary.disconnect_diagnostic {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Disconnect: {diagnostic}"),
            );
        }
    }

    fn set_endpoint(&mut self, endpoint: String) {
        if self.endpoint != endpoint {
            self.endpoint = endpoint;
            self.invalidate_inspection();
        }
    }

    fn invalidate_inspection(&mut self) {
        self.inspected = None;
        self.selected = [false; 8];
    }

    fn download_snapshot(&self) -> Result<DownloadSnapshot, String> {
        let endpoint = parse_endpoint(&self.endpoint)?;
        let inspected = self
            .inspected
            .as_ref()
            .ok_or("Inspect the device before downloading.")?;
        if inspected.endpoint != endpoint
            || inspected.summary.endpoint.parse::<SocketAddr>() != Ok(endpoint)
        {
            return Err("The endpoint changed. Inspect the device again.".into());
        }
        let mut systems = Vec::new();
        let mut sectors = 0u64;
        for (id, selected) in self.selected.iter().enumerate() {
            if !selected {
                continue;
            }
            let system = inspected
                .summary
                .systems
                .iter()
                .find(|system| usize::from(system.id) == id)
                .ok_or("A selected logging system is no longer available.")?;
            if !can_download(&inspected.summary, system) {
                return Err(
                    "A selected logging system cannot be downloaded. Inspect the device again."
                        .into(),
                );
            }
            sectors = sectors
                .checked_add(system.sector_count.unwrap_or_default())
                .ok_or("The selected sector count is too large.")?;
            systems.push(id as u8);
        }
        if systems.is_empty() {
            return Err("Select an available, nonempty logging system.".into());
        }
        if sectors > CaptureLimits::default().max_sectors {
            return Err(
                "Selected systems exceed the raw download sector limit. Select fewer systems."
                    .into(),
            );
        }
        let destination = self
            .destination
            .clone()
            .ok_or("Choose a destination folder.")?;
        Ok(DownloadSnapshot {
            endpoint,
            systems,
            destination,
        })
    }

    fn start_scan(&mut self, ctx: &Context) {
        self.discovery = None;
        self.start_job(
            "Scanning for candidate devices…",
            ctx,
            |cancellation, _, _| {
                Finished::Scan(
                    discover(Duration::from_secs(5), &cancellation)
                        .map_err(|error| error.to_string()),
                )
            },
        );
    }

    fn start_inspect(&mut self, endpoint: SocketAddr, ctx: &Context) {
        self.invalidate_inspection();
        self.start_job("Inspecting device…", ctx, move |cancellation, _, _| {
            Finished::Inspect(
                endpoint,
                inspect_device(
                    endpoint,
                    COMMAND_TIMEOUT,
                    cancellation,
                    &CaptureLimits::default(),
                ),
            )
        });
    }

    fn start_download(&mut self, snapshot: DownloadSnapshot, ctx: &Context) {
        self.result_directory = None;
        self.start_job(
            "Downloading raw ECU records…",
            ctx,
            move |cancellation, tx, ctx| {
                let result = download_raw(
                    snapshot.endpoint,
                    snapshot.systems,
                    &snapshot.destination,
                    &CaptureLimits::default(),
                    COMMAND_TIMEOUT,
                    cancellation.clone(),
                    |progress| {
                        if tx.try_send(JobEvent::Progress(progress)).is_ok() {
                            ctx.request_repaint();
                        }
                    },
                );
                match result {
                    Err(error) => Finished::Download(Err(error)),
                    Ok(raw) => {
                        let converted = convert_capture_directory(
                            &raw.directory,
                            &snapshot.destination,
                            &ConversionLimits::default(),
                            cancellation,
                            |progress| {
                                if tx.try_send(JobEvent::Converting(progress)).is_ok() {
                                    ctx.request_repaint();
                                }
                            },
                        );
                        Finished::Converted(Some(raw.directory), converted)
                    }
                }
            },
        );
    }

    fn start_conversion(&mut self, source: PathBuf, bundle: bool, ctx: &Context) {
        let Some(destination) = self.destination.clone() else {
            return;
        };
        self.result_directory = None;
        self.recordings.clear();
        self.start_job(
            "Converting M1 logs…",
            ctx,
            move |cancellation, tx, ctx| {
                let progress = |progress| {
                    if tx.try_send(JobEvent::Converting(progress)).is_ok() {
                        ctx.request_repaint();
                    }
                };
                let result = if bundle {
                    convert_capture_directory(
                        &source,
                        &destination,
                        &ConversionLimits::default(),
                        cancellation,
                        progress,
                    )
                } else {
                    convert_image_file(
                        &source,
                        &destination,
                        &ConversionLimits::default(),
                        cancellation,
                        progress,
                    )
                };
                Finished::Converted(bundle.then_some(source), result)
            },
        );
    }

    fn start_job(
        &mut self,
        label: &'static str,
        ctx: &Context,
        operation: impl FnOnce(CancellationToken, &SyncSender<JobEvent>, &Context) -> Finished
        + Send
        + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        let (tx, events) = mpsc::sync_channel(32);
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let ctx = ctx.clone();
        self.status.clear();
        self.progress = None;
        self.conversion_progress = None;
        self.recordings.clear();
        match std::thread::Builder::new()
            .name("i3rs-m1-download".into())
            .spawn(move || {
                let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    operation(worker_cancellation, &tx, &ctx)
                }))
                .unwrap_or(Finished::WorkerFailed);
                // Progress may be dropped under pressure; completion is delivered
                // reliably unless the UI/receiver has itself been dropped.
                let _ = tx.send(JobEvent::Finished(Box::new(finished)));
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.job = Some(Job {
                    events,
                    cancellation,
                    label,
                })
            }
            Err(error) => self.status = format!("Could not start download worker: {error}"),
        }
    }

    fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancellation.cancel();
        }
    }

    fn poll(&mut self, ctx: &Context) {
        for _ in 0..32 {
            let Some(job) = &self.job else {
                break;
            };
            match job.events.try_recv() {
                Ok(JobEvent::Progress(progress)) => self.progress = Some(progress),
                Ok(JobEvent::Converting(progress)) => {
                    self.progress = None;
                    self.conversion_progress = Some(progress);
                }
                Ok(JobEvent::Finished(finished)) => {
                    self.job = None;
                    self.finish(*finished);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.status =
                        "The acquisition worker stopped before returning a result.".into();
                    break;
                }
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn finish(&mut self, finished: Finished) {
        match finished {
            Finished::Scan(Ok(report)) => {
                self.status = format!(
                    "Scan finished: {} candidate advertisement(s).",
                    report.devices.len()
                );
                self.discovery = Some(report);
            }
            Finished::Scan(Err(error)) => self.status = error,
            Finished::Inspect(endpoint, Ok(summary)) => {
                if parse_endpoint(&self.endpoint) != Ok(endpoint)
                    || summary.endpoint.parse::<SocketAddr>() != Ok(endpoint)
                {
                    self.status = "Inspection discarded because the endpoint changed.".into();
                    return;
                }
                self.selected = [false; 8];
                for system in &summary.systems {
                    if can_download(&summary, system) {
                        self.selected[usize::from(system.id)] = true;
                    }
                }
                self.inspected = Some(InspectedDevice { endpoint, summary });
                self.status =
                    "Inspection finished. Choose systems and a destination folder.".into();
            }
            Finished::Inspect(_, Err(error)) | Finished::Download(Err(error)) => {
                self.result_directory = error.bundle_directory.clone();
                self.status = error.to_string();
            }
            Finished::Download(Ok(result)) => {
                self.status = format!(
                    "Raw bundle saved: {} records, {} bytes. This bundle cannot be opened as an .ld recording.",
                    result.manifest.files.len(),
                    result.manifest.total_bytes
                );
                if !result.manifest.diagnostics.is_empty() {
                    self.status
                        .push_str(&format!("\n{}", result.manifest.diagnostics.join("\n")));
                }
                self.result_directory = Some(result.directory);
            }
            Finished::Converted(raw, result) => {
                self.progress = None;
                match result {
                    Ok(result) => {
                        self.status =
                            format!("Converted {} recording(s).", result.manifest.files.len());
                        for warning in &result.manifest.warnings {
                            self.status.push_str(&format!("\n{warning}"));
                        }
                        self.recordings = result
                            .manifest
                            .files
                            .iter()
                            .filter(|file| file.complete)
                            .map(|file| result.directory.join(&file.file_name))
                            .collect();
                        self.result_directory = Some(result.directory);
                    }
                    Err(error) => {
                        self.result_directory = error.directory.clone().or_else(|| raw.clone());
                        self.status = format!("Conversion failed: {error}");
                    }
                }
                if let Some(raw) = raw {
                    self.status
                        .push_str(&format!("\nOriginal raw bundle: {}", raw.display()));
                }
            }
            Finished::WorkerFailed => {
                self.invalidate_inspection();
                self.status = "The acquisition worker failed unexpectedly. Inspect the device again before retrying.".into();
            }
        }
    }
}

fn can_download(summary: &DeviceInspection, system: &SystemSummary) -> bool {
    summary.permissions_available
        && summary
            .download_allowed
            .get(usize::from(system.id))
            .copied()
            .unwrap_or(false)
        && system.plan_error.is_none()
        && system.sector_count.is_some_and(|count| count > 0)
}

fn system_reason(summary: &DeviceInspection, system: &SystemSummary) -> String {
    if !summary.permissions_available {
        return "permissions unavailable".into();
    }
    if !summary
        .download_allowed
        .get(usize::from(system.id))
        .copied()
        .unwrap_or(false)
    {
        return "download denied".into();
    }
    if system.plan_error.is_some() {
        return "read plan unavailable".into();
    }
    match system.sector_count {
        Some(0) => "empty".into(),
        Some(count) => format!("{count} sectors"),
        None => "sector count unavailable".into(),
    }
}

fn parse_endpoint(text: &str) -> Result<SocketAddr, String> {
    let endpoint = text.trim().parse::<SocketAddr>().map_err(|_| {
        "Enter an IP address and port; use a numeric interface index for scoped IPv6.".to_string()
    })?;
    if endpoint.port() == 0 {
        return Err("The endpoint port must be greater than zero.".into());
    }
    if let SocketAddr::V6(address) = endpoint
        && let Some(ip) = address.ip().to_ipv4_mapped()
        && (ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast())
    {
        return Err("Choose a unicast device address.".into());
    }
    match endpoint {
        SocketAddr::V4(address) if address.ip().is_unspecified() || address.ip().is_multicast() || address.ip().is_broadcast() => {
            Err("Choose a unicast device address.".into())
        }
        SocketAddr::V6(address) if address.ip().is_unspecified() || address.ip().is_multicast() => {
            Err("Choose a unicast device address.".into())
        }
        SocketAddr::V6(address) if address.ip().is_unicast_link_local() && address.scope_id() == 0 => {
            Err("A link-local IPv6 endpoint needs its interface index, for example [fe80::1%4]:port.".into())
        }
        _ => Ok(endpoint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_failures_keep_recovery_paths_and_open_requests_are_consumed_once() {
        let raw = PathBuf::from("/raw-capture");
        let partial = PathBuf::from("/partial-conversion");
        let mut ui = M1DownloadUi::default();
        for output in [None, Some(partial.clone())] {
            ui.finish(Finished::Converted(
                Some(raw.clone()),
                Err(ConversionError {
                    message: "unsupported recording".into(),
                    directory: output.clone(),
                    manifest_diagnostic: None,
                }),
            ));
            assert_eq!(ui.result_directory, output.or_else(|| Some(raw.clone())));
            assert!(ui.status.contains("/raw-capture"));
            assert!(ui.recordings.is_empty());
        }
        let recording = PathBuf::from("/converted/system-1.ld");
        ui.pending_open = Some(recording.clone());
        let ctx = Context::default();
        assert_eq!(ui.show(&ctx), Some(recording));
        assert_eq!(ui.show(&ctx), None);
    }

    fn inspected_ui() -> M1DownloadUi {
        let endpoint: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let summary = DeviceInspection {
            endpoint: endpoint.to_string(),
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
        M1DownloadUi {
            endpoint: endpoint.to_string(),
            inspected: Some(InspectedDevice { endpoint, summary }),
            selected: [true, false, false, false, false, false, false, false],
            destination: Some(std::env::temp_dir()),
            ..Default::default()
        }
    }

    #[test]
    fn endpoint_validation_requires_unicast_port_and_ipv6_link_local_scope() {
        for invalid in [
            "",
            "192.0.2.1",
            "0.0.0.0:1",
            "224.0.0.1:1",
            "255.255.255.255:1",
            "[::]:1",
            "[ff02::1%4]:1",
            "[fe80::1]:1",
            "[fe80::1%0]:1",
            "[::ffff:224.0.0.1]:1",
            "127.0.0.1:0",
        ] {
            assert!(parse_endpoint(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            parse_endpoint("[fe80::1%7]:12345").unwrap(),
            "[fe80::1%7]:12345".parse().unwrap()
        );
        assert!(parse_endpoint("127.0.0.1:12345").is_ok());
    }

    #[test]
    fn stale_inspection_permissions_and_empty_plans_cannot_start_downloads() {
        let mut ui = inspected_ui();
        assert_eq!(ui.download_snapshot().unwrap().systems, [0]);
        ui.endpoint = "127.0.0.2:12345".into();
        assert!(ui.download_snapshot().is_err());
        ui.endpoint = "127.0.0.1:12345".into();
        ui.inspected.as_mut().unwrap().summary.download_allowed[0] = false;
        assert!(ui.download_snapshot().is_err());
        ui.inspected.as_mut().unwrap().summary.download_allowed[0] = true;
        ui.inspected.as_mut().unwrap().summary.permissions_available = false;
        assert!(ui.download_snapshot().is_err());
        ui.inspected.as_mut().unwrap().summary.permissions_available = true;
        ui.inspected.as_mut().unwrap().summary.systems[0].plan_error = Some("invalid range".into());
        assert!(ui.download_snapshot().is_err());
        ui.inspected.as_mut().unwrap().summary.systems[0].plan_error = None;
        ui.inspected.as_mut().unwrap().summary.systems[0].sector_count = Some(0);
        assert!(ui.download_snapshot().is_err());
        ui.set_endpoint("127.0.0.2:12345".into());
        assert!(ui.inspected.is_none());
        assert_eq!(ui.selected, [false; 8]);
    }
}

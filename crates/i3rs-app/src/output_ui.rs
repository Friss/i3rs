//! Log discovery and output dialogs consume the same recording/range models as analysis.
use crate::{comparison::recording_context, state::SharedState};
use egui::{self, Context};
use i3rs_core::{
    Interpolation, LdFile,
    log_index::LogIndex,
    output::{OutputPlan, OutputSeries, earth_kml, series_csv},
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, mpsc},
};

enum OutputJobResult {
    Preview(Result<(String, String, String), String>),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Written(Result<String, String>),
}
#[derive(Default)]
pub struct OutputUi {
    pub browser_open: bool,
    pub export_open: bool,
    index: LogIndex,
    query: String,
    folder: Option<PathBuf>,
    scan: Option<mpsc::Receiver<(LogIndex, Vec<String>)>>,
    pending_main: Option<(PathBuf, i3rs_core::ranges::RecordingAnalysis, Option<usize>)>,
    initialized: bool,
    output_job: Option<mpsc::Receiver<OutputJobResult>>,
    pub capture_requested: bool,
    capture_image: Option<Arc<egui::ColorImage>>,
    capture_texture: Option<egui::TextureHandle>,
    capture_open: bool,
    source: usize,
    scope: usize,
    format: usize,
    rate: f64,
    start: f64,
    end: f64,
    relative: bool,
    include_time: bool,
    display: bool,
    distance: bool,
    derived: bool,
    channels: BTreeSet<String>,
    channel_query: String,
    preview: Option<(String, String, String)>, // immutable label, extension, bytes
    status: String,
    browser_status: String,
}
impl OutputUi {
    pub fn begin_export(&mut self, shared: &SharedState) {
        self.export_open = true;
        self.rate = 20.;
        self.source = 0;
        self.scope = if shared.selected_lap.is_some() { 1 } else { 0 };
        self.relative = true;
        self.include_time = true;
        self.display = true;
        self.start = shared.analysis_range().0;
        self.end = shared.analysis_range().1;
        self.derived = false;
        self.format = 0;
        self.preview = None;
        self.status.clear();
        self.channels = shared
            .display_channel_registry
            .iter()
            .filter_map(|info| match info.channel_id {
                crate::state::ChannelId::Physical(i) => shared
                    .ld_file
                    .as_ref()?
                    .channels
                    .get(i)
                    .map(|c| c.name.clone()),
                _ => None,
            })
            .collect();
        if self.channels.is_empty()
            && let Some(ld) = &shared.ld_file
        {
            self.channels = ld.channels.iter().map(|c| c.name.clone()).collect();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn index_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let base = if cfg!(target_os = "macos") {
            home?.join("Library/Application Support")
        } else if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)?
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.map(|p| p.join(".config")))?
        };
        Some(base.join("i3rs/log-index.json"))
    }
    fn scan(&mut self, folder: PathBuf, ctx: &Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = mpsc::channel();
            let mut index = self.index.clone();
            let ctx = ctx.clone();
            self.scan = Some(rx);
            std::thread::spawn(move || {
                let errors = index.scan(&folder);
                let _ = tx.send((index, errors));
                ctx.request_repaint();
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (folder, ctx);
            self.browser_status = "Folder indexing is available in the desktop app".into();
        }
    }
    pub fn show(&mut self, ctx: &Context, shared: &mut SharedState) -> Option<PathBuf> {
        self.capture_events(ctx);
        if let Some(rx) = &self.output_job {
            match rx.try_recv() {
                Ok(result) => {
                    self.output_job = None;
                    match result {
                        OutputJobResult::Preview(Ok(snapshot)) => {
                            self.preview = Some(snapshot);
                            self.status =
                                "Preview ready. Copy and export use this exact snapshot.".into();
                        }
                        OutputJobResult::Written(Ok(message)) => self.status = message,
                        OutputJobResult::Preview(Err(e)) | OutputJobResult::Written(Err(e)) => {
                            self.status = e
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.output_job = None;
                    self.status = "Output worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.initialized {
            self.initialized = true;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = Self::index_path() {
                match std::fs::read(&path) {
                    Ok(bytes) => match serde_json::from_slice(&bytes) {
                        Ok(index) => self.index = index,
                        Err(e) => self.browser_status = format!("Could not read log index: {e}"),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => self.browser_status = e.to_string(),
                }
            }
        }
        if let Some((path, analysis, range)) = &self.pending_main
            && shared.ld_path.as_ref() == Some(path)
            && shared
                .ld_file
                .as_ref()
                .is_some_and(|ld| ld.recording_id() == analysis.recording)
        {
            let analysis = analysis.clone();
            let range = *range;
            self.pending_main = None;
            if let Err(e) = shared.apply_recording_analysis(analysis) {
                self.browser_status = e;
            } else {
                shared.select_lap(range);
                shared.comparisons.main_lap = range;
            }
        }
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok((index, errors)) => {
                    self.index = index;
                    self.scan = None;
                    self.browser_status = if errors.is_empty() {
                        format!("{} recordings indexed", self.index.logs.len())
                    } else {
                        errors.join("\n")
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(path) = Self::index_path() {
                        let result = (|| -> Result<(), String> {
                            std::fs::create_dir_all(path.parent().unwrap())
                                .map_err(|e| e.to_string())?;
                            let bytes =
                                serde_json::to_vec(&self.index).map_err(|e| e.to_string())?;
                            let tmp = path.with_extension("tmp");
                            std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
                            std::fs::rename(tmp, path).map_err(|e| e.to_string())
                        })();
                        if let Err(e) = result {
                            self.browser_status = format!("Index persistence: {e}");
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.browser_status = "Index worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut action = None;
        let mut scan_folder = None;
        let mut open = self.browser_open;
        if open {
            egui::Window::new("Log Browser")
                .open(&mut open)
                .default_size([960., 600.])
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Filter driver / venue / vehicle / date / range");
                        ui.text_edit_singleline(&mut self.query);
                        #[cfg(not(target_arch = "wasm32"))]
                        if ui
                            .add_enabled(self.scan.is_none(), egui::Button::new("Add folder…"))
                            .clicked()
                            && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        {
                            self.folder = Some(folder.clone());
                            scan_folder = Some(folder);
                        }
                        if ui
                            .add_enabled(self.scan.is_none(), egui::Button::new("Refresh"))
                            .clicked()
                        {
                            scan_folder = self
                                .folder
                                .clone()
                                .or_else(|| self.index.folders.first().cloned());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Folders:");
                        if ui.selectable_label(self.folder.is_none(), "All").clicked() {
                            self.folder = None;
                        }
                        for f in &self.index.folders {
                            if ui
                                .selectable_label(
                                    self.folder.as_ref() == Some(f),
                                    f.file_name().unwrap_or(f.as_os_str()).to_string_lossy(),
                                )
                                .on_hover_text(f.display().to_string())
                                .clicked()
                            {
                                self.folder = Some(f.clone());
                            }
                        }
                    });
                    if self.scan.is_some() {
                        ui.spinner();
                        ui.label("Indexing recording metadata and ranges…");
                    }
                    ui.label(&self.browser_status);
                    egui::ScrollArea::both().show(ui, |ui| {
                        for log in self.index.logs.values().filter(|l| {
                            l.matches(&self.query)
                                && self.folder.as_ref().is_none_or(|f| l.path.starts_with(f))
                        }) {
                            egui::CollapsingHeader::new(format!(
                                "{} {} · {} · {} · {} · {:.3}s",
                                log.date,
                                log.time,
                                log.driver,
                                log.venue,
                                log.path.file_name().unwrap_or_default().to_string_lossy(),
                                log.duration
                            ))
                            .id_salt(&log.path)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "{} · {} · {} channels · {} bytes",
                                    log.vehicle, log.event, log.channels, log.size
                                ));
                                ui.weak(log.path.display().to_string());
                                ui.label(&log.analysis.details.notes);
                                for (k, v) in &log.analysis.details.custom {
                                    ui.label(format!("{k}: {v}"));
                                }
                                if ui.button("Open full recording as Main").clicked() {
                                    action = Some((log.clone(), None, false));
                                }
                                for (i, r) in log.analysis.ranges.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(format!(
                                            "{} · {} · {:?} · {:.3}–{:.3}s",
                                            r.name,
                                            r.kind.label(),
                                            r.trust,
                                            r.start,
                                            r.end
                                        ));
                                        if ui.button("Main").clicked() {
                                            action = Some((log.clone(), Some(i), false));
                                        }
                                        if ui
                                            .add_enabled(
                                                shared.ld_file.is_some(),
                                                egui::Button::new("Overlay"),
                                            )
                                            .clicked()
                                        {
                                            action = Some((log.clone(), Some(i), true));
                                        }
                                    });
                                }
                            });
                        }
                    });
                });
        }
        self.browser_open = open;
        if let Some(folder) = scan_folder {
            self.scan(folder, ctx);
        }
        let mut main = None;
        if let Some((log, range, overlay)) = action {
            if overlay {
                match self.add_overlay(shared, &log, range.unwrap()) {
                    Ok(()) => self.browser_status = "Selected range added as Overlay".into(),
                    Err(e) => self.browser_status = e,
                }
            } else {
                let analysis = shared
                    .snapshot_recording_analysis()
                    .get(&log.analysis.recording)
                    .cloned()
                    .unwrap_or(log.analysis);
                self.pending_main = Some((log.path.clone(), analysis, range));
                main = Some(log.path);
            }
        }
        self.export_window(ctx, shared);
        main
    }
    fn add_overlay(
        &self,
        shared: &mut SharedState,
        log: &i3rs_core::log_index::IndexedLog,
        range: usize,
    ) -> Result<(), String> {
        use crate::panels::graph::{LapOverlay, OverlaySession, OverlaySource};
        let ld = Arc::new(LdFile::open(&log.path)?);
        if ld.recording_id() != log.analysis.recording {
            return Err("Recording changed; refresh the index".into());
        }
        let analysis = shared
            .recording_analyses
            .entry(log.analysis.recording.clone())
            .or_insert_with(|| log.analysis.clone())
            .clone();
        let range_id = &log
            .analysis
            .ranges
            .get(range)
            .ok_or("Selected range is unavailable")?
            .id;
        let range = analysis
            .ranges
            .iter()
            .position(|r| &r.id == range_id)
            .ok_or("This range was edited or removed; refresh the browser")?;
        let idx = if let Some(i) = shared
            .comparisons
            .sessions
            .iter()
            .position(|s| s.path == log.path)
        {
            shared.comparisons.sessions[i].laps = analysis.laps();
            i
        } else {
            let i = shared.comparisons.sessions.len();
            shared.comparisons.sessions.push(OverlaySession {
                settings_key: String::new(),
                requests: Default::default(),
                failures: Default::default(),
                path: log.path.clone(),
                file_name: log
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                ld_file: ld,
                laps: analysis.laps(),
                distance_axis_cache: None,
                channel_cache: Default::default(),
            });
            i
        };
        shared.comparisons.overlays.push(LapOverlay {
            alignment_valid: true,
            gps_align: false,
            visible: true,
            color: None,
            source: OverlaySource::External(idx),
            lap_idx: range,
            manual_offset: 0.,
            stretch_to_reference: false,
        });
        Ok(())
    }
    fn export_window(&mut self, ctx: &Context, shared: &mut SharedState) {
        let mut open = self.export_open;
        if !open {
            return;
        }
        egui::Window::new("Export Data / Split Log").open(&mut open).default_size([720.,660.]).vscroll(true).show(ctx,|ui|{
            let sources=std::iter::once("Main".to_owned()).chain(shared.comparisons.overlays.iter().enumerate().map(|(i,_)|format!("Overlay {}",i+1))).collect::<Vec<_>>();
            self.source=self.source.min(sources.len()-1);
            ui.horizontal(|ui|{ui.label("Source");egui::ComboBox::from_id_salt("output_source").selected_text(&sources[self.source]).show_ui(ui,|ui|{for(i,s)in sources.iter().enumerate(){ui.selectable_value(&mut self.source,i,s);}});});
            let Some((ld,analysis,selected))=source(shared,self.source) else{ui.label("Source recording or selected range is unavailable");return;};
            #[cfg(target_arch="wasm32")] let _=&analysis;
            ui.horizontal_wrapped(|ui|{ui.label("Scope");for(i,s)in ["Full recording","Selected range","Zoom","Custom"].iter().enumerate(){ui.selectable_value(&mut self.scope,i,*s);}});
            if self.scope==1 && selected.is_none(){ui.label("Choose a Main/Overlay range before exporting Selected range.");return;}
            if self.scope==2 && self.source!=0{ui.label("Zoom belongs to Main. Choose Selected range or Custom bounds for this Overlay.");return;}
            let bounds=match self.scope{0=>(0.,ld.duration_secs()),1=>selected.unwrap_or((0.,ld.duration_secs())),2 if self.source==0=>shared.analysis_range(),_ =>(self.start,self.end)};
            if self.scope==3{ui.horizontal(|ui|{ui.label("Start / end [s]");ui.add(egui::DragValue::new(&mut self.start).speed(0.1));ui.add(egui::DragValue::new(&mut self.end).speed(0.1));});}
            ui.label(format!("Scope: [{:.6}, {:.6}) seconds",bounds.0,bounds.1));
            ui.horizontal_wrapped(|ui|{ui.label("Format");for(i,label)in ["CSV","MoTeC .ld","Reference lap .ld","Earth KML","Device reference .td1"].iter().enumerate(){ui.selectable_value(&mut self.format,i,*label);}});
            let raw_log=matches!(self.format,1|2);
            let device_reference=self.format==4;
            #[cfg(target_arch="wasm32")]
            if raw_log || device_reference{ui.weak("Log/reference writing is available in the desktop app.");}
            if !device_reference {ui.horizontal_wrapped(|ui|{ui.label("Output Hz");ui.add(egui::DragValue::new(&mut self.rate).range(1. ..=65535.).speed(1.));if self.format==0{ui.checkbox(&mut self.include_time,"Time column");ui.checkbox(&mut self.relative,"Relative time");ui.checkbox(&mut self.display,"Display units");ui.checkbox(&mut self.distance,"Distance column [m]");}});}
            if raw_log{ui.weak("Logged channels only; source units. Full recording preserves native bytes/rates. Crops use the chosen integral Hz, linear continuous / previous state values, rebased time. Analysis ranges/details travel in paired sidecars. All logged channels are retained.");}
            else if device_reference{ui.weak("TD1 v1: selected range; built-in corrected distance in metres, one-metre spatial grid and final endpoint. Time rounded to 0.01 s, distance to 0.1 m. No user maths/display transforms. ASCII metadata. Layout verified against i2; physical device acceptance has not been tested.");}
            else if self.format==3{ui.weak("GPS latitude/longitude in degrees; selected scope at chosen Hz. Invalid positions break the path. No derived channels.");}
            else{ui.weak("CSV: chosen channels, per-channel linear/previous interpolation, selection-anchored rate, [start,end). Missing values are empty. Derived results are opt-in.");}
            if self.format==2{ui.weak("Selected range becomes a reusable comparison log. This .ld output is not a MoTeC device .td1 prediction file.");}
            if self.format==0{
                ui.checkbox(&mut self.derived,"Include derived channels / results");
                ui.horizontal(|ui|{ui.label("Channels");ui.text_edit_singleline(&mut self.channel_query);if ui.button("All logged").clicked(){self.channels=ld.channels.iter().map(|c|c.name.clone()).collect();}
if ui.button("None").clicked(){self.channels.clear();}});
                egui::ScrollArea::vertical().id_salt("output_channels").max_height(190.).show(ui,|ui|{
                    for (name,label) in ld.channels.iter().map(|c|(c.name.clone(),c.name.clone())).chain(shared.math_channels.iter().filter(|c|self.derived && c.options.recording.as_deref().is_none_or(|r|r==ld.recording_id())).map(|c|(format!("Math::{}",c.id),format!("{}::{} [derived]",c.options.source,c.name)))){if label.to_lowercase().contains(&self.channel_query.to_lowercase()){let mut enabled=self.channels.contains(&name);if ui.checkbox(&mut enabled,&label).changed(){if enabled{self.channels.insert(name);}else{self.channels.remove(&name);}}}}
                });
            }
            let plan=OutputPlan{start:bounds.0,end:bounds.1,rate:self.rate,relative_time:self.relative,include_time:self.include_time};
            ui.horizontal_wrapped(|ui|{
                if !raw_log && !device_reference && ui.add_enabled(self.output_job.is_none(), egui::Button::new("Preview output")).clicked(){
                    let mut context=recording_context(shared,ld.clone());
                    context.channel_preferences=shared.channel_preferences.clone();
                    context.display_channel_registry=shared.display_channel_registry.clone();
                    context.recording_analyses=shared.snapshot_recording_analysis();
                    let task=Self { source:self.source, format:self.format, channels:self.channels.clone(), derived:self.derived, distance:self.distance, display:self.display, ..Default::default() };
                    let label=format!("{} · {} · {:.6}–{:.6}s · {} Hz",sources[self.source],ld.session.venue,bounds.0,bounds.1,self.rate);
                    let ld=ld.clone();self.start_job(ctx,move || OutputJobResult::Preview(task.make_preview(&context,ld,plan).map(|(ext,content)|(label,ext,content))));
                }
                #[cfg(not(target_arch="wasm32"))]
                if device_reference && ui.add_enabled(self.output_job.is_none(),egui::Button::new("Save device reference…")).clicked(){
                    if selected.is_none() || self.scope!=1 {self.status="Device reference requires Selected range scope".into();}
                    else if let Some(path)=rfd::FileDialog::new().add_filter("MoTeC reference lap", &["td1"]).set_file_name("reference.td1").save_file(){
                        let mut context=recording_context(shared,ld.clone());
                        context.recording_analyses=shared.snapshot_recording_analysis();
                        let ld=ld.clone();let analysis=analysis.clone();
                        self.start_job(ctx,move || OutputJobResult::Written(make_device_reference(&context,ld,&analysis,bounds).and_then(|lap|{
                            let backup=lap.write(&path)?;
                            Ok(format!("Saved {} · {} spatial points · {:.2} s · {:.1} m{}",path.display(),lap.time_cs.len(),*lap.time_cs.last().unwrap() as f64/100.,*lap.distance_dm.last().unwrap() as f64/10.,backup.map(|p|format!("; recovery backup: {}",p.display())).unwrap_or_default()))
                        })));
                    }
                }
                #[cfg(not(target_arch="wasm32"))]
                if raw_log && ui.add_enabled(self.output_job.is_none(),egui::Button::new(if self.format==2{"Save reference lap…"}else{"Write log / split…"})).clicked(){
                    if self.format==2 && (selected.is_none() || self.scope!=1){self.status="Reference lap output requires Selected range scope".into();}
                    else if let Some(path)=rfd::FileDialog::new().add_filter("MoTeC log",&["ld"]).set_file_name(if self.format==2{"reference.ld"}else{"export.ld"}).save_file(){
                        let selection=if self.scope==0{None}else{Some(plan)};
                        let ld=ld.clone();let analysis=analysis.clone();self.start_job(ctx,move || OutputJobResult::Written(i3rs_core::log_writer::write_log(&path,&ld,&analysis,selection).map(|backup|format!("Saved {}{}",path.display(),backup.map(|p|format!("; recovery backup: {}",p.display())).unwrap_or_default()))));
                    }
                }
                #[cfg(not(target_arch="wasm32"))]
                if raw_log && self.source==0 && self.scope==0 && ui.add_enabled(self.output_job.is_none(),egui::Button::new("Write details/ranges back (with backup)")).clicked()
                    && let Some(path)=shared.ld_path.clone(){let ld=ld.clone();let analysis=analysis.clone();self.start_job(ctx,move || OutputJobResult::Written(i3rs_core::log_writer::write_log(&path,&ld,&analysis,None).map(|backup|format!("Updated log and sidecars. Reopen to refresh. Recovery backup: {}",backup.map(|p|p.display().to_string()).unwrap_or_default()))));}
            });
            if self.output_job.is_some(){ui.spinner();}
            ui.label(&self.status);
            if let Some((label,ext,content))=&self.preview{
                ui.separator();ui.label(label);ui.weak("Saved preview snapshot; change options and Preview output to regenerate.");
                if ext=="csv"{result_buttons(ui,content);}else{
                    if ui.button("Copy KML").clicked(){ui.ctx().copy_text(content.clone());}
                    #[cfg(not(target_arch="wasm32"))]
                    if ui.button("Save KML…").clicked()&& let Some(path)=rfd::FileDialog::new().add_filter("Earth KML",&["kml"]).set_file_name("track.kml").save_file()&& let Err(e)=std::fs::write(path,content){self.status=e.to_string();}
                }
                egui::ScrollArea::both().max_height(180.).show(ui,|ui|{ui.monospace(content.chars().take(16000).collect::<String>());});ui.weak(format!("{} bytes · first 16,000 characters shown",content.len()));
            }
        });
        self.export_open = open;
    }
    fn start_job(
        &mut self,
        ctx: &Context,
        work: impl FnOnce() -> OutputJobResult + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.output_job = Some(rx);
        self.status = "Preparing output…".into();
        let ctx = ctx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        #[cfg(target_arch = "wasm32")]
        {
            let _ = tx.send(work());
            ctx.request_repaint();
        }
    }
    fn capture_events(&mut self, ctx: &Context) {
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot {
                user_data, image, ..
            } = event
                && user_data
                    .data
                    .as_ref()
                    .and_then(|d| d.downcast_ref::<String>())
                    .is_some_and(|s| s == "i3rs-print")
            {
                self.capture_texture = Some(ctx.load_texture(
                    "worksheet-print",
                    (*image).clone(),
                    egui::TextureOptions::LINEAR,
                ));
                self.capture_image = Some(image);
                self.capture_open = true;
            }
        }
        egui::Window::new("Worksheet Print Preview").open(&mut self.capture_open).default_size([950.,700.]).show(ctx,|ui|{
            ui.weak("Snapshot of the visible worksheet, including charts. The saved page prints this exact image.");
            if let (Some(texture),Some(image))=(&self.capture_texture,&self.capture_image){
                if ui.button("Copy image").clicked(){ui.ctx().copy_image((**image).clone());}
                #[cfg(not(target_arch="wasm32"))]
                if ui.button("Save printable HTML…").clicked()&& let Some(path)=rfd::FileDialog::new().add_filter("HTML",&["html"]).set_file_name("worksheet.html").save_file(){match printable_image(image).and_then(|s|std::fs::write(&path,s).map_err(|e|e.to_string())){Ok(())=>open_print_page(&path),Err(e)=>{ui.label(e);}}}
                egui::ScrollArea::both().show(ui,|ui|{ui.add(egui::Image::from_texture(texture).fit_to_original_size(1.0).max_width(ui.available_width()));});
            }
        });
    }
    fn make_preview(
        &self,
        shared: &SharedState,
        ld: Arc<LdFile>,
        plan: OutputPlan,
    ) -> Result<(String, String), String> {
        let mut context = recording_context(shared, ld.clone());
        context.channel_preferences = shared.channel_preferences.clone();
        if self.format == 3 {
            let settings = context.current_corrections();
            let (lat, lon) =
                if settings.latitude_channel.is_empty() && settings.longitude_channel.is_empty() {
                    i3rs_core::track::gps_channels(&ld)
                } else {
                    ld.find_channel_by_name(&settings.latitude_channel)
                        .zip(ld.find_channel_by_name(&settings.longitude_channel))
                }
                .ok_or("Configured GPS latitude/longitude sources are unavailable")?;
            let lat = logged_series(&context, lat, false)?;
            let lon = logged_series(&context, lon, false)?;
            if !["deg", "°", ""].contains(&lon.unit.as_str()) {
                return Err("GPS longitude must be in degrees".into());
            }
            if !["deg", "°", ""].contains(&lat.unit.as_str()) {
                return Err("GPS latitude must be in degrees".into());
            }
            return Ok((
                "kml".into(),
                earth_kml(&ld.session.venue, plan, &lat, &lon)?,
            ));
        }
        let mut channels = vec![];
        for name in &self.channels {
            if let Some(id) = name.strip_prefix("Math::") {
                if !self.derived {
                    continue;
                }
                let id = id
                    .parse::<u64>()
                    .map_err(|_| "Invalid math channel identity")?;
                let def = shared
                    .math_channels
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or("Unknown math channel")?;
                let reference = format!("{}::{}", def.options.source, def.name);
                match context.resolve_analysis_reference(&reference) {
                    i3rs_core::ChannelResolution::Resolved {
                        key: i3rs_core::ChannelKey::Derived(resolved),
                        ..
                    } if resolved == id => {}
                    _ => {
                        return Err(format!(
                            "{} is unavailable or shadowed in this recording",
                            def.name
                        ));
                    }
                }
                let (data, rate, unit) = crate::comparison::evaluate_channel(&context, &reference)?;
                let name = &def.name;
                channels.push(OutputSeries {
                    name: name.clone(),
                    unit,
                    data,
                    rate,
                    precision: def.dec_places.clamp(0, 15) as usize,
                    interpolation: def.options.interpolation,
                    scale: 1.,
                    offset: 0.,
                });
                if self.display {
                    let series = channels.last_mut().unwrap();
                    if let Some(pref) = shared
                        .channel_preferences
                        .get(&crate::state::channel_preference_key(name))
                    {
                        series.scale = pref.display_scale;
                        series.offset = pref.display_offset;
                        series.unit = pref
                            .display_unit
                            .clone()
                            .unwrap_or_else(|| series.unit.clone());
                    }
                    if self.source == 0
                        && let Some(info)=shared.display_channel_registry.iter().find(|c|matches!(c.channel_id,crate::state::ChannelId::Math(i) if shared.math_channels.get(i).is_some_and(|m|m.id==id))){series.scale=info.display_scale;series.offset=info.display_offset;series.unit=info.unit.clone();series.precision=info.dec_places.clamp(0,15)as usize;}
                }
            } else if let Some(ch) = ld.channels.iter().find(|c| &c.name == name) {
                let mut s = logged_series(&context, ch, self.display)?;
                if self.display && self.source == 0
                    && let Some(info) = shared
                        .display_channel_registry
                        .iter()
                        .find(|c| matches!(c.channel_id,crate::state::ChannelId::Physical(i) if i==ch.index))
                    {
                        s.unit = info.unit.clone();
                        s.scale = info.display_scale;
                        s.offset = info.display_offset;
                        s.precision = info.dec_places.clamp(0, 15) as usize;
                    }
                channels.push(s);
            } else {
                return Err(format!("{name} is unavailable in selected recording"));
            }
        }
        let distance = if self.distance {
            let (data, rate, unit) =
                crate::comparison::evaluate_channel(&context, "MoTeC::Corr Dist")?;
            let (scale, offset) = i3rs_core::channels::unit_transform(&unit, "m")?;
            Some(OutputSeries {
                name: "Distance".into(),
                unit: "m".into(),
                data,
                rate,
                precision: 6,
                interpolation: Interpolation::Linear,
                scale,
                offset,
            })
        } else {
            None
        };
        Ok((
            "csv".into(),
            series_csv(plan, &channels, distance.as_ref())?,
        ))
    }
}
#[cfg(any(test, not(target_arch = "wasm32")))]
fn make_device_reference(
    shared: &SharedState,
    ld: Arc<LdFile>,
    analysis: &i3rs_core::ranges::RecordingAnalysis,
    bounds: (f64, f64),
) -> Result<i3rs_core::reference_lap::ReferenceLap, String> {
    analysis.validate()?;
    if analysis.recording != ld.recording_id() || shared.recording_key() != ld.recording_id() {
        return Err("Reference analysis belongs to another recording".into());
    }
    let (data, rate, unit) = crate::comparison::evaluate_channel(shared, "MoTeC::Corr Dist")?;
    let (scale, offset) = i3rs_core::channels::unit_transform(&unit, "m")?;
    let distance = OutputSeries {
        name: "Corrected distance".into(),
        unit: "m".into(),
        data,
        rate,
        precision: 6,
        interpolation: Interpolation::Linear,
        scale,
        offset,
    };
    let mut session = ld.session.clone();
    session.driver = analysis.details.driver.clone();
    session.vehicle_id = analysis.details.vehicle.clone();
    session.venue = analysis.details.venue.clone();
    i3rs_core::reference_lap::ReferenceLap::from_distance(&session, &distance, bounds.0, bounds.1)
}

fn logged_series(
    context: &SharedState,
    ch: &i3rs_core::ChannelMeta,
    display: bool,
) -> Result<OutputSeries, String> {
    let mapping = context
        .channel_settings
        .mapping(&ch.name, context.recording_key());
    let preference = context
        .channel_preferences
        .get(&crate::state::channel_preference_key(&ch.name));
    Ok(OutputSeries {
        name: ch.name.clone(),
        unit: if display {
            mapping
                .and_then(|m| m.display_unit.clone().or_else(|| m.unit.clone()))
                .or_else(|| preference.and_then(|p| p.display_unit.clone()))
                .unwrap_or(ch.unit.clone())
        } else {
            ch.unit.clone()
        },
        data: context
            .ld_file
            .as_ref()
            .unwrap()
            .read_channel_data(ch)
            .ok_or("Cannot decode channel")?,
        rate: ch.freq,
        precision: if display {
            mapping
                .and_then(|m| m.precision)
                .unwrap_or(ch.display_decimals as i16)
        } else {
            ch.display_decimals as i16
        }
        .clamp(0, 15) as usize,
        interpolation: if !ch.enum_labels.is_empty() || i3rs_core::is_state_channel(&ch.name) {
            Interpolation::Previous
        } else {
            mapping.map_or(Interpolation::Linear, |m| m.interpolation)
        },
        scale: if display {
            mapping.map_or_else(
                || preference.map_or(1., |p| p.display_scale),
                |m| m.display_scale,
            )
        } else {
            1.
        },
        offset: if display {
            mapping.map_or_else(
                || preference.map_or(0., |p| p.display_offset),
                |m| m.display_offset,
            )
        } else {
            0.
        },
    })
}
type OutputSource = (
    Arc<LdFile>,
    i3rs_core::ranges::RecordingAnalysis,
    Option<(f64, f64)>,
);
fn source(shared: &SharedState, source: usize) -> Option<OutputSource> {
    if source == 0 {
        return Some((
            shared.ld_file.clone()?,
            shared
                .snapshot_recording_analysis()
                .get(shared.recording_key())?
                .clone(),
            shared
                .selected_lap
                .and_then(|i| shared.laps.get(i))
                .map(|l| (l.start_time, l.end_time)),
        ));
    }
    let o = shared.comparisons.overlays.get(source - 1)?;
    let (ld, lap, laps) = match o.source {
        crate::panels::graph::OverlaySource::MainSession => (
            shared.ld_file.clone()?,
            shared.laps.get(o.lap_idx)?,
            &shared.laps,
        ),
        crate::panels::graph::OverlaySource::External(i) => {
            let s = shared.comparisons.sessions.get(i)?;
            (s.ld_file.clone(), s.laps.get(o.lap_idx)?, &s.laps)
        }
    };
    let mut a = shared
        .recording_analyses
        .get(ld.recording_id())
        .cloned()
        .unwrap_or_else(|| {
            let mut a = i3rs_core::ranges::RecordingAnalysis::from_laps(
                ld.recording_id().into(),
                ld.duration_secs(),
                laps,
            );
            a.details.driver = ld.session.driver.clone();
            a.details.venue = ld.session.venue.clone();
            a.details.vehicle = ld.session.vehicle_id.clone();
            a.details.event = ld.event.event_name.clone();
            a
        });
    a.selected_range = Some(a.ranges.get(o.lap_idx)?.id.clone());
    Some((ld, a, Some((lap.start_time, lap.end_time))))
}

type PrintSnapshot = Arc<(String, Vec<Vec<String>>)>;
/// One immutable CSV powers copy, export and printable preview for all result components.
pub fn result_buttons(ui: &mut egui::Ui, csv: &str) {
    if ui.button("Copy CSV").clicked() {
        ui.ctx().copy_text(csv.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Export CSV…").clicked()
        && let Some(path) = crate::platform::save_csv_file()
        && let Err(e) = std::fs::write(path, csv)
    {
        ui.label(format!("Export failed: {e}"));
    }
    let id = ui.id().with("result_preview");
    let mut preview = ui
        .ctx()
        .data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let snapshot_id = id.with("snapshot");
    if ui.button("Print / Preview…").clicked() {
        preview = true;
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                snapshot_id,
                Arc::new((csv.to_owned(), parse_csv_limit(csv, 500))),
            )
        });
    }
    if preview {
        let snapshot = ui
            .ctx()
            .data_mut(|d| d.get_temp::<PrintSnapshot>(snapshot_id))
            .unwrap_or_else(|| Arc::new((csv.to_owned(), parse_csv_limit(csv, 500))));
        #[cfg(not(target_arch = "wasm32"))]
        let csv = snapshot.0.as_str();
        egui::Window::new("Result Print Preview")
            .id(id)
            .open(&mut preview)
            .default_size([760., 500.])
            .show(ui.ctx(), |ui| {
                ui.label(
                    "The preview, saved HTML and print page contain the same result snapshot.",
                );
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Save printable HTML…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("HTML", &["html"])
                        .set_file_name("results.html")
                        .save_file()
                {
                    match std::fs::write(&path, printable_csv(csv)) {
                        Ok(()) => {
                            open_print_page(&path);
                        }
                        Err(e) => {
                            ui.label(e.to_string());
                        }
                    }
                }
                egui::ScrollArea::both().show(ui, |ui| {
                    egui::Grid::new(id.with("table"))
                        .striped(true)
                        .min_col_width(150.)
                        .max_col_width(320.)
                        .show(ui, |ui| {
                            for row in &snapshot.1 {
                                for cell in row {
                                    ui.add(egui::Label::new(cell).wrap());
                                }
                                ui.end_row();
                            }
                        });
                    ui.weak("Preview shows up to 500 rows. Saved HTML contains all rows.");
                });
            });
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, preview));
}
#[cfg(any(not(target_arch = "wasm32"), test))]
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn parse_csv(csv: &str) -> Vec<Vec<String>> {
    parse_csv_limit(csv, usize::MAX)
}
fn parse_csv_limit(csv: &str, limit: usize) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = csv.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            }
            ',' if !quoted => row.push(std::mem::take(&mut cell)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= limit {
                    return rows;
                }
            }
            '\r' if !quoted => {}
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn printable_csv(csv: &str) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>i3rs results</title><style>body{font:12px system-ui;margin:24px}table{border-collapse:collapse;width:100%}td,th{white-space:pre-wrap;border:1px solid #aaa;padding:5px;text-align:left}thead{display:table-header-group}tr{break-inside:avoid}@media print{button{display:none}body{margin:0}}</style></head><body><button onclick=\"window.print()\">Print / Save PDF</button><h1>i3rs results</h1><table>",
    );
    for (i, row) in parse_csv(csv).iter().enumerate() {
        if i == 0 {
            html.push_str("<thead>");
        } else if i == 1 {
            html.push_str("<tbody>");
        }
        html.push_str("<tr>");
        for c in row {
            let tag = if i == 0 { "th" } else { "td" };
            html.push_str(&format!("<{tag}>{}</{tag}>", escape(c)));
        }
        html.push_str("</tr>");
        if i == 0 {
            html.push_str("</thead>");
        }
    }
    html.push_str("</tbody></table></body></html>");
    html
}
#[cfg(not(target_arch = "wasm32"))]
pub fn open_print_page(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn print_and_clipboard_preserve_quoted_multiline_cells_and_escape_html() {
        let csv = "Name,Value\n\"A,\"\"B\"\"\n<script>\",12\n";
        let rows = parse_csv(csv);
        assert_eq!(rows[1][0], "A,\"B\"\n<script>");
        let html = printable_csv(csv);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<td>12</td>"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn printable_image(image: &egui::ColorImage) -> Result<String, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.size[0] as u32, image.size[1] as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let rgba: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|p| p.to_srgba_unmultiplied())
            .collect();
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>i3rs worksheet</title><style>img{{width:100%;height:auto}}@page{{size:landscape}}@media print{{button{{display:none}}}}</style></head><body><button onclick=\"window.print()\">Print / Save PDF</button><img alt=\"i3rs worksheet snapshot\" src=\"data:image/png;base64,{}\"></body></html>",
        base64_png(&bytes)
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn base64_png(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for shift in [18, 12, 6, 0] {
            out.push(
                if shift == 6 && chunk.len() < 2 || shift == 0 && chunk.len() < 3 {
                    '='
                } else {
                    TABLE[((n >> shift) & 63) as usize] as char
                },
            );
        }
    }
    out
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    fn state() -> SharedState {
        let ld = Arc::new(
            LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        );
        let mut shared = SharedState::new();
        shared.data_duration = Some(ld.duration_secs());
        shared.ld_file = Some(ld.clone());
        shared.activate_recording_analysis(&i3rs_core::detect_laps(&ld));
        shared
    }
    #[test]
    fn device_reference_uses_scoped_corrected_distance_and_edited_metadata() {
        let shared = state();
        let ld = shared.ld_file.clone().unwrap();
        let mut analysis = shared.snapshot_recording_analysis()[ld.recording_id()].clone();
        analysis.details.driver = "Reference driver".into();
        let lap = make_device_reference(&shared, ld.clone(), &analysis, (0.125, 2.375)).unwrap();
        assert_eq!(lap.driver, "Reference driver");
        assert_eq!(lap.time_cs.last(), Some(&225));
        assert!(lap.distance_dm.last().unwrap() > &0);
        assert_eq!(
            i3rs_core::reference_lap::ReferenceLap::decode(&lap.encode().unwrap()).unwrap(),
            lap
        );
        analysis.recording = "another recording".into();
        assert!(make_device_reference(&shared, ld, &analysis, (0.125, 2.375)).is_err());
    }
    #[test]
    fn selected_main_and_overlay_keep_their_own_range_and_recording() {
        use crate::panels::graph::{LapOverlay, OverlaySource};
        let mut shared = state();
        shared.select_lap(Some(0));
        let main = source(&shared, 0).unwrap();
        let lap = shared.laps.last().unwrap().clone();
        shared.comparisons.overlays.push(LapOverlay {
            alignment_valid: true,
            gps_align: false,
            visible: true,
            color: None,
            source: OverlaySource::MainSession,
            lap_idx: shared.laps.len() - 1,
            manual_offset: 1.,
            stretch_to_reference: false,
        });
        let overlay = source(&shared, 1).unwrap();
        assert_eq!(overlay.2, Some((lap.start_time, lap.end_time)));
        assert_eq!(main.0.recording_id(), overlay.0.recording_id());
        shared.zoom_range = Some((0.25, 0.75));
        assert_eq!(source(&shared, 0).unwrap().2, main.2);
    }
    #[test]
    fn overlay_reference_log_reopens_on_its_exported_range() {
        use crate::panels::graph::{LapOverlay, OverlaySession, OverlaySource};
        for external in [false, true] {
            for selected in [None, Some("lap-0".to_owned())] {
                let mut shared = state();
                let ld = if external {
                    let mut bytes = std::fs::read(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../test_data/VIR_LAP.ld"
                    ))
                    .unwrap();
                    bytes.push(0);
                    Arc::new(LdFile::from_bytes(bytes).unwrap())
                } else {
                    shared.ld_file.clone().unwrap()
                };
                let laps = (0..2)
                    .map(|i| i3rs_core::Lap {
                        number: i,
                        name: format!("Range {i}"),
                        start_time: i as f64,
                        end_time: i as f64 + 1.,
                    })
                    .collect::<Vec<_>>();
                let mut analysis = i3rs_core::ranges::RecordingAnalysis::from_laps(
                    ld.recording_id().into(),
                    ld.duration_secs(),
                    &laps,
                );
                analysis.selected_range = selected;
                if external {
                    shared
                        .recording_analyses
                        .insert(analysis.recording.clone(), analysis.clone());
                } else {
                    shared.apply_recording_analysis(analysis.clone()).unwrap();
                }
                if external {
                    shared.comparisons.sessions.push(OverlaySession {
                        settings_key: String::new(),
                        requests: Default::default(),
                        failures: Default::default(),
                        path: "overlay.ld".into(),
                        file_name: "overlay.ld".into(),
                        ld_file: ld.clone(),
                        laps: laps.clone(),
                        distance_axis_cache: None,
                        channel_cache: Default::default(),
                    });
                }
                shared.comparisons.overlays.push(LapOverlay {
                    alignment_valid: true,
                    gps_align: false,
                    visible: true,
                    color: None,
                    source: if external {
                        OverlaySource::External(0)
                    } else {
                        OverlaySource::MainSession
                    },
                    lap_idx: 1,
                    manual_offset: 3.,
                    stretch_to_reference: true,
                });
                let (ld, output_analysis, bounds) = source(&shared, 1).unwrap();
                assert_eq!(
                    output_analysis.selected_range,
                    Some(analysis.ranges[1].id.clone())
                );
                let (start, end) = bounds.unwrap();
                let (bytes, saved) = i3rs_core::log_writer::encode_log(
                    &ld,
                    &output_analysis,
                    Some(OutputPlan {
                        start,
                        end,
                        rate: 20.,
                        relative_time: true,
                        include_time: true,
                    }),
                )
                .unwrap();
                let reopened = Arc::new(LdFile::from_bytes(bytes).unwrap());
                let mut restored = SharedState::new();
                restored.data_duration = Some(reopened.duration_secs());
                restored.ld_file = Some(reopened);
                restored
                    .recording_analyses
                    .insert(saved.recording.clone(), saved.clone());
                restored.activate_recording_analysis(&[]);
                assert_eq!(saved.ranges.len(), 1);
                assert_eq!(restored.selected_lap, Some(0));
                assert_eq!(
                    restored.range_selection().range_id,
                    Some(analysis.ranges[1].id.clone())
                );
            }
        }
    }

    #[test]
    fn earth_export_uses_configured_gps_pair_or_canonical_short_names() {
        for configured in [true, false] {
            let mut ld = LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap();
            let (lat, lon) = i3rs_core::track::gps_channels(&ld).unwrap();
            let (lat_index, lon_index) = (lat.index, lon.index);
            ld.channels[lat_index].name = if configured { "Lat" } else { "GPS Lat" }.into();
            ld.channels[lon_index].name = if configured { "Long" } else { "GPS Long" }.into();
            let mut decoy = ld.channels[lat_index].clone();
            decoy.name = "Unrelated latitude".into();
            decoy.shift = 1000;
            ld.channels.insert(0, decoy);
            for (index, channel) in ld.channels.iter_mut().enumerate() {
                channel.index = index;
            }
            let ld = Arc::new(ld);
            let mut shared = SharedState::new();
            shared.ld_file = Some(ld.clone());
            shared.data_duration = Some(ld.duration_secs());
            shared.activate_recording_analysis(&[]);
            if configured {
                let analysis = shared
                    .recording_analyses
                    .get_mut(ld.recording_id())
                    .unwrap();
                analysis.corrections.latitude_channel = "Lat".into();
                analysis.corrections.longitude_channel = "Long".into();
            }
            let output = OutputUi {
                format: 3,
                ..Default::default()
            };
            let plan = OutputPlan {
                start: 0.125,
                end: 0.375,
                rate: 20.,
                relative_time: true,
                include_time: true,
            };
            let (_, kml) = output.make_preview(&shared, ld.clone(), plan).unwrap();
            let lat = logged_series(&shared, &ld.channels[lat_index + 1], false).unwrap();
            let lon = logged_series(&shared, &ld.channels[lon_index + 1], false).unwrap();
            assert_eq!(kml, earth_kml(&ld.session.venue, plan, &lat, &lon).unwrap());
            let analysis = shared
                .recording_analyses
                .get_mut(ld.recording_id())
                .unwrap();
            analysis.corrections.latitude_channel = "Missing".into();
            assert!(output.make_preview(&shared, ld, plan).is_err());
        }
    }

    #[test]
    fn logged_and_derived_exports_are_explicit_and_use_selected_unit_contract() {
        let mut shared = state();
        let ld = shared.ld_file.clone().unwrap();
        let ch = &ld.channels[0];
        shared.channel_preferences.insert(
            crate::state::channel_preference_key(&ch.name),
            crate::state::ChannelPreference {
                display_scale: 2.,
                display_offset: 3.,
                display_unit: Some("test unit".into()),
                ..Default::default()
            },
        );
        let mut mc =
            shared.create_math_channel_def("Export result".into(), "42".into(), "V".into(), 2);
        mc.options.source = "User".into();
        let id = mc.id;
        shared.math_channels.push(mc);
        let mut output = OutputUi {
            display: true,
            channels: [ch.name.clone(), format!("Math::{id}")].into(),
            ..Default::default()
        };
        let plan = OutputPlan {
            start: 0.125,
            end: 0.375,
            rate: 8.,
            relative_time: true,
            include_time: true,
        };
        let raw = output.make_preview(&shared, ld.clone(), plan).unwrap().1;
        assert!(raw.contains("test unit"));
        assert!(!raw.contains("Export result"));
        output.derived = true;
        let derived = output.make_preview(&shared, ld.clone(), plan).unwrap().1;
        assert!(derived.contains("Export result (V)"));
        assert!(derived.contains("42.00"));
        output.display = false;
        let units = output.make_preview(&shared, ld, plan).unwrap().1;
        assert!(!units.contains("test unit"));
    }
    #[test]
    fn export_dialog_renders_each_format_without_decoding_until_requested() {
        let mut shared = state();
        let ctx = Context::default();
        let mut output = OutputUi {
            initialized: true,
            ..Default::default()
        };
        output.begin_export(&shared);
        for format in 0..5 {
            output.format = format;
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 900.),
                    )),
                    ..Default::default()
                },
                |ui| output.export_window(ui.ctx(), &mut shared),
            );
            frame.textures_delta.clear();
            assert!(output.preview.is_none());
            assert!(output.output_job.is_none());
        }
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn image_print_contains_exact_png_and_safe_print_control() {
        assert_eq!(base64_png(b"f"), "Zg==");
        assert_eq!(base64_png(b"fo"), "Zm8=");
        assert_eq!(base64_png(b"foo"), "Zm9v");
        let img = egui::ColorImage::filled([2, 2], egui::Color32::RED);
        let html = printable_image(&img).unwrap();
        assert!(html.contains("data:image/png;base64,iVBOR"));
        assert!(html.contains("window.print()"));
    }
}

#[cfg(test)]
mod reopen_selection_tests {
    use super::*;
    #[test]
    fn incoming_range_identity_takes_precedence_over_old_workbook_main_index() {
        let original = Arc::new(
            LdFile::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap(),
        );
        for (old_main, selected) in [(Some(0), 1), (Some(9), 0)] {
            let mut shared = SharedState::new();
            shared.ld_file = Some(original.clone());
            shared.data_duration = Some(original.duration_secs());
            shared.activate_recording_analysis(&i3rs_core::detect_laps(&original));
            shared.comparisons.main_lap = old_main;
            let workspace = crate::workspace::save_workspace(&[], 0, &shared);
            let mut bytes = std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_data/VIR_LAP.ld"
            ))
            .unwrap();
            bytes.push(0);
            let new = Arc::new(LdFile::from_bytes(bytes).unwrap());
            shared.ld_file = Some(new.clone());
            shared.data_duration = Some(new.duration_secs());
            let mut analysis = i3rs_core::ranges::RecordingAnalysis::from_laps(
                new.recording_id().into(),
                new.duration_secs(),
                &[],
            );
            for i in 0..2 {
                analysis.ranges.push(i3rs_core::ranges::AnalysisRange {
                    id: format!("new-{i}"),
                    name: format!("Stage {i}"),
                    start: i as f64,
                    end: i as f64 + 1.,
                    kind: i3rs_core::ranges::RangeKind::Stage,
                    trust: i3rs_core::ranges::Trust::Trusted,
                    comment: String::new(),
                    reaction_time: None,
                });
            }
            analysis.selected_range = Some(format!("new-{selected}"));
            shared
                .recording_analyses
                .insert(analysis.recording.clone(), analysis);
            shared.activate_recording_analysis(&[]);
            crate::workspace::load_workspace(&workspace, &mut shared);
            assert_eq!(shared.comparisons.main_lap, Some(selected));
            assert_eq!(shared.selected_lap, Some(selected));
            assert_eq!(
                shared.range_selection().range_id,
                Some(format!("new-{selected}"))
            );
        }
    }
}

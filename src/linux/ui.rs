use super::*;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Stroke, Vec2};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use stutter_watch::design;
use stutter_watch::i18n::{LANGUAGE_NAMES, LANGUAGES, Text};

const fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn run(path: &Path) -> Result<()> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime(path)?.with_extension("gui.lock"))?;
    if lock.try_lock().is_err() {
        return open_panel(path);
    }
    let endpoint = socket(path, true)?;
    if endpoint.exists() {
        std::fs::remove_file(&endpoint)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(&endpoint)?;
    listener.set_nonblocking(true)?;
    let config = Config::load(path)?;
    let mut rgba = vec![0; 32 * 32 * 4];
    for (left, height) in [(5, 12), (14, 20), (23, 27)] {
        for x in left..left + 5 {
            for y in 30 - height..30 {
                rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4].copy_from_slice(&[106, 158, 216, 255]);
            }
        }
    }
    let file = path.to_path_buf();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([900.0, 680.0])
            .with_app_id("io.github.flochiii.StutterWatch")
            .with_icon(egui::IconData {
                rgba,
                width: 32,
                height: 32,
            }),
        ..Default::default()
    };
    eframe::run_native(
        stutter_watch::branding::NAME,
        options,
        Box::new(move |cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = color(design::BACKGROUND);
            visuals.window_fill = visuals.panel_fill;
            visuals.override_text_color = Some(color(design::FOREGROUND));
            visuals.selection.bg_fill = Color32::from_rgb(35, 53, 77);
            visuals.widgets.inactive.weak_bg_fill = color(design::BUTTON);
            visuals.widgets.inactive.bg_fill = color(design::FIELD);
            visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, color(design::BORDER));
            visuals.widgets.hovered.weak_bg_fill = color(design::HOVER);
            visuals.widgets.active.weak_bg_fill = color(design::ACTIVE);
            for widget in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
            ] {
                widget.corner_radius = egui::CornerRadius::same(1);
            }
            cc.egui_ctx.set_visuals(visuals);
            cc.egui_ctx.style_mut_of(egui::Theme::Dark, |style| {
                style.spacing.item_spacing = Vec2::splat(design::GAP as f32);
                style.spacing.button_padding = Vec2::new(12.0, 6.0);
                style.text_styles.insert(
                    egui::TextStyle::Body,
                    FontId::proportional(design::BODY_SIZE as f32),
                );
                style.text_styles.insert(
                    egui::TextStyle::Button,
                    FontId::proportional(design::BODY_SIZE as f32),
                );
            });
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "Ubuntu-Light".into(),
                Arc::new(egui::FontData::from_static(design::LATIN_FONT)),
            );
            for (name, bytes) in [
                ("cjk-sc", design::CJK_SC_FONT),
                ("cjk-jp", design::CJK_JP_FONT),
            ] {
                fonts
                    .font_data
                    .insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push(name.into());
            }
            cc.egui_ctx.set_fonts(fonts);
            let data = Arc::new(Mutex::new(None));
            let error = Arc::new(Mutex::new(String::new()));
            let alive = Arc::new(AtomicBool::new(true));
            let state = data.clone();
            let errors = error.clone();
            let running = alive.clone();
            let config_path = file.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let mut since = 0;
                let mut points = std::collections::VecDeque::new();
                while running.load(Ordering::Relaxed) {
                    match rpc(&config_path, &format!("snapshot {since}")) {
                        Ok(value) => match serde_json::from_value::<Snapshot>(value) {
                            Ok(mut snapshot) => {
                                for point in snapshot.points.drain(..) {
                                    since = since.max(point.ts);
                                    points.push_back(point);
                                }
                                while points.len() > 6002
                                    || points
                                        .front()
                                        .is_some_and(|p| p.ts < now_ms().saturating_sub(600_000))
                                {
                                    points.pop_front();
                                }
                                snapshot.points = points.iter().cloned().collect();
                                *state.lock().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
                                errors.lock().unwrap_or_else(|p| p.into_inner()).clear();
                            }
                            Err(e) => {
                                *errors.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string()
                            }
                        },
                        Err(e) => *errors.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string(),
                    };
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(500));
                }
            });
            let running = alive.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    if let Ok((_, _)) = listener.accept() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            Ok(Box::new(App {
                path: file,
                config,
                page: 0,
                data,
                error,
                alive,
                endpoint,
                selected: None,
                incident: Vec::new(),
                entries: Vec::new(),
                last_scan: Instant::now() - Duration::from_secs(10),
                shown: [true; 9],
                pending: None,
                selected_header: None,
                importing: None,
                mango_view: false,
                selected_modified: None,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

struct Capture {
    points: Vec<Point>,
    mango: bool,
    header: Option<Incident>,
}
type LoadedIncident = std::sync::mpsc::Receiver<Result<Capture>>;
struct App {
    path: PathBuf,
    config: Config,
    page: usize,
    data: Arc<Mutex<Option<Snapshot>>>,
    error: Arc<Mutex<String>>,
    alive: Arc<AtomicBool>,
    endpoint: PathBuf,
    selected: Option<PathBuf>,
    incident: Vec<Point>,
    entries: Vec<(PathBuf, String)>,
    last_scan: Instant,
    shown: [bool; 9],
    pending: Option<LoadedIncident>,
    selected_header: Option<Incident>,
    importing: Option<std::sync::mpsc::Receiver<Result<Option<PathBuf>>>>,
    mango_view: bool,
    selected_modified: Option<std::time::SystemTime>,
}
impl Drop for App {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.endpoint);
    }
}
impl App {
    fn action(&self, command: &str) {
        let path = self.path.clone();
        let command = command.to_owned();
        let errors = self.error.clone();
        std::thread::spawn(move || {
            if let Err(e) = rpc(&path, &command) {
                *errors.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string();
            }
        });
    }
    fn scan(&mut self) {
        if self.last_scan.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_scan = Instant::now();
        let mut files =
            stutter_watch::storage::managed_files(&self.config.logs_dir).unwrap_or_default();
        files.retain(|(p, _, _)| {
            p.parent()
                .is_some_and(|p| p.ends_with("incidents") || p.ends_with("protected"))
        });
        files.sort_by_key(|(_, m, _)| std::cmp::Reverse(*m));
        files.truncate(200);
        let next: Vec<_> = files
            .into_iter()
            .map(|(p, _, _)| {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                (p, name)
            })
            .collect();
        if self.entries != next {
            self.entries = next;
        }
        if self.selected.is_none()
            && let Some((path, _)) = self.entries.first()
        {
            self.select(path.clone());
        }
        if self.pending.is_none()
            && let Some(path) = self.selected.clone()
        {
            let modified = path.metadata().ok().and_then(|m| m.modified().ok());
            if modified != self.selected_modified {
                self.select(path);
            }
        }
    }
    fn select(&mut self, path: PathBuf) {
        self.selected = Some(path.clone());
        self.selected_modified = path.metadata().ok().and_then(|m| m.modified().ok());
        let c = self.config.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let mut points = Vec::new();
                let mut mango = false;
                let mut header = None;
                stutter_watch::report::read_records(&path, |r| {
                    match r {
                        Record::Sample(s) => points.push(Point::from_sample(&s, &c)),
                        Record::ExternalLog { .. } => mango = true,
                        Record::Incident(value) => header = Some(value),
                        Record::ExternalFrame {
                            elapsed_ns,
                            fps,
                            frametime_ms,
                        } => {
                            let mut values = [None; 9];
                            values[0] = Some(fps);
                            values[2] = Some(frametime_ms);
                            points.push(Point {
                                ts: elapsed_ns / 1_000_000,
                                values,
                                alerts: if frametime_ms > c.frame_ms {
                                    stutter_watch::assessment::PRESENT
                                } else {
                                    0
                                },
                            });
                        }
                        _ => {}
                    }
                    if points.len() > 6002 {
                        let mut reduced = Vec::new();
                        for chunk in points.chunks(4) {
                            let min = chunk
                                .iter()
                                .min_by(|a, b| {
                                    a.values[2]
                                        .unwrap_or(0.0)
                                        .total_cmp(&b.values[2].unwrap_or(0.0))
                                })
                                .unwrap();
                            let max = chunk
                                .iter()
                                .max_by(|a, b| {
                                    a.values[2]
                                        .unwrap_or(0.0)
                                        .total_cmp(&b.values[2].unwrap_or(0.0))
                                })
                                .unwrap();
                            if min.ts <= max.ts {
                                reduced.push(min.clone());
                                if min.ts != max.ts {
                                    reduced.push(max.clone());
                                }
                            } else {
                                reduced.push(max.clone());
                                reduced.push(min.clone());
                            }
                        }
                        points = reduced;
                    }
                })?;
                Ok(Capture {
                    points,
                    mango,
                    header,
                })
            })();
            let _ = tx.send(result);
        });
    }
    fn apply(&mut self) {
        match self.config.save(&self.path) {
            Ok(()) => self.action("apply"),
            Err(e) => *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string(),
        }
    }
    fn apply_tray(&mut self) {
        match Config::load(&self.path).and_then(|mut c| {
            c.tray_display = self.config.tray_display.clone();
            c.save(&self.path)
        }) {
            Ok(()) => self.action("apply_tray"),
            Err(e) => *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string(),
        }
    }
    fn import_mangohud(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.importing = Some(rx);
        let config = self.config.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Option<PathBuf>> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let selected = runtime.block_on(async {
                    ashpd::desktop::file_chooser::OpenFileRequest::default()
                        .title(Some("MangoHud CSV"))
                        .multiple(Some(false))
                        .send()
                        .await?
                        .response()
                });
                let selected = match selected {
                    Ok(value) => value,
                    Err(e) if e.to_string().to_lowercase().contains("cancel") => return Ok(None),
                    Err(e) => return Err(e.into()),
                };
                let uri = selected.uris().first().context("No file selected")?;
                let path = url::Url::parse(uri.as_str())?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("Select a local CSV file"))?;
                Ok(Some(stutter_watch::mangohud::import(
                    &path,
                    &config.logs_dir,
                    config.history.reserve_free_bytes,
                )?))
            })();
            let _ = tx.send(result);
        });
    }
    fn data_page(&mut self, ui: &mut egui::Ui, t: &Text, snapshot: Option<&Snapshot>) {
        ui.horizontal(|ui| {
            ui.label(t.get("language"));
            egui::ComboBox::from_id_salt("language")
                .selected_text(
                    LANGUAGE_NAMES[LANGUAGES
                        .iter()
                        .position(|l| *l == self.config.language)
                        .unwrap_or(1)],
                )
                .height(170.0)
                .show_ui(ui, |ui| {
                    for (code, name) in LANGUAGES.iter().zip(LANGUAGE_NAMES) {
                        ui.selectable_value(&mut self.config.language, code.to_string(), name);
                    }
                });
        });
        ui.add_space(12.0);
        ui.label(t.get("collect_hint"));
        let mut flags = self.config.collection.flags();
        let keys = [
            "cpu_total",
            "cpu_cores",
            "memory",
            "disks",
            "process_cpu",
            "process_memory",
            "process_io",
            "window_response",
            "frames",
            "gpu",
            "gpu_details",
            "dpc",
            "dpc_events",
            "boot",
        ];
        egui::Grid::new("sources")
            .num_columns(4)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for row in 0..7 {
                    for col in 0..2 {
                        let i = col * 7 + row;
                        let key = keys[i];
                        let supported = !matches!(i, 7 | 8 | 11 | 12 | 13);
                        ui.add_enabled_ui(supported, |ui| {
                            check_row(
                                ui,
                                &mut flags[i],
                                t.get(key),
                                if col == 0 { 170.0 } else { 195.0 },
                                26.0,
                            );
                        });
                        let help = t.get(&format!("help_{key}")).to_owned();
                        let status = snapshot
                            .and_then(|s| s.latest.as_ref())
                            .and_then(|s| s.source_status.get(key));
                        let reason = status
                            .filter(|s| !s.available || s.partial)
                            .map(|s| t.get(&s.reason))
                            .unwrap_or("");
                        ui.add_sized([24.0, 26.0], egui::Label::new("ⓘ"))
                            .on_hover_text(format!("{help}\n{reason}"));
                    }
                    ui.end_row();
                }
            });
        self.config.collection.set_flags(flags);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.linux.pressure, t.get("linux_pressure"));
            ui.label("ⓘ").on_hover_text(t.get("linux_pressure_help"));
        });
        ui.checkbox(
            &mut self.config.linux.global_shortcut,
            t.get("linux_global_shortcut"),
        );
        ui.horizontal(|ui| {
            ui.label(t.get("history_mode"));
            egui::ComboBox::from_id_salt("history-mode")
                .selected_text(t.get(
                    if self.config.history.mode == stutter_watch::config::HistoryMode::Lossless {
                        "lossless"
                    } else {
                        "summary"
                    },
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.history.mode,
                        stutter_watch::config::HistoryMode::Lossless,
                        t.get("lossless"),
                    );
                    ui.selectable_value(
                        &mut self.config.history.mode,
                        stutter_watch::config::HistoryMode::Summary,
                        t.get("summary"),
                    );
                });
        });
        ui.horizontal(|ui| {
            ui.label(t.get("budget"));
            let mut budget = self.config.history.ordinary_budget_bytes / 1_000_000;
            if ui
                .add(egui::DragValue::new(&mut budget).range(1..=100000))
                .changed()
            {
                self.config.history.ordinary_budget_bytes = budget * 1_000_000;
            }
            ui.label(t.get("retention"));
            ui.add(
                egui::DragValue::new(&mut self.config.history.retention_hours).range(1..=876000),
            );
        });
        ui.add_space(10.0);
        if ui
            .add_sized([90.0, 30.0], egui::Button::new(t.get("apply")))
            .clicked()
        {
            self.apply();
        }
        egui::CollapsingHeader::new(t.get("source_status")).show(ui, |ui| {
            if let Some(s) = snapshot {
                for (key, value) in &s.integrations {
                    ui.label(format!("{}: {}", t.get(key), t.get(value)));
                }
                if let Some(sample) = &s.latest {
                    for (key, status) in &sample.source_status {
                        ui.label(format!(
                            "{}: {}",
                            t.get(key),
                            if status.available {
                                t.get("active")
                            } else {
                                t.get(&status.reason)
                            }
                        ));
                    }
                }
            }
        });
    }
    fn tray_page(&mut self, ui: &mut egui::Ui, t: &Text, snapshot: Option<&Snapshot>) {
        ui.label(t.get("tray_choose"));
        ui.add_space(8.0);
        for (key, selected) in [
            ("tray_status", &mut self.config.tray_display.status),
            ("cpu_total", &mut self.config.tray_display.cpu),
            ("memory", &mut self.config.tray_display.memory),
            ("gpu", &mut self.config.tray_display.gpu),
            ("disks", &mut self.config.tray_display.disk_latency),
            ("dpc", &mut self.config.tray_display.dpc),
            (
                "window_response",
                &mut self.config.tray_display.window_response,
            ),
        ] {
            ui.add_enabled_ui(!matches!(key, "dpc" | "window_response"), |ui| {
                check_row(ui, selected, t.get(key), 420.0, 22.0);
            })
            .response
            .on_hover_text(if matches!(key, "dpc" | "window_response") {
                t.get("linux_platform_source")
            } else {
                ""
            });
        }
        let preview = stutter_watch::tray_display::tooltip(
            &self.config,
            t,
            snapshot.and_then(|s| s.latest.as_ref()),
            snapshot.is_some_and(|s| s.paused),
            snapshot.is_some_and(|s| !s.recording.error.is_empty()),
        );
        ui.label(t.get("tray_preview"));
        tray_preview(ui, &preview);
        if ui
            .add_sized([90.0, 30.0], egui::Button::new(t.get("apply")))
            .clicked()
        {
            self.apply_tray();
        }
        ui.add_space(16.0);
        if ui.button(t.get("linux_enable_autostart")).clicked()
            && let Err(e) = autostart(&self.path, true)
        {
            *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string();
        }
        if ui.button(t.get("linux_disable_autostart")).clicked() {
            let _ = autostart(&self.path, false);
        }
        if let Some(s) = snapshot
            && let Some(status) = s.integrations.get("tray")
        {
            ui.label(t.get(status));
        }
        if ui.button(t.get("report")).clicked() {
            let path = self.path.clone();
            let errors = self.error.clone();
            std::thread::spawn(move || {
                if let Err(e) = open_report(&path) {
                    *errors.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string();
                }
            });
        }
    }
}
impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        let snapshot = self.data.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let text = Text::new(&self.config.language);
        if let Some(receiver) = &self.pending
            && let Ok(result) = receiver.try_recv()
        {
            match result {
                Ok(capture) => {
                    self.incident = capture.points;
                    self.mango_view = capture.mango;
                    self.selected_header = capture.header;
                }
                Err(e) => *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string(),
            };
            self.pending = None;
        }
        if let Some(receiver) = &self.importing
            && let Ok(result) = receiver.try_recv()
        {
            match result {
                Ok(Some(path)) => {
                    self.page = 2;
                    self.last_scan = Instant::now() - Duration::from_secs(5);
                    self.select(path);
                }
                Ok(None) => {}
                Err(e) => *self.error.lock().unwrap_or_else(|p| p.into_inner()) = e.to_string(),
            };
            self.importing = None;
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .inner_margin(design::MARGIN as f32)
                    .fill(color(design::BACKGROUND)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    for (i, key) in ["live", "data", "incidents", "tray_settings"]
                        .iter()
                        .enumerate()
                    {
                        let response = ui.add_sized(
                            [design::NAV_WIDTH as f32, design::NAV_HEIGHT as f32],
                            egui::Button::new(text.get(key)).fill(if self.page == i {
                                color(design::ACTIVE)
                            } else {
                                color(design::BUTTON)
                            }),
                        );
                        if self.page == i {
                            ui.painter().line_segment(
                                [response.rect.left_bottom(), response.rect.right_bottom()],
                                Stroke::new(2.0, color(design::BLUE)),
                            );
                        }
                        if response.clicked() {
                            self.page = i;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([70.0, 32.0], egui::Button::new(text.get("quit")))
                            .clicked()
                        {
                            self.action("shutdown");
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui
                            .add_sized(
                                [102.0, 32.0],
                                egui::Button::new(text.get(
                                    if snapshot.as_ref().is_some_and(|s| s.paused) {
                                        "resume"
                                    } else {
                                        "pause"
                                    },
                                )),
                            )
                            .clicked()
                        {
                            self.action("pause");
                        }
                        if ui
                            .add_sized([102.0, 32.0], egui::Button::new(text.get("mark")))
                            .clicked()
                        {
                            self.action("mark");
                        }
                    });
                });
                ui.separator();
                let pressed = ui.input(|input| {
                    input.key_pressed(egui::Key::F12)
                        && input.modifiers.ctrl
                        && input.modifiers.shift
                });
                if pressed {
                    self.action("mark");
                }
                let error = self.error.lock().unwrap_or_else(|p| p.into_inner()).clone();
                if !error.is_empty() {
                    egui::CollapsingHeader::new(
                        egui::RichText::new(text.get("operation_failed")).color(Color32::LIGHT_RED),
                    )
                    .show(ui, |ui| {
                        ui.label(error);
                    });
                }
                if self.page == 1 {
                    self.data_page(ui, &text, snapshot.as_ref());
                } else if self.page == 3 {
                    self.tray_page(ui, &text, snapshot.as_ref());
                } else {
                    if self.page == 2 {
                        if ui
                            .add_enabled(
                                self.importing.is_none(),
                                egui::Button::new(text.get("import_mangohud")),
                            )
                            .clicked()
                        {
                            self.import_mangohud();
                        }
                        self.scan();
                        egui::ScrollArea::vertical()
                            .id_salt("incidents")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                let entries = self.entries.clone();
                                for (path, name) in entries {
                                    if ui
                                        .selectable_label(
                                            self.selected.as_ref() == Some(&path),
                                            name,
                                        )
                                        .clicked()
                                    {
                                        self.select(path);
                                    }
                                }
                            });
                        if self.entries.is_empty() {
                            ui.label(text.get("linux_no_incidents"));
                        }
                        if self.mango_view {
                            ui.label(text.get("relative_time"));
                        } else if !self.incident.is_empty() {
                            let flags = self.incident.iter().fold(0, |mask, p| mask | p.alerts)
                                | self.selected_header.iter().flat_map(|h| &h.triggers).fold(
                                    0,
                                    |mask, t| {
                                        mask | stutter_watch::assessment::reason_flag(&t.reason)
                                    },
                                );
                            ui.label(text.get(stutter_watch::assessment::conclusion_key(flags)));
                            if let Some(header) = &self.selected_header {
                                ui.label(
                                    header
                                        .triggers
                                        .iter()
                                        .map(|t| {
                                            text.get(stutter_watch::assessment::reason_key(
                                                &t.reason,
                                            ))
                                        })
                                        .collect::<std::collections::BTreeSet<_>>()
                                        .into_iter()
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                );
                            }
                        }
                    }
                    let points = if self.page == 2 {
                        self.incident.as_slice()
                    } else {
                        snapshot
                            .as_ref()
                            .map(|s| s.points.as_slice())
                            .unwrap_or(&[])
                    };
                    let (width, height) = design::chart_size(
                        ui.available_width(),
                        ui.available_height(),
                        if self.page == 2 && self.mango_view {
                            1
                        } else {
                            2
                        },
                    );
                    let (width, height) = (width.max(240.0), height.max(100.0));
                    egui::Grid::new("charts")
                        .spacing([design::CHART_GAP, design::CHART_GAP])
                        .show(ui, |ui| {
                            let charts = if self.page == 2 && self.mango_view {
                                vec![
                                    ("frame_time", vec![(2, "frame_time")]),
                                    ("fps", vec![(0, "fps")]),
                                ]
                            } else {
                                vec![
                                    ("graph_cpu", vec![(0, "cpu_total"), (1, "memory")]),
                                    ("graph_disk", vec![(2, "linux_read"), (3, "linux_write")]),
                                    (
                                        "linux_pressure",
                                        vec![
                                            (4, "linux_cpu_wait"),
                                            (5, "linux_memory_stall"),
                                            (6, "linux_io_stall"),
                                        ],
                                    ),
                                    ("linux_gpu_graph", vec![(7, "gpu"), (8, "linux_vram")]),
                                ]
                            };
                            for (i, (title, series)) in charts.into_iter().enumerate() {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(width, height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let header_top = ui.cursor().top();
                                        ui.label(text.get(title));
                                        ui.horizontal(|ui| {
                                            for (j, (index, name)) in series.iter().enumerate() {
                                                ui.visuals_mut().widgets.active.bg_fill = COLORS[j];
                                                ui.checkbox(
                                                    &mut self.shown[*index],
                                                    egui::RichText::new(text.get(name))
                                                        .color(COLORS[j]),
                                                );
                                            }
                                        });
                                        ui.add_space(
                                            (design::CHART_HEADER as f32
                                                - (ui.cursor().top() - header_top))
                                                .max(0.0),
                                        );
                                        chart(
                                            ui,
                                            points,
                                            &series,
                                            &self.shown,
                                            if self.page == 2 && self.mango_view {
                                                i + 4
                                            } else {
                                                i
                                            },
                                            Vec2::new(
                                                width,
                                                (height - design::CHART_HEADER as f32).max(60.0),
                                            ),
                                        );
                                    },
                                );
                                if i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                }
                if let Some(s) = &snapshot {
                    ui.add_space((ui.available_height() - 34.0).max(0.0));
                    ui.separator();
                    ui.label(format!(
                        "{}: {:.1} / {:.0} MB · {}: {:.1} MB · {} MB/{}",
                        text.get("ordinary"),
                        s.recording.usage.ordinary_bytes as f64 / 1e6,
                        s.config.history.ordinary_budget_bytes as f64 / 1e6,
                        text.get("protected"),
                        s.recording.usage.protected_bytes as f64 / 1e6,
                        stutter_watch::capacity::estimate(
                            s.recording.usage.written_bytes,
                            s.observed_seconds
                        )
                        .bytes_per_hour
                        .map(|v| format!("{:.1}", v / 1e6))
                        .unwrap_or_else(|| "—".into()),
                        text.get("unit_hour")
                    ));
                    if !s.recording.error.is_empty() {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            text.recording_issue(&s.recording.error),
                        );
                    }
                }
            });
        root.ctx().request_repaint_after(Duration::from_millis(500));
    }
}
const COLORS: [Color32; 3] = [
    color(design::BLUE),
    color(design::AMBER),
    color(design::GREEN),
];
fn check_row(ui: &mut egui::Ui, checked: &mut bool, label: &str, width: f32, height: f32) {
    let size = Vec2::new(width, height);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.checkbox(checked, label);
        },
    );
}
fn tray_preview(ui: &mut egui::Ui, text: &str) {
    let height = 64.0 + text.lines().count().saturating_sub(1) as f32 * 22.0;
    let content_width = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            ui.painter()
                .layout_no_wrap(
                    line.to_owned(),
                    FontId::proportional(if i == 0 {
                        16.0
                    } else {
                        design::BODY_SIZE as f32
                    }),
                    color(design::FOREGROUND),
                )
                .size()
                .x
                + if i == 0 { 70.0 } else { 36.0 }
        })
        .fold(0.0, f32::max);
    let width = (((content_width + 47.0) / 32.0).floor() * 32.0).clamp(220.0, 460.0);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().min(width), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        4.0,
        color(design::FIELD),
        Stroke::new(1.0, color(design::BORDER)),
        egui::StrokeKind::Inside,
    );
    for (x, y) in [(18.0, 28.0), (25.0, 20.0), (32.0, 24.0)] {
        painter.rect_filled(
            egui::Rect::from_min_max(
                rect.min + Vec2::new(x, y),
                rect.min + Vec2::new(x + 4.0, 36.0),
            ),
            0.0,
            color(design::BLUE),
        );
    }
    let mut lines = text.lines();
    painter.text(
        rect.min + Vec2::new(52.0, 20.0),
        Align2::LEFT_TOP,
        lines.next().unwrap_or(""),
        FontId::proportional(16.0),
        color(design::FOREGROUND),
    );
    for (i, line) in lines.enumerate() {
        painter.text(
            rect.min + Vec2::new(18.0, 50.0 + i as f32 * 22.0),
            Align2::LEFT_TOP,
            line,
            FontId::proportional(design::BODY_SIZE as f32),
            color(if line.contains("ms/") {
                design::MUTED
            } else {
                design::FOREGROUND
            }),
        );
    }
}
fn chart(
    ui: &mut egui::Ui,
    points: &[Point],
    series: &[(usize, &str)],
    shown: &[bool; 9],
    group: usize,
    size: Vec2,
) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, color(design::BACKGROUND));
    let plot = egui::Rect::from_min_max(
        rect.min + Vec2::new(42.0, 0.0),
        rect.max - Vec2::new(8.0, 22.0),
    );
    let end = points.last().map(|p| p.ts).unwrap_or(now_ms());
    let first = points.first().map(|p| p.ts).unwrap_or(end);
    let start = if group >= 4 {
        first
    } else {
        first.max(end.saturating_sub(600_000))
    };
    let span = end.saturating_sub(start).max(1000) as f32;
    let max = if group == 1 || group >= 4 {
        points
            .iter()
            .flat_map(|p| series.iter().filter_map(|(s, _)| p.values[*s]))
            .filter(|v| v.is_finite())
            .fold(1.0, f64::max)
            * 1.1
    } else {
        100.0
    };
    for i in 0..=4 {
        let y = plot.bottom() - plot.height() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, color(design::GRID)),
        );
        painter.text(
            Pos2::new(rect.left(), y),
            Align2::LEFT_CENTER,
            format!("{:.0}", max * i as f64 / 4.0),
            FontId::proportional(design::SMALL_SIZE as f32),
            color(design::MUTED),
        );
    }
    for p in points {
        let red = match group {
            0 => p.alerts & stutter_watch::assessment::RAM_LOW != 0,
            1 => p.alerts & stutter_watch::assessment::DISK != 0,
            2 => {
                p.alerts
                    & (stutter_watch::assessment::MEMORY_STALL
                        | stutter_watch::assessment::IO_STALL)
                    != 0
            }
            4 | 5 => p.alerts & stutter_watch::assessment::PRESENT != 0,
            _ => false,
        };
        let yellow = match group {
            0 => {
                p.alerts
                    & (stutter_watch::assessment::CPU_BUSY | stutter_watch::assessment::RAM_BUSY)
                    != 0
            }
            2 => p.alerts & stutter_watch::assessment::CPU_STALL != 0,
            _ => false,
        };
        if red || yellow {
            let x = plot.left() + (p.ts.saturating_sub(start) as f32 / span) * plot.width();
            painter.rect_filled(
                egui::Rect::from_min_size(
                    Pos2::new(x, plot.top()),
                    Vec2::new((plot.width() * 500.0 / span).max(1.0), plot.height()),
                ),
                0.0,
                if red {
                    color(design::DANGER_BG)
                } else {
                    color(design::WARNING_BG)
                },
            );
        }
    }
    for (j, (index, _)) in series.iter().enumerate() {
        if !shown[*index] {
            continue;
        }
        let mut previous: Option<(u64, Pos2)> = None;
        for p in points {
            if p.ts < start {
                continue;
            }
            let Some(value) = p.values[*index].filter(|v| v.is_finite()) else {
                previous = None;
                continue;
            };
            let position = Pos2::new(
                plot.left() + (p.ts.saturating_sub(start) as f32 / span) * plot.width(),
                plot.bottom() - (value / max).clamp(0.0, 1.0) as f32 * plot.height(),
            );
            if let Some((time, prev)) = previous
                && p.ts.saturating_sub(time)
                    <= if group >= 4 {
                        (span * 8.0 / plot.width()).max(3000.0) as u64
                    } else {
                        3000
                    }
            {
                painter.line_segment([prev, position], Stroke::new(1.3, COLORS[j]));
            } else {
                painter.circle_filled(position, 1.0, COLORS[j]);
            }
            previous = Some((p.ts, position));
        }
    }
    let time = |value: u64| {
        if value < 946684800000 {
            format!("{}:{:02}", value / 60000, value / 1000 % 60)
        } else {
            chrono::DateTime::from_timestamp_millis(value as i64)
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default()
        }
    };
    painter.text(
        Pos2::new(plot.left(), rect.bottom()),
        Align2::LEFT_BOTTOM,
        time(start),
        FontId::proportional(design::SMALL_SIZE as f32),
        color(design::MUTED),
    );
    painter.text(
        Pos2::new(plot.right(), rect.bottom()),
        Align2::RIGHT_BOTTOM,
        time(end),
        FontId::proportional(design::SMALL_SIZE as f32),
        color(design::MUTED),
    );
}

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod assessment;
mod autostart;
mod branding;
mod capacity;
mod collectors;
mod config;
mod detector;
mod history;
mod i18n;
mod identity;
mod model;
mod panel;
mod recorder;
mod report;
mod sources;
mod startup;
mod storage;
mod theme;
mod tray;
mod tray_display;
mod tray_popup;

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use collectors::{cpu_mem, pdh};
use config::Config;
use model::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Console::*, Threading::*},
    },
    core::{BOOL, PCWSTR},
};

static STOP: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicBool = AtomicBool::new(false);

#[derive(Parser)]
#[command(version, about = "Low-overhead Windows stutter flight recorder")]
struct Cli {
    #[arg(long, global = true, default_value = "config.toml")]
    config: PathBuf,
    #[arg(long, value_name = "SECONDS")]
    self_test: Option<u64>,
    #[arg(long, global = true)]
    boot_trace: bool,
    #[arg(long, global = true)]
    no_tray: bool,
    /// Open the native live panel on startup. Closing it leaves the recorder running.
    #[arg(long)]
    panel: bool,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Run,
    /// Open the panel of the existing recorder.
    Show,
    /// Flush and stop the existing recorder for a controlled update.
    Shutdown,
    /// Read a small, recent status summary without exporting log contents.
    Status,
    /// Compress legacy JSONL losslessly while the recorder is stopped.
    CompressHistory,
    /// Repair the per-user notification identity and application shortcut.
    RepairNotifications,
    /// Save a schema-2 configuration before installing an update.
    MigrateConfig {
        #[arg(long)]
        history_defaults: bool,
    },
    Report {
        #[arg(long, default_value_t = 24)]
        hours: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    InstallAutostart {
        #[arg(long)]
        elevated: bool,
        #[arg(long)]
        start: bool,
    },
    RemoveAutostart,
}

fn main() -> Result<()> {
    // GUI launches (including Task Scheduler) never allocate a console. CLI
    // launches can still use their parent's console or redirected standard I/O.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    identity::set_process_id();
    let mut cli = Cli::parse();
    if std::env::args_os().len() == 1 {
        portable_launch(&mut cli, &std::env::current_exe()?);
    }
    // Reopening a portable app activates its existing panel without replacing
    // that recorder's startup diagnostics or starting another set of sources.
    let recording = matches!(cli.command, None | Some(Command::Run));
    if recording && cli.panel && cli.self_test.is_none() && tray::signal(&cli.config, false).is_ok()
    {
        return Ok(());
    }
    if recording {
        startup::begin(&cli.config);
        startup::phase("configuration.loading");
    }
    let result = execute(cli);
    if recording {
        match &result {
            Ok(()) => startup::phase("recorder.stopped"),
            Err(error) => startup::failed(&format!("{error:#}")),
        }
    }
    result
}

fn portable_launch(cli: &mut Cli, executable: &Path) {
    cli.config = executable.with_file_name("config.toml");
    cli.panel = true;
}

fn execute(cli: Cli) -> Result<()> {
    if let Some(seconds) = cli.self_test {
        ensure!(
            (1..=3600).contains(&seconds),
            "--self-test must be 1..3600 seconds"
        );
        ensure!(
            matches!(cli.command, None | Some(Command::Run)),
            "--self-test is only valid for run"
        );
    }
    match cli.command {
        Some(Command::Show) => tray::signal(&cli.config, false),
        Some(Command::Shutdown) => tray::signal(&cli.config, true),
        Some(Command::Status) => {
            let config = Config::load(&cli.config)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report::runtime_status(&config)?)?
            );
            Ok(())
        }
        Some(Command::CompressHistory) => {
            let config = Config::load(&cli.config)?;
            let _instance = Instance::acquire(&cli.config)?;
            let result = storage::compress_history(&config.logs_dir)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Some(Command::RepairNotifications) => identity::install_shortcut(&cli.config),
        Some(Command::MigrateConfig { history_defaults }) => {
            let mut config = Config::load(&cli.config)?;
            if history_defaults {
                config.history.ordinary_budget_bytes = 500_000_000;
                config.history.retention_hours = 24;
            }
            config.save(&cli.config)
        }
        Some(Command::Report { hours, output }) => {
            let config = Config::load(&cli.config)?;
            println!(
                "{}",
                report::generate(&config, hours, output.as_deref())?.display()
            );
            Ok(())
        }
        Some(Command::InstallAutostart { elevated, start }) => {
            let _ = Config::load(&cli.config)?;
            let result = (|| -> Result<()> {
                identity::install_shortcut(&cli.config)?;
                autostart::install(&cli.config, elevated)?;
                if start {
                    autostart::start()?;
                }
                Ok(())
            })();
            let receipt = serde_json::json!({"ts_ms":now_ms(),"success":result.is_ok(),"elevated":elevated,"started":start&&result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))});
            std::fs::write(
                cli.config
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("install-result.json"),
                serde_json::to_vec_pretty(&receipt)?,
            )?;
            result?;
            println!("Autostart installed (elevated: {elevated}).");
            Ok(())
        }
        Some(Command::RemoveAutostart) => {
            autostart::remove()?;
            println!("Autostart removed.");
            Ok(())
        }
        _ => {
            let mut config = Config::load(&cli.config)?;
            if cli.no_tray {
                config.tray = false;
            }
            run(
                config,
                &cli.config,
                cli.self_test,
                cli.boot_trace,
                cli.panel,
            )
        }
    }
}

struct Instance(HANDLE);
impl Instance {
    fn acquire(config: &Path) -> Result<Self> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        config
            .canonicalize()?
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hash);
        let name = pdh::wide(&format!("Local\\stutter-watch-{:016x}", hash.finish()));
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            anyhow::bail!("stutter-watch is already running with this config");
        }
        Ok(Self(handle))
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
unsafe extern "system" fn console_control(kind: u32) -> BOOL {
    STOP.store(true, Ordering::Relaxed);
    if matches!(
        kind,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    ) {
        for _ in 0..250 {
            if FINISHED.load(Ordering::Relaxed) {
                break;
            }
            unsafe {
                Sleep(10);
            }
        }
    }
    BOOL(1)
}

fn flush(store: &recorder::Recorder, batch: &mut Vec<Arc<Sample>>) {
    store.samples(std::mem::take(batch));
}
fn notify_error(ui: &tray::Ui, store: &recorder::Recorder, text: &i18n::Text, error: &str) {
    ui.notify(&text.diagnostic(error));
    store.append(Record::Event {
        ts_ms: now_ms(),
        name: "warning".into(),
        detail: error.into(),
    });
}
fn dump_ready(store: &recorder::Recorder, detector: &mut detector::Detector, shutdown: bool) {
    while let Some(pending) = detector.take_ready(shutdown) {
        store.incident(&pending);
    }
}
fn run(
    mut config: Config,
    config_path: &Path,
    seconds: Option<u64>,
    boot_trace: bool,
    show_panel: bool,
) -> Result<()> {
    let _instance = Instance::acquire(config_path)?;
    startup::phase("instance.acquired");
    let wall_start = Instant::now();
    let (_, initial_cpu) = cpu_mem::own_usage()?;
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        if GetConsoleProcessList(&mut [0u32; 1]) > 0 {
            SetConsoleCtrlHandler(Some(console_control), true)?;
        }
    }
    let dashboard = Arc::new(Mutex::new(panel::Dashboard::new(&config)));
    // Show a usable tray and publish show/shutdown before initializing providers.
    let (mut ui, receiver) = tray::Ui::start(
        Arc::clone(&dashboard),
        config.tray,
        &config.hotkey,
        false,
        false,
    )?;
    ui.publish_endpoint(config_path)?;
    startup::phase("tray.ready");
    if show_panel {
        ui.open_panel();
    }
    startup::phase("autostart.query");
    ui.autostart
        .store(seconds.is_none() && autostart::enabled(), Ordering::Relaxed);
    let mut sources = sources::Sources::new(&config, boot_trace)?;
    startup::phase("writer.initializing");
    let store = recorder::Recorder::new(&config)?;
    let mut detector = detector::Detector::new(&config);
    dashboard.lock().unwrap_or_else(|p| p.into_inner()).warnings = sources.warnings.clone();
    let history_config = config.clone();
    let history_dashboard = Arc::clone(&dashboard);
    let history_worker = std::thread::Builder::new()
        .name("graph-history".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = history::restore(&history_config, now_ms());
            let mut d = history_dashboard.lock().unwrap_or_else(|p| p.into_inner());
            match result {
                Ok(points) => d.merge_history(points),
                Err(e) => {
                    d.history_loading = false;
                    d.warnings.push(format!("History: {e:#}"));
                }
            }
        })?;
    for warning in &sources.warnings {
        eprintln!("Warning: {warning}");
    }
    store.append(Record::Session {
        ts_ms: now_ms(),
        version: env!("CARGO_PKG_VERSION").into(),
        warnings: sources.warnings.clone(),
        pid: std::process::id(),
        elevated: unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool(),
        sample_ms: config.sample_ms,
        flush_seconds: config.flush_seconds,
        detailed_logs: config.history.mode == config::HistoryMode::Lossless,
    });
    ui.set_warning(!sources.warnings.is_empty());
    startup::phase("recorder.running");
    let start = Instant::now();
    let (_, steady_cpu) = cpu_mem::own_usage()?;
    let mut last = start;
    let mut last_flush = start;
    let mut next_tick = start;
    let mut calibration = start;
    let mut batch = Vec::new();
    let mut total_samples = 0usize;
    let mut peak_memory = 0u64;
    let mut last_recording_error = String::new();
    let mut last_recording_notice: Option<Instant> = None;
    let mut report_worker: Option<std::thread::JoinHandle<Result<PathBuf>>> = None;
    let result = (|| -> Result<()> {
        loop {
            if STOP.load(Ordering::Relaxed)
                || seconds.is_some_and(|s| start.elapsed() >= Duration::from_secs(s))
            {
                break;
            }
            if report_worker.as_ref().is_some_and(|w| w.is_finished()) {
                match report_worker
                    .take()
                    .expect("report worker")
                    .join()
                    .map_err(|_| anyhow::anyhow!("Report worker failed"))?
                {
                    Ok(path) => ui.notify(&format!(
                        "{}: {}",
                        i18n::Text::new(&config.language).get("report"),
                        path.display()
                    )),
                    Err(e) => notify_error(
                        &ui,
                        &store,
                        &i18n::Text::new(&config.language),
                        &format!("{e:#}"),
                    ),
                }
            }
            let timeout = if ui.paused.load(Ordering::Relaxed) {
                Duration::from_millis(100)
            } else {
                next_tick
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100))
            };
            match receiver.recv_timeout(timeout) {
                Ok(command) => {
                    let text = i18n::Text::new(&config.language);
                    match command {
                        tray::Command::Quit => break,
                        tray::Command::ApplyTray(display) => {
                            let mut updated = config.clone();
                            updated.tray_display = display;
                            if let Err(e) = updated.save(config_path) {
                                notify_error(&ui, &store, &text, &format!("{e:#}"));
                                continue;
                            }
                            config = updated;
                            dashboard
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .config
                                .tray_display = config.tray_display.clone();
                            ui.notify(text.get("changed"));
                        }
                        tray::Command::Apply(new_config) => {
                            new_config.validate()?;
                            // Finish the old policy explicitly before starting the new sources.
                            dump_ready(&store, &mut detector, true);
                            flush(&store, &mut batch);
                            store.barrier()?;
                            if let Err(e) = new_config.save(config_path) {
                                notify_error(&ui, &store, &text, &format!("{e:#}"));
                                continue;
                            }
                            sources.pause();
                            config = *new_config;
                            sources = sources::Sources::new(&config, false)?;
                            if ui.paused.load(Ordering::Relaxed) {
                                sources.pause();
                            }
                            store.configure(&config);
                            detector = detector::Detector::new(&config);
                            calibration = Instant::now();
                            last_flush = Instant::now();
                            next_tick = Instant::now();
                            last = Instant::now();
                            {
                                let mut d = dashboard.lock().unwrap_or_else(|p| p.into_inner());
                                d.config = config.clone();
                                d.warnings = sources.warnings.clone();
                            }
                            ui.set_warning(
                                !sources.warnings.is_empty() || !store.status().error.is_empty(),
                            );
                            ui.rebuild_panel();
                            ui.notify(i18n::Text::new(&config.language).get("changed"));
                        }
                        tray::Command::Pause => {
                            let paused = !ui.paused.load(Ordering::Relaxed);
                            ui.paused.store(paused, Ordering::Relaxed);
                            if paused {
                                dump_ready(&store, &mut detector, true);
                                flush(&store, &mut batch);
                                detector.clear_history();
                                sources.pause();
                            } else {
                                sources = sources::Sources::new(&config, false)?;
                                next_tick = Instant::now();
                                last = Instant::now();
                            }
                            dashboard.lock().unwrap_or_else(|p| p.into_inner()).paused = paused;
                            store.append(Record::Event {
                                ts_ms: now_ms(),
                                name: if paused { "paused" } else { "resumed" }.into(),
                                detail: String::new(),
                            });
                            ui.notify(text.get(if paused { "pause" } else { "resume" }));
                        }
                        tray::Command::Mark => {
                            let active = detector
                                .ring
                                .latest()
                                .map(|s| s.foreground.clone())
                                .unwrap_or_default();
                            if detector.mark_now("manual_mark", active) {
                                if let Some(pending) = detector.pending.as_ref() {
                                    if let Some(marker) = pending.header.triggers.last() {
                                        store.append(Record::Mark(marker.clone()));
                                    }
                                    store.incident(pending);
                                }
                                ui.notify(text.get("marked"));
                            } else {
                                ui.notify(text.get("unknown"));
                            }
                        }
                        tray::Command::Report => {
                            if report_worker.is_none() {
                                flush(&store, &mut batch);
                                store.barrier()?;
                                let config = config.clone();
                                report_worker = Some(std::thread::spawn(move || {
                                    report::generate(&config, 24, None)
                                }));
                            }
                        }
                        tray::Command::OpenLogs => {
                            if let Err(e) = tray::open(&store.root) {
                                notify_error(&ui, &store, &text, &format!("{e:#}"));
                            }
                        }
                        tray::Command::Autostart => {
                            let enabled = ui.autostart.load(Ordering::Relaxed);
                            match if enabled {
                                autostart::remove()
                            } else {
                                autostart::install(config_path, config.autostart_elevated)
                            } {
                                Ok(()) => {
                                    ui.autostart.store(!enabled, Ordering::Relaxed);
                                    ui.notify(text.get("autostart"));
                                }
                                Err(e) => notify_error(&ui, &store, &text, &format!("{e:#}")),
                            }
                        }
                        tray::Command::Warning(detail) => {
                            eprintln!("Warning: {detail}");
                            ui.notify(&text.diagnostic(&detail));
                            store.append(Record::Event {
                                ts_ms: now_ms(),
                                name: "warning".into(),
                                detail,
                            });
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            if ui.paused.load(Ordering::Relaxed) || Instant::now() < next_tick {
                continue;
            }
            let tick = Instant::now();
            let sample =
                Arc::new(sources.collect(start, tick.duration_since(last).as_millis() as u64));
            last = tick;
            {
                let mut d = dashboard.lock().unwrap_or_else(|p| p.into_inner());
                d.observed_seconds = calibration.elapsed().as_secs_f64();
                d.recording = store.status();
                d.warnings = sources.warnings.clone();
                d.publish(Arc::clone(&sample));
            }
            let recording_error = store.status().error;
            ui.set_warning(!recording_error.is_empty() || !sources.warnings.is_empty());
            if recording_error != last_recording_error {
                if !recording_error.is_empty()
                    && last_recording_notice.is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
                {
                    ui.notify(&i18n::Text::new(&config.language).recording_issue(&recording_error));
                    last_recording_notice = Some(Instant::now());
                }
                last_recording_error = recording_error;
            }
            batch.push(Arc::clone(&sample));
            if detector.push(sample)
                && let Some(pending) = detector.pending.as_ref()
            {
                store.incident(pending);
            }
            dump_ready(&store, &mut detector, false);
            // Bound both the retained batch and the two-slot disk queue, independently of UI.
            if batch.len() >= 64
                || last_flush.elapsed() >= Duration::from_secs(config.flush_seconds)
            {
                flush(&store, &mut batch);
                last_flush = Instant::now();
            }
            if seconds.is_some() {
                peak_memory = peak_memory.max(cpu_mem::own_usage()?.0);
            }
            total_samples += 1;
            next_tick += Duration::from_millis(config.sample_ms);
            if next_tick < Instant::now() {
                next_tick = Instant::now() + Duration::from_millis(config.sample_ms);
            }
        }
        Ok(())
    })();
    if seconds.is_some() {
        detector.mark("self_test");
    } else {
        detector.shutdown_snapshot();
    }
    dump_ready(&store, &mut detector, true);
    flush(&store, &mut batch);
    store.barrier()?;
    if let Some(worker) = report_worker {
        let _ = worker.join();
    }
    let _ = history_worker.join();
    FINISHED.store(true, Ordering::Relaxed);
    result?;
    if seconds.is_some() {
        let (memory, cpu) = cpu_mem::own_usage()?;
        peak_memory = peak_memory
            .max(memory)
            .max(cpu_mem::own_peak_working_set()?);
        let processors = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1) as f64;
        let cpu_pct =
            (cpu - steady_cpu) as f64 / 10_000_000.0 / start.elapsed().as_secs_f64() / processors
                * 100.0;
        let total_cpu_pct = (cpu - initial_cpu) as f64
            / 10_000_000.0
            / wall_start.elapsed().as_secs_f64()
            / processors
            * 100.0;
        let mut rows = 0usize;
        let mut malformed = 0;
        for (path, _, _) in storage::managed_files(&store.root)? {
            malformed += report::read_records(&path, |_| rows += 1)?;
        }
        let state = store.status();
        let output = serde_json::json!({"self_test_seconds":start.elapsed().as_secs_f64(),"samples":total_samples,"records_validated":rows,"malformed_records":malformed,"working_set_bytes":memory,"peak_working_set_bytes":peak_memory,"cpu_pct":cpu_pct,"cpu_including_startup_pct":total_cpu_pct,"cpu_basis":"all logical processors (Task Manager convention)","resource_limits":"not yet established; measurement only","frame_etw":sources.frames.status,"kernel_etw":sources.frames.kernel_status,"ordinary_bytes":state.usage.ordinary_bytes,"protected_bytes":state.usage.protected_bytes,"omitted_jobs":state.omitted_jobs,"recording_error":state.error});
        std::fs::write(
            store.root.join("self-test-last.json"),
            serde_json::to_vec_pretty(&output)?,
        )?;
        println!("{}", serde_json::to_string_pretty(&output)?);
        ensure!(
            total_samples >= 2,
            "Self-test did not collect enough samples"
        );
        ensure!(malformed == 0, "Malformed stored records");
        ensure!(
            state.omitted_jobs == 0 && state.error.is_empty(),
            "Recording incomplete; see self-test-last.json"
        );
    }
    Ok(())
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    #[test]
    fn portable_launch_keeps_data_beside_executable() {
        let mut cli = Cli::parse_from(["stutter-watch"]);
        portable_launch(
            &mut cli,
            Path::new(r"C:\Portable Apps\Stutter Watch\stutter-watch.exe"),
        );
        assert_eq!(
            cli.config,
            Path::new(r"C:\Portable Apps\Stutter Watch\config.toml")
        );
        assert!(cli.panel);
        assert!(cli.command.is_none());
        assert!(cli.self_test.is_none());
    }

    #[test]
    fn explicit_run_remains_background_and_uses_selected_config() {
        let cli = Cli::parse_from(["stutter-watch", "--config", "custom.toml", "run"]);
        assert!(!cli.panel);
        assert_eq!(cli.config, Path::new("custom.toml"));
        assert!(matches!(cli.command, Some(Command::Run)));
    }
}

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use stutter_watch::{config::Config, model::*, recorder::Status};
mod daemon;
mod gpu;
mod integrations;
mod ui;

#[derive(Parser)]
#[command(version, about = "Local stutter recorder and desktop monitor")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long)]
    self_test: Option<u64>,
    #[arg(long, global = true)]
    no_tray: bool,
    #[arg(long)]
    panel: bool,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    ImportMangohud {
        file: PathBuf,
    },
    Run,
    Show,
    Shutdown,
    Status,
    Mark,
    Pause,
    Report {
        #[arg(long, default_value_t = 24)]
        hours: u64,
    },
    InstallAutostart,
    RemoveAutostart,
    CompressHistory,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub ts: u64,
    pub values: [Option<f64>; 9],
    pub alerts: u16,
}
impl Point {
    pub fn from_sample(s: &Sample, c: &Config) -> Self {
        let max = |select: fn(&DiskDevice) -> Option<f64>| {
            s.disk
                .devices
                .iter()
                .filter_map(select)
                .max_by(f64::total_cmp)
        };
        let pressure = s.pressure.as_ref();
        let gpu = s
            .gpu
            .adapters
            .iter()
            .filter_map(|a| a.usage_pct)
            .max_by(f64::total_cmp);
        let vram = s
            .gpu
            .adapters
            .iter()
            .filter_map(|a| a.nvml.as_ref().or(a.sensors.as_ref()))
            .filter_map(|n| {
                Some(n.vram_used_bytes? as f64 / n.vram_total_bytes?.max(1) as f64 * 100.0)
            })
            .max_by(f64::total_cmp);
        Self {
            ts: s.ts_ms,
            alerts: stutter_watch::assessment::signals(c, s),
            values: [
                s.cpu.usage_pct,
                (s.memory.total_bytes > 0)
                    .then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0),
                max(|d| d.read_latency_ms),
                max(|d| d.write_latency_ms),
                pressure
                    .and_then(|p| p.cpu.as_ref())
                    .and_then(|p| p.some_pct),
                pressure
                    .and_then(|p| p.memory.as_ref())
                    .and_then(|p| p.full_pct),
                pressure
                    .and_then(|p| p.io.as_ref())
                    .and_then(|p| p.full_pct),
                gpu,
                vram,
            ],
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub observed_seconds: f64,
    pub config: Config,
    pub latest: Option<Sample>,
    pub points: Vec<Point>,
    pub recording: Status,
    pub paused: bool,
    pub integrations: std::collections::BTreeMap<String, String>,
    pub error: String,
}

pub fn launch_executable() -> Result<PathBuf> {
    // An AppImage child needs its own runtime/mount after its parent GUI closes.
    Ok(std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .unwrap_or(std::env::current_exe()?))
}

pub fn xdg(variable: &str, fallback: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(path.join("stutter-watch"));
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?);
    ensure!(home.is_absolute(), "HOME must be absolute");
    Ok(home.join(fallback).join("stutter-watch"))
}
pub fn runtime(config: &Path) -> Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    config.hash(&mut hash);
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("stutter-watch-{}", unsafe { libc::getuid() }));
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(&root)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::getuid() } && meta.mode() & 0o077 == 0,
        "Runtime directory must be private and owned by this user"
    );
    Ok(root.join(format!("{:016x}", hash.finish())))
}
pub fn socket(config: &Path, gui: bool) -> Result<PathBuf> {
    Ok(runtime(config)?.with_extension(if gui { "gui.sock" } else { "sock" }))
}
pub fn rpc(config: &Path, request: &str) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket(config, false)?)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut bytes = Vec::new();
    stream.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if let Some(error) = value
        .get("error")
        .and_then(|e| e.as_str())
        .filter(|e| !e.is_empty())
    {
        anyhow::bail!("{error}");
    }
    Ok(value)
}
pub fn ensure_recorder(config: &Path, no_tray: bool) -> Result<()> {
    if rpc(config, "ping").is_ok() {
        return Ok(());
    }
    let mut cmd = Command::new(launch_executable()?);
    cmd.arg("--config")
        .arg(config)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if no_tray {
        cmd.arg("--no-tray");
    }
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if rpc(config, "ping").is_ok() {
            return Ok(());
        }
        if let Some(code) = child.try_wait()? {
            ensure!(
                code.success(),
                "Recorder failed to start; check its startup diagnostic"
            );
        }
        ensure!(Instant::now() < deadline, "Recorder did not become ready");
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn open_panel(config: &Path) -> Result<()> {
    if let Ok(mut socket) = UnixStream::connect(socket(config, true)?) {
        let _ = socket.write_all(b"show");
        return Ok(());
    }
    Command::new(launch_executable()?)
        .arg("--config")
        .arg(config)
        .arg("--panel")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
pub fn open_report(config: &Path) -> Result<()> {
    let config = Config::load(config)?;
    let path = stutter_watch::report::generate(&config, 24, None)?;
    Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
pub fn autostart(config: &Path, enable: bool) -> Result<()> {
    let root = xdg("XDG_CONFIG_HOME", ".config")?
        .parent()
        .unwrap()
        .join("autostart");
    std::fs::create_dir_all(&root)?;
    let file = root.join("io.github.flochiii.StutterWatch.desktop");
    if enable {
        let exe = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_exe()?);
        let quote = |p: &Path| {
            format!(
                "\"{}\"",
                p.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('`', "\\`")
                    .replace('$', "\\$")
                    .replace('%', "%%")
            )
        };
        std::fs::write(
            file,
            format!(
                "[Desktop Entry]\nType=Application\nName=Stutter Watch\nExec={} --config {} run\nIcon=io.github.flochiii.StutterWatch\nTerminal=false\nX-StutterWatch-Owned=true\n",
                quote(&exe),
                quote(config)
            ),
        )?;
    } else if file.exists() {
        let content = std::fs::read_to_string(&file)?;
        ensure!(
            content.contains("X-StutterWatch-Owned=true"),
            "Autostart file belongs to another application"
        );
        std::fs::remove_file(file)?;
    }
    Ok(())
}
pub fn main() -> Result<()> {
    unsafe {
        libc::umask(0o077);
    }
    let cli = Cli::parse();
    if let Some(seconds) = cli.self_test {
        ensure!(
            (1..=3600).contains(&seconds),
            "--self-test must be 1..3600 seconds"
        );
    }
    let default_config = cli.config.is_none();
    let path = match cli.config {
        Some(path) => path,
        None => xdg("XDG_CONFIG_HOME", ".config")?.join("config.toml"),
    };
    let fresh = !path.exists();
    let mut config = Config::load(&path)?;
    if fresh && default_config {
        config.logs_dir = xdg("XDG_STATE_HOME", ".local/state")?.join("logs");
        config.save(&path)?;
    }
    let path = path.canonicalize()?;
    match cli.command {
        Some(Action::ImportMangohud { file }) => {
            println!(
                "{}",
                stutter_watch::mangohud::import(
                    &file,
                    &config.logs_dir,
                    config.history.reserve_free_bytes
                )?
                .display()
            );
            Ok(())
        }
        Some(Action::Run) => daemon::run(config, &path, cli.self_test, cli.no_tray),
        None if cli.self_test.is_some() => daemon::run(config, &path, cli.self_test, true),
        None | Some(Action::Show) => {
            ensure_recorder(&path, cli.no_tray)?;
            ui::run(&path)
        }
        Some(Action::Shutdown) => rpc(&path, "shutdown").map(|_| ()),
        Some(Action::Mark) => rpc(&path, "mark").map(|_| ()),
        Some(Action::Pause) => rpc(&path, "pause").map(|_| ()),
        Some(Action::Status) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&stutter_watch::report::runtime_status(&config)?)?
            );
            Ok(())
        }
        Some(Action::Report { hours }) => {
            println!(
                "{}",
                stutter_watch::report::generate(&config, hours, None)?.display()
            );
            Ok(())
        }
        Some(Action::InstallAutostart) => autostart(&path, true),
        Some(Action::RemoveAutostart) => autostart(&path, false),
        Some(Action::CompressHistory) => {
            let lock = std::fs::File::create(runtime(&path)?.with_extension("lock"))?;
            lock.try_lock()?;
            println!(
                "{}",
                serde_json::to_string(&stutter_watch::storage::compress_history(
                    &config.logs_dir
                )?)?
            );
            Ok(())
        }
    }
}

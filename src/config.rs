use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub linux: Linux,
    pub schema_version: u32,
    pub collection: Collection,
    pub history: History,
    pub privacy: Privacy,
    pub tray_display: TrayDisplay,
    pub language: String,
    pub sample_ms: u64,
    pub ring_seconds: u64,
    pub flush_seconds: u64,
    pub detailed_logs: bool,
    pub logs_dir: PathBuf,
    pub rotate_mb: u64,
    pub max_disk_mb: u64,
    pub retention_days: u64,
    pub pre_seconds: u64,
    pub post_seconds: u64,
    pub manual_seconds: u64,
    pub manual_post_seconds: u64,
    pub cooldown_seconds: u64,
    pub frame_ms: f64,
    pub dwm_ms: f64,
    pub hung_ms: u64,
    pub disk_latency_ms: f64,
    pub disk_queue: f64,
    pub dpc_pct: f64,
    pub driver_spike_ms: f64,
    pub hotkey: String,
    pub tray: bool,
    pub autostart_elevated: bool,
    pub nvml: bool,
    pub nvidia_tgp_w: f64,
    pub etw: bool,
    pub kernel_etw: bool,
    pub boot_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            linux: Linux::default(),
            schema_version: 2,
            collection: Collection::default(),
            history: History::default(),
            privacy: Privacy::default(),
            tray_display: TrayDisplay::default(),
            language: "en".into(),
            sample_ms: 500,
            ring_seconds: 35,
            flush_seconds: 60,
            detailed_logs: true,
            logs_dir: "logs".into(),
            rotate_mb: 20,
            max_disk_mb: 500,
            retention_days: 7,
            pre_seconds: 15,
            post_seconds: 5,
            manual_seconds: 30,
            manual_post_seconds: 10,
            cooldown_seconds: 10,
            frame_ms: 100.0,
            dwm_ms: 100.0,
            hung_ms: 150,
            disk_latency_ms: 50.0,
            disk_queue: 4.0,
            dpc_pct: 15.0,
            driver_spike_ms: 2.0,
            hotkey: "Ctrl+Shift+F12".into(),
            tray: true,
            autostart_elevated: false,
            nvml: true,
            nvidia_tgp_w: 40.0,
            etw: true,
            kernel_etw: true,
            boot_seconds: 180,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Linux {
    pub pressure: bool,
    pub global_shortcut: bool,
}
impl Default for Linux {
    fn default() -> Self {
        Self {
            pressure: true,
            global_shortcut: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayDisplay {
    pub status: bool,
    pub cpu: bool,
    pub memory: bool,
    pub gpu: bool,
    pub disk_latency: bool,
    pub dpc: bool,
    pub window_response: bool,
}
impl Default for TrayDisplay {
    fn default() -> Self {
        Self {
            status: true,
            cpu: true,
            memory: true,
            gpu: false,
            disk_latency: false,
            dpc: false,
            window_response: false,
        }
    }
}
impl TrayDisplay {
    pub fn flags(&self) -> [bool; 7] {
        [
            self.status,
            self.cpu,
            self.memory,
            self.gpu,
            self.disk_latency,
            self.dpc,
            self.window_response,
        ]
    }
    pub fn set_flags(&mut self, flags: [bool; 7]) {
        [
            self.status,
            self.cpu,
            self.memory,
            self.gpu,
            self.disk_latency,
            self.dpc,
            self.window_response,
        ] = flags;
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, toml::to_string_pretty(&Self::default())?)?;
        }
        let text = std::fs::read_to_string(path)?;
        let document: toml::Value = toml::from_str(&text)?;
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("Invalid config: {}", path.display()))?;
        if document.get("schema_version").is_none() {
            // Legacy max_disk_mb was MiB. Preserve explicitly supplied limits exactly.
            if document.get("max_disk_mb").is_some() {
                config.history.ordinary_budget_bytes = config.max_disk_mb.saturating_mul(1_048_576);
            }
            if document.get("retention_days").is_some() {
                config.history.retention_hours = config.retention_days.saturating_mul(24);
            }
            if document.get("detailed_logs").is_some() && !config.detailed_logs {
                config.history.mode = HistoryMode::Summary;
            }
            if document.get("etw").is_some() {
                config.collection.frames = config.etw;
            }
            if document.get("kernel_etw").is_some() {
                config.collection.dpc_events = config.kernel_etw;
            }
            if document.get("nvml").is_some() {
                config.collection.gpu_details = config.nvml;
            }
            if document.get("boot_seconds").is_some() {
                config.collection.boot = config.boot_seconds > 0;
            }
        }
        config.validate()?;
        if config.logs_dir.is_relative() {
            config.logs_dir = std::path::absolute(path)?
                .parent()
                .unwrap_or(Path::new("."))
                .join(&config.logs_dir);
        }
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 2,
            "Unsupported config schema_version"
        );
        ensure!(
            (1_000_000..=100_000_000_000).contains(&self.history.ordinary_budget_bytes),
            "Invalid ordinary history budget"
        );
        ensure!(
            (1..=876_000).contains(&self.history.retention_hours),
            "Invalid history retention"
        );
        ensure!(
            (1..=60).contains(&self.history.summary_seconds),
            "Invalid summary interval"
        );
        ensure!(
            self.history.reserve_free_bytes >= 1_000_000,
            "Invalid disk reserve"
        );
        ensure!(
            (1..=30).contains(&self.manual_post_seconds),
            "manual_post_seconds must be 1..30"
        );
        ensure!(
            (1..=20).contains(&self.collection.process_limit),
            "process_limit must be 1..20"
        );
        ensure!(
            ["uk", "en", "pl", "de", "fr", "es", "pt", "ru", "zh", "ja"]
                .contains(&self.language.as_str()),
            "Unsupported language"
        );
        ensure!(
            (100..=5000).contains(&self.sample_ms),
            "sample_ms must be 100..5000"
        );
        ensure!(
            (10..=300).contains(&self.flush_seconds),
            "flush_seconds must be 10..300"
        );
        ensure!(
            self.ring_seconds >= self.manual_seconds.max(self.pre_seconds)
                && self.ring_seconds <= 120,
            "ring_seconds must cover pre_seconds and manual_seconds and be <= 120"
        );
        ensure!(
            self.pre_seconds > 0
                && self.manual_seconds > 0
                && (1..=30).contains(&self.post_seconds),
            "invalid incident window"
        );
        ensure!(
            self.rotate_mb > 0 && self.rotate_mb <= self.max_disk_mb && self.max_disk_mb <= 100_000,
            "invalid disk limits"
        );
        ensure!(
            (1..=36500).contains(&self.retention_days)
                && (1..=3600).contains(&self.cooldown_seconds)
                && self.boot_seconds <= 600,
            "invalid retention, cooldown or boot duration"
        );
        for value in [
            self.frame_ms,
            self.dwm_ms,
            self.disk_latency_ms,
            self.disk_queue,
            self.dpc_pct,
            self.driver_spike_ms,
            self.nvidia_tgp_w,
        ] {
            ensure!(
                value.is_finite() && value > 0.0,
                "thresholds must be finite and positive"
            );
        }
        ensure!(
            (100..=10_000).contains(&self.hung_ms),
            "hung_ms must be 100..10000"
        );
        crate::hotkey::parse_hotkey(&self.hotkey)?;
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, toml::to_string_pretty(self)?)?;
        if path.exists() && !path.with_extension("toml.bak").exists() {
            std::fs::copy(path, path.with_extension("toml.bak"))?;
        }
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    Lossless,
    Summary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    pub mode: HistoryMode,
    pub ordinary_budget_bytes: u64,
    pub retention_hours: u64,
    pub summary_seconds: u64,
    pub reserve_free_bytes: u64,
}
impl Default for History {
    fn default() -> Self {
        Self {
            mode: HistoryMode::Lossless,
            ordinary_budget_bytes: 500_000_000,
            retention_hours: 24,
            summary_seconds: 5,
            reserve_free_bytes: 256_000_000,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Privacy {
    pub window_titles: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Collection {
    pub cpu_total: bool,
    pub cpu_cores: bool,
    pub memory: bool,
    pub disks: bool,
    pub process_cpu: bool,
    pub process_memory: bool,
    pub process_io: bool,
    pub window_response: bool,
    pub frames: bool,
    pub gpu: bool,
    pub gpu_details: bool,
    pub dpc: bool,
    pub dpc_events: bool,
    pub boot: bool,
    pub process_limit: usize,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            cpu_total: true,
            cpu_cores: false,
            memory: true,
            disks: true,
            process_cpu: true,
            process_memory: true,
            process_io: true,
            window_response: true,
            frames: true,
            gpu: true,
            gpu_details: false,
            dpc: true,
            dpc_events: true,
            boot: true,
            process_limit: 5,
        }
    }
}
impl Collection {
    pub fn gpu_source(&self) -> bool {
        self.gpu || self.gpu_details
    }
    pub fn foreground_identity(&self) -> bool {
        self.window_response || self.frames || self.gpu_source()
    }
    pub fn process_snapshot(&self, boot_active: bool) -> bool {
        self.process_cpu
            || self.process_memory
            || self.process_io
            || self.frames
            || (self.boot && boot_active)
    }
    pub fn flags(&self) -> [bool; 14] {
        [
            self.cpu_total,
            self.cpu_cores,
            self.memory,
            self.disks,
            self.process_cpu,
            self.process_memory,
            self.process_io,
            self.window_response,
            self.frames,
            self.gpu,
            self.gpu_details,
            self.dpc,
            self.dpc_events,
            self.boot,
        ]
    }
    pub fn set_flags(&mut self, f: [bool; 14]) {
        self.cpu_total = f[0];
        self.cpu_cores = f[1];
        self.memory = f[2];
        self.disks = f[3];
        self.process_cpu = f[4];
        self.process_memory = f[5];
        self.process_io = f[6];
        self.window_response = f[7];
        self.frames = f[8];
        self.gpu = f[9];
        self.gpu_details = f[10];
        self.dpc = f[11];
        self.dpc_events = f[12];
        self.boot = f[13];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_configs_get_tray_defaults_and_choices_survive_save() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "schema_version = 2\nsample_ms = 1000\n").unwrap();
        let mut config = Config::load(&path).unwrap();
        assert_eq!(config.tray_display, TrayDisplay::default());
        config
            .tray_display
            .set_flags([false, false, true, true, true, false, true]);
        config.save(&path).unwrap();
        let restored = Config::load(&path).unwrap();
        assert_eq!(restored.tray_display, config.tray_display);
        assert_eq!(restored.sample_ms, 1000);
        assert_eq!(restored.collection.flags(), config.collection.flags());
    }
    #[test]
    fn migration_preserves_explicit_mib_and_age_without_overwriting_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.toml");
        let original = "max_disk_mb = 500\nretention_days = 7\ndetailed_logs = false\netw = false\nnvml = false\n";
        std::fs::write(&path, original).unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.history.ordinary_budget_bytes, 524_288_000);
        assert_eq!(c.history.retention_hours, 168);
        assert_eq!(c.history.mode, HistoryMode::Summary);
        assert!(!c.collection.frames);
        assert!(!c.collection.gpu_details);
        assert!(!c.privacy.window_titles);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        c.save(&path).unwrap();
        let reopened = Config::load(&path).unwrap();
        assert_eq!(c.logs_dir, reopened.logs_dir);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("toml.bak")).unwrap(),
            original
        );
    }
    #[test]
    fn new_install_has_decimal_500mb_24h_and_private_titles() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(c.history.ordinary_budget_bytes, 500_000_000);
        assert_eq!(c.history.retention_hours, 24);
        assert!(!c.privacy.window_titles);
        assert!(!c.collection.gpu_details);
        assert_eq!(c.manual_post_seconds, 10);
    }
    #[test]
    fn rejects_invalid_thresholds_and_overflowing_retention() {
        let mut config = Config::default();
        assert!(config.validate().is_ok());
        config.frame_ms = f64::NAN;
        assert!(config.validate().is_err());
        config.frame_ms = 100.0;
        config.retention_days = u64::MAX;
        assert!(config.validate().is_err());
        config.retention_days = 7;
        config.ring_seconds = 10;
        assert!(config.validate().is_err());
        assert!(toml::from_str::<Config>("sample_mss = 500").is_err());
    }
    #[test]
    fn creates_config_and_resolves_log_paths_relative_to_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sub/config.toml");
        let config = Config::load(&path).unwrap();
        assert_eq!(config.logs_dir, path.parent().unwrap().join("logs"));
        assert!(Config::load(&path).is_ok());
    }
}

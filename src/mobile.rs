//! Android's service adapter uses the same bounded recorder, LZ4 storage and incident windows.
use crate::{
    assessment, config::Config, detector::Detector, model::*, recorder::Recorder, storage,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub provider: String,
    pub session_id: String,
    pub elapsed_realtime_ns: u64,
    pub low_memory: Option<bool>,
    pub low_memory_threshold_bytes: Option<u64>,
    pub battery_pct: Option<f64>,
    pub battery_temperature_c: Option<f64>,
    pub charging: Option<bool>,
    pub thermal_status: Option<u32>,
    pub thermal_headroom: Option<f64>,
    pub cpu_headroom: Option<f64>,
    pub gpu_headroom: Option<f64>,
    pub own_cpu_pct: Option<f64>,
    pub own_memory_bytes: Option<u64>,
    pub cpu_frequency_mhz: Option<f64>,
    pub gpu_frequency_mhz: Option<f64>,
    pub cpu_temperature_c: Option<f64>,
    pub gpu_temperature_c: Option<f64>,
    pub fps: Option<f64>,
    pub frame_ms: Option<f64>,
    pub frame_duration_ms: Option<f64>,
    pub frame_source: String,
    pub fps_sample_elapsed_ms: Option<u64>,
    pub fps_task_id: Option<i32>,
    pub deadline_misses: u64,
    pub deadline_source: String,
    pub target_package: String,
    pub layer: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    ts_ms: u64,
    elapsed_ms: u64,
    uptime_ms: u64,
    interactive: bool,
    cpu_pct: Option<f64>,
    ram_total_bytes: u64,
    ram_available_bytes: u64,
    android: Metrics,
    status: BTreeMap<String, SourceStatus>,
    frame_stats: Option<String>,
}

/// SurfaceFlinger presentation timestamps are distinct from its refresh-period header.
#[derive(Default)]
pub struct FrameTracker {
    layer: String,
    last_ns: u64,
}

impl FrameTracker {
    pub fn read(&mut self, layer: &str, text: &str, uptime_ns: u64) -> Option<(f64, f64)> {
        if layer != self.layer {
            self.layer = layer.to_owned();
            self.last_ns = 0;
        }
        let mut lines = text.lines();
        // This line describes display refresh, NOT application frame delivery.
        lines.next()?.trim().parse::<u64>().ok()?;
        let frames: BTreeSet<u64> = lines
            .take(512)
            .filter_map(|line| {
                let values: Vec<u64> = line
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()
                    .ok()?;
                let actual = *values.get(1)?;
                (values.len() == 3
                    && actual > 0
                    && actual < i64::MAX as u64
                    && actual <= uptime_ns
                    && uptime_ns - actual <= 5_000_000_000)
                    .then_some(actual)
            })
            .collect();
        let latest = frames.last().copied()?;
        if latest <= self.last_ns {
            return None;
        }
        let mut previous = frames.iter().copied().rfind(|&v| v <= self.last_ns);
        let mut intervals = Vec::new();
        for actual in frames.iter().copied().filter(|&v| v > self.last_ns) {
            if let Some(before) = previous {
                let delta = actual.saturating_sub(before);
                if delta > 0 && delta <= 1_000_000_000 {
                    intervals.push(delta);
                }
            }
            previous = Some(actual);
        }
        self.last_ns = latest;
        if intervals.is_empty() {
            return None;
        }
        let total: u64 = intervals.iter().sum();
        let fps = intervals.len() as f64 * 1_000_000_000.0 / total as f64;
        let peak = *intervals.iter().max()? as f64 / 1_000_000.0;
        (fps.is_finite() && fps <= 1000.0).then_some((fps, peak))
    }
}

#[derive(Deserialize)]
struct Open {
    directory: PathBuf,
    #[serde(default = "default_budget")]
    budget_mb: u64,
    #[serde(default = "default_retention")]
    retention_hours: u64,
}
fn default_budget() -> u64 {
    500
}
fn default_retention() -> u64 {
    24
}

pub struct Session {
    config: Config,
    recorder: Recorder,
    detector: Detector,
    history: VecDeque<Arc<Sample>>,
    batch: Vec<Arc<Sample>>,
    frames: FrameTracker,
    previous_elapsed: Option<u64>,
    finished: bool,
    started: bool,
}

impl Session {
    pub fn open(input: &str) -> Result<Self> {
        let open: Open = serde_json::from_str(input)?;
        ensure!(
            open.directory.is_absolute(),
            "Recording directory must be absolute"
        );
        ensure!(
            (10..=4096).contains(&open.budget_mb),
            "History limit must be 10..4096 MB"
        );
        ensure!(
            (1..=168).contains(&open.retention_hours),
            "Retention must be 1..168 hours"
        );
        let mut config = Config {
            logs_dir: open.directory,
            sample_ms: 1000,
            ring_seconds: 45,
            pre_seconds: 30,
            post_seconds: 10,
            manual_seconds: 30,
            manual_post_seconds: 10,
            ..Config::default()
        };
        config.history.ordinary_budget_bytes = open.budget_mb * 1024 * 1024;
        config.history.retention_hours = open.retention_hours;
        config.history.reserve_free_bytes = 64 * 1024 * 1024;
        let mut session = Self {
            recorder: Recorder::new(&config)?,
            detector: Detector::new(&config),
            config,
            history: VecDeque::new(),
            batch: Vec::new(),
            frames: FrameTracker::default(),
            previous_elapsed: None,
            finished: false,
            started: false,
        };
        session.restore()?;
        Ok(session)
    }

    fn restore(&mut self) -> Result<()> {
        let cutoff = now_ms().saturating_sub(600_000);
        let mut files = storage::managed_files(&self.config.logs_dir)?;
        files.retain(|(p, _, _)| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("stutter-"))
        });
        files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
        let mut bytes = 0u64;
        let mut points = BTreeMap::new();
        for (path, _, size) in files.into_iter().take(1024) {
            bytes = bytes.saturating_add(size);
            if bytes > 64 * 1024 * 1024 {
                break;
            }
            crate::report::read_records(&path, |record| {
                if let Record::Sample(s) = record
                    && s.android.is_some()
                    && s.ts_ms >= cutoff
                {
                    points.insert(s.ts_ms, Arc::new(*s));
                    if points.len() > 6002 {
                        points.pop_first();
                    }
                }
            })?;
        }
        self.history = points.into_values().collect();
        Ok(())
    }

    pub fn command(&mut self, command: &str, payload: &str) -> Result<String> {
        let value = match command {
            "ingest" => {
                self.ingest(serde_json::from_str(payload)?)?;
                json!({})
            }
            "snapshot" => self.snapshot(),
            "latest" => self.latest(),
            "mark" => {
                let marked = self.detector.mark("manual_mark");
                if let Some(pending) = &self.detector.pending {
                    self.recorder.incident(pending);
                }
                json!({"marked": marked})
            }
            "flush" => {
                self.flush();
                self.recorder.barrier()?;
                json!({})
            }
            "configure" => {
                let open: Open = serde_json::from_str(payload)?;
                ensure!(
                    open.directory == self.config.logs_dir,
                    "Recording directory cannot change while open"
                );
                ensure!(
                    (10..=4096).contains(&open.budget_mb)
                        && (1..=168).contains(&open.retention_hours),
                    "Invalid history limits"
                );
                self.config.history.ordinary_budget_bytes = open.budget_mb * 1024 * 1024;
                self.config.history.retention_hours = open.retention_hours;
                self.recorder.configure(&self.config);
                json!({})
            }
            "gap" => {
                while let Some(pending) = self.detector.take_ready(true) {
                    self.recorder.incident(&pending);
                }
                self.detector.clear_history();
                self.previous_elapsed = None;
                self.frames = FrameTracker::default();
                self.flush();
                self.recorder.append(Record::Event {
                    ts_ms: now_ms(),
                    name: "android_gap".into(),
                    detail: payload.chars().take(256).collect(),
                });
                json!({})
            }
            "incidents" => json!(self.incidents()?),
            "incident" => {
                let id: String = serde_json::from_str(payload)?;
                ensure!(
                    id.len() <= 100 && id.chars().all(|c| c.is_ascii_digit() || c == '-'),
                    "Invalid incident ID"
                );
                let path = ["protected", "incidents"]
                    .into_iter()
                    .map(|dir| {
                        self.config
                            .logs_dir
                            .join(dir)
                            .join(format!("incident-{id}.jsonl.lz4"))
                    })
                    .find(|p| p.is_file())
                    .ok_or_else(|| anyhow::anyhow!("Incident unavailable"))?;
                let mut points = Vec::new();
                crate::report::read_records(&path, |record| {
                    if let Record::Sample(s) = record
                        && points.len() < 6002
                    {
                        points.push(self.point(&s));
                    }
                })?;
                json!({"header": storage::incident_header(&path)?, "points": points})
            }
            _ => anyhow::bail!("Unknown recorder command"),
        };
        Ok(serde_json::to_string(&value)?)
    }

    fn ingest(&mut self, mut input: Input) -> Result<()> {
        ensure!(
            input.ts_ms > 0 && input.ram_available_bytes <= input.ram_total_bytes,
            "Invalid sample"
        );
        for value in [
            input.cpu_pct,
            input.android.fps,
            input.android.frame_ms,
            input.android.frame_duration_ms,
            input.android.battery_pct,
            input.android.battery_temperature_c,
            input.android.cpu_headroom,
            input.android.gpu_headroom,
        ] {
            ensure!(value.is_none_or(f64::is_finite), "Non-finite metric");
        }
        if !self.started {
            self.started = true;
            self.recorder.append(Record::Event {
                ts_ms: input.ts_ms,
                name: "android_session".into(),
                detail: "clock=elapsed_realtime; platform=android".into(),
            });
        }
        let interval = self
            .previous_elapsed
            .map(|p| input.elapsed_ms.saturating_sub(p))
            .unwrap_or(1000);
        if self.previous_elapsed.is_some_and(|p| input.elapsed_ms <= p) {
            self.frames = FrameTracker::default();
            self.detector.clear_history();
        }
        self.previous_elapsed = Some(input.elapsed_ms);
        if input.android.frame_source != "android_task_fps"
            || input
                .android
                .fps_sample_elapsed_ms
                .is_none_or(|ts| ts > input.elapsed_ms || input.elapsed_ms - ts > 3000)
            || input
                .android
                .fps
                .is_some_and(|v| !(0.0..=1000.0).contains(&v))
        {
            input.android.fps = None;
        }
        input.android.frame_ms = None;
        if let Some(text) = &input.frame_stats
            && let Some((fps, frame_ms)) =
                self.frames
                    .read(&input.android.layer, text, input.uptime_ms * 1_000_000)
        {
            input.android.fps = Some(fps);
            input.android.frame_ms = Some(frame_ms);
            input.android.frame_source = "surfaceflinger_present".into();
        }
        // A missing frame deadline provider cannot confirm a jank from a long static gap.
        if !matches!(
            input.android.deadline_source.as_str(),
            "frametimeline" | "gfxinfo"
        ) {
            input.android.deadline_misses = 0;
        }
        let sample = Arc::new(Sample {
            ts_ms: input.ts_ms,
            elapsed_ms: input.elapsed_ms,
            uptime_ms: input.uptime_ms,
            interval_ms: interval,
            interactive: Some(input.interactive),
            cpu: Cpu {
                usage_pct: input.cpu_pct.filter(|v| (0.0..=100.0).contains(v)),
                ..Default::default()
            },
            memory: Memory {
                total_bytes: input.ram_total_bytes,
                available_bytes: input.ram_available_bytes,
                used_bytes: input.ram_total_bytes - input.ram_available_bytes,
                ..Default::default()
            },
            foreground: Foreground {
                program: input.android.target_package.clone(),
                ..Default::default()
            },
            android: Some(input.android),
            source_status: input.status,
            ..Default::default()
        });
        if self.detector.push(Arc::clone(&sample))
            && let Some(pending) = &self.detector.pending
        {
            self.recorder.incident(pending);
        }
        while let Some(pending) = self.detector.take_ready(false) {
            self.recorder.incident(&pending);
        }
        let cutoff = sample.ts_ms.saturating_sub(600_000);
        while self.history.front().is_some_and(|p| p.ts_ms < cutoff) || self.history.len() >= 6002 {
            self.history.pop_front();
        }
        self.history.push_back(Arc::clone(&sample));
        self.batch.push(sample);
        if self.batch.len() >= 60 {
            self.flush();
        }
        Ok(())
    }

    fn flush(&mut self) {
        self.recorder.samples(std::mem::take(&mut self.batch));
    }

    fn point(&self, s: &Sample) -> Value {
        json!({"ts": s.ts_ms, "elapsed": s.elapsed_ms, "interval": s.interval_ms,
            "interactive": s.interactive, "cpu": s.cpu.usage_pct,
            "ram": (s.memory.total_bytes > 0).then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0),
            "metrics": s.android, "alerts": assessment::signals(&self.config, s)})
    }
    fn snapshot(&self) -> Value {
        json!({"points": self.history.iter().map(|s| self.point(s)).collect::<Vec<_>>(),
            "status": self.history.back().map(|s| &s.source_status), "writer": self.recorder.status(),
            "latest": self.history.back().map(|s| self.point(s)), "pending": self.detector.pending.as_ref().map(|p| &p.header)})
    }
    fn latest(&self) -> Value {
        json!({"status": self.history.back().map(|s| &s.source_status),
            "writer": self.recorder.status(),
            "latest": self.history.back().map(|s| self.point(s)),
            "pending": self.detector.pending.as_ref().map(|p| &p.header)})
    }
    fn incidents(&self) -> Result<Vec<Incident>> {
        let mut files = storage::managed_files(&self.config.logs_dir)?;
        files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
        Ok(files
            .into_iter()
            .filter(|(p, _, _)| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("incident-"))
            })
            .take(512)
            .filter_map(|(p, _, _)| storage::incident_header(&p).ok())
            .collect())
    }
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        while let Some(pending) = self.detector.take_ready(true) {
            self.recorder.incident(&pending);
        }
        self.flush();
        self.recorder.barrier()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_fps_is_not_the_display_refresh_rate_and_old_frames_are_not_recounted() {
        let mut t = FrameTracker::default();
        let data = "8333333\n1000000000 1000000000 999000000\n1033333333 1033333333 1032000000\n1066666666 1066666666 1065000000\n";
        let (fps, ms) = t.read("game", data, 1_100_000_000).unwrap();
        assert!((fps - 30.0).abs() < 0.01);
        assert!((ms - 33.3333).abs() < 0.01);
        assert!(t.read("game", data, 1_200_000_000).is_none());
        assert!(
            t.read(
                "other",
                "8333333\n0 0 0\n0 9223372036854775807 0",
                2_000_000_000
            )
            .is_none()
        );
    }
    #[test]
    fn android_cached_ram_and_static_frame_gaps_are_not_desktop_incidents() {
        let mut s = Sample {
            interval_ms: 1000,
            android: Some(Metrics::default()),
            memory: Memory {
                total_bytes: 1000,
                used_bytes: 999,
                available_bytes: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(assessment::signals(&Config::default(), &s), 0);
        s.android.as_mut().unwrap().frame_ms = Some(1000.0);
        assert!(assessment::reasons(&Config::default(), &s).is_empty());
        s.android.as_mut().unwrap().deadline_misses = 1;
        s.android.as_mut().unwrap().frame_duration_ms = Some(24.0);
        assert!(assessment::reasons(&Config::default(), &s).is_empty());
        assert_eq!(
            assessment::signals(&Config::default(), &s),
            assessment::FRAME_JANK
        );
        s.android.as_mut().unwrap().frame_duration_ms = Some(150.0);
        assert_eq!(
            assessment::reasons(&Config::default(), &s),
            vec!["android_frame_jank"]
        );
        s.interactive = Some(false);
        assert_eq!(assessment::signals(&Config::default(), &s), 0);
    }
    #[test]
    fn mobile_history_and_protected_mark_restore_after_service_restart() {
        let temp = tempfile::tempdir().unwrap();
        let open = json!({"directory":temp.path()}).to_string();
        let start = now_ms() - 60_000;
        let mut session = Session::open(&open).unwrap();
        for n in 0..45 {
            session.command("ingest", &json!({"ts_ms":start+n*1000,"elapsed_ms":n*1000,"uptime_ms":n*1000,"interactive":true,"ram_total_bytes":1000,"ram_available_bytes":400,"android":{"session_id":"test"}}).to_string()).unwrap();
            if n == 32 {
                session.command("mark", "").unwrap();
            }
        }
        session.finish().unwrap();
        drop(session);
        let mut restored = Session::open(&open).unwrap();
        let snapshot: Value =
            serde_json::from_str(&restored.command("snapshot", "").unwrap()).unwrap();
        assert_eq!(snapshot["points"].as_array().unwrap().len(), 45);
        let incidents: Value =
            serde_json::from_str(&restored.command("incidents", "").unwrap()).unwrap();
        assert!(incidents[0]["protected"].as_bool().unwrap());
        assert!(incidents[0]["complete"].as_bool().unwrap());
    }
}

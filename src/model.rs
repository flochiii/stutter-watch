use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
    pub created_ms: u64,
    pub name: String,
    pub cpu_pct: f64,
    pub threads: u32,
    pub base_priority: i32,
    pub working_set_bytes: u64,
    pub private_bytes: Option<u64>,
    pub io_read_bytes_sec: Option<f64>,
    pub io_write_bytes_sec: Option<f64>,
    /// Includes soft faults. Windows does not expose per-process hard faults in this counter.
    pub page_faults_sec: Option<f64>,
    pub hard_page_faults_sec: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Core {
    pub logical_processor: String,
    pub efficiency_class: Option<u8>,
    pub usage_pct: Option<f64>,
    pub frequency_mhz: Option<f64>,
    pub effective_frequency_mhz: Option<f64>,
    pub performance_pct: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cpu {
    pub usage_pct: Option<f64>,
    pub cores: Vec<Core>,
    pub top: Vec<Process>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Memory {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub standby_bytes: Option<f64>,
    pub cache_bytes: Option<f64>,
    pub commit_bytes: Option<f64>,
    pub commit_limit_bytes: Option<f64>,
    pub pagefile_used_bytes: Option<u64>,
    pub page_reads_sec: Option<f64>,
    pub pages_input_sec: Option<f64>,
    pub top: Vec<Process>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiskDevice {
    pub name: String,
    pub read_mb_sec: Option<f64>,
    pub write_mb_sec: Option<f64>,
    pub read_latency_ms: Option<f64>,
    pub write_latency_ms: Option<f64>,
    pub queue: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Disk {
    pub devices: Vec<DiskDevice>,
    pub top: Vec<Process>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Foreground {
    pub hwnd: u64,
    pub pid: u32,
    pub title: String,
    #[serde(default)]
    pub program: String,
    pub responsive: Option<bool>,
    pub unresponsive_ms: u64,
    pub fullscreen: bool,
    #[serde(default)]
    pub response_peak_ms: Option<u64>,
    #[serde(default)]
    pub response_events: Vec<WindowDelay>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowDelay {
    pub ts_ms: u64,
    pub pid: u32,
    pub duration_ms: u64,
    pub probe_interval_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GpuProcess {
    pub pid: u32,
    pub name: Option<String>,
    pub engine: String,
    pub usage_pct: f64,
    pub dedicated_bytes: Option<f64>,
    pub shared_bytes: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NvmlMetrics {
    pub core_mhz: Option<u32>,
    pub memory_mhz: Option<u32>,
    pub temperature_c: Option<u32>,
    pub power_w: Option<f64>,
    pub tgp_w: f64,
    pub power_tgp_pct: Option<f64>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub clocks_throttle_reasons: Option<u64>,
    pub throttle_reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Adapter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensors: Option<NvmlMetrics>,
    pub name: String,
    pub vendor_id: u32,
    pub luid: String,
    /// Utilization of the busiest physical engine, summed across processes.
    pub usage_pct: Option<f64>,
    /// active, sleeping, idle_d0 or unknown; PDH silence alone never means sleep.
    pub state: String,
    pub most_recent_power_state: Option<u32>,
    pub processes: Vec<GpuProcess>,
    pub nvml: Option<NvmlMetrics>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GpuTransition {
    pub ts_ms: u64,
    pub adapter: String,
    pub from: String,
    pub to: String,
    pub process_candidates: Vec<u32>,
    pub attribution: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Gpu {
    pub adapters: Vec<Adapter>,
    pub transitions: Vec<GpuTransition>,
    pub nvml_status: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Frames {
    pub status: String,
    pub source: String,
    pub pid: u32,
    pub fps: Option<f64>,
    pub frame_time_ms: Option<f64>,
    pub max_frame_time_ms: Option<f64>,
    pub low_1pct_fps: Option<f64>,
    pub low_01pct_fps: Option<f64>,
    pub dropped_frames: Option<u64>,
    pub dwm_max_gap_ms: Option<f64>,
    pub dwm_work_ms: Option<f64>,
    pub events_lost: u32,
    #[serde(default)]
    pub present_work_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DriverSpike {
    pub ts_ms: u64,
    pub kind: String,
    pub duration_ms: f64,
    pub routine: String,
    pub module: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Dpc {
    pub time_pct: Option<f64>,
    pub interrupt_pct: Option<f64>,
    pub queued_sec: Option<f64>,
    pub kernel_status: String,
    pub spikes: Vec<DriverSpike>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BootProcess {
    pub pid: u32,
    pub created_ms: u64,
    pub first_seen_ms: u64,
    pub name: String,
    pub peak_cpu_pct: f64,
    pub peak_io_bytes_sec: f64,
    pub already_running: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Boot {
    pub active: bool,
    pub stable_at_ms: Option<u64>,
    pub processes: Vec<BootProcess>,
    pub observation: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sample {
    pub ts_ms: u64,
    pub elapsed_ms: u64,
    pub uptime_ms: u64,
    pub interval_ms: u64,
    pub collection_ms: f64,
    pub cpu: Cpu,
    pub memory: Memory,
    pub disk: Disk,
    pub foreground: Foreground,
    pub gpu: Gpu,
    pub frames: Frames,
    pub dpc: Dpc,
    pub boot: Boot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<crate::config::Collection>,
    #[serde(default)]
    pub interactive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<Pressure>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_status: BTreeMap<String, SourceStatus>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceStatus {
    pub available: bool,
    pub reason: String,
    #[serde(default)]
    pub partial: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pressure {
    pub cpu: Option<PressureResource>,
    pub memory: Option<PressureResource>,
    pub io: Option<PressureResource>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PressureResource {
    pub some_pct: Option<f64>,
    pub full_pct: Option<f64>,
    pub avg10: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stats {
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub min_ts_ms: u64,
    #[serde(default)]
    pub max_ts_ms: u64,
    pub min: f64,
    pub avg: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub start_ms: u64,
    pub end_ms: u64,
    pub samples: usize,
    pub metrics: BTreeMap<String, Stats>,
    pub top_cpu: Vec<Process>,
    pub top_memory: Vec<Process>,
    pub top_io: Vec<Process>,
    pub gpu_transitions: Vec<GpuTransition>,
    pub boot: Boot,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub ts_ms: u64,
    pub reason: String,
    pub foreground: Foreground,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub complete: bool,
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub protected: bool,
    #[serde(default)]
    pub requested_start_ms: u64,
    #[serde(default)]
    pub requested_end_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    ExternalLog {
        source: String,
        basename: String,
        sha256: String,
        imported_ms: u64,
        time_basis: String,
    },
    ExternalFrame {
        elapsed_ns: u64,
        fps: f64,
        frametime_ms: f64,
    },
    Session {
        ts_ms: u64,
        version: String,
        warnings: Vec<String>,
        #[serde(default)]
        pid: u32,
        #[serde(default)]
        elevated: bool,
        #[serde(default)]
        sample_ms: u64,
        #[serde(default)]
        flush_seconds: u64,
        #[serde(default)]
        detailed_logs: bool,
    },
    Summary(Box<Summary>),
    Incident(Incident),
    Sample(Box<Sample>),
    Mark(Trigger),
    Event {
        ts_ms: u64,
        name: String,
        detail: String,
    },
}

pub fn top_processes(processes: &[Process], metric: impl Fn(&Process) -> f64) -> Vec<Process> {
    let mut rows: Vec<_> = processes.iter().collect();
    rows.sort_unstable_by(|a, b| metric(b).total_cmp(&metric(a)));
    rows.into_iter().take(5).cloned().collect()
}

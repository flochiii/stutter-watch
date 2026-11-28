use super::*;
use nvml_wrapper::{
    Nvml,
    bitmasks::InitFlags,
    enum_wrappers::device::{Clock, TemperatureSensor},
};
use std::{
    collections::{BTreeMap, HashSet},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

pub struct Collector {
    request: SyncSender<Config>,
    reply: Receiver<Gpu>,
    last: Option<(Instant, Gpu)>,
}
impl Collector {
    pub fn new() -> Self {
        let (tx, rx) = sync_channel::<Config>(1);
        let (result_tx, result_rx) = sync_channel(1);
        std::thread::spawn(move || {
            let mut worker = Worker {
                old: BTreeMap::new(),
                previous: Instant::now(),
            };
            while let Ok(c) = rx.recv() {
                let result = worker.collect(&c);
                let _ = result_tx.try_send(result);
            }
        });
        Self {
            request: tx,
            reply: result_rx,
            last: None,
        }
    }
    pub fn collect(&mut self, c: &Config, status: &mut BTreeMap<String, SourceStatus>) -> Gpu {
        while let Ok(value) = self.reply.try_recv() {
            self.last = Some((Instant::now(), value));
        }
        let _ = self.request.try_send(c.clone());
        let result = self
            .last
            .as_ref()
            .filter(|(when, _)| when.elapsed() < Duration::from_secs(3))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        status.insert(
            "gpu".into(),
            SourceStatus {
                available: result.adapters.iter().any(|a| a.usage_pct.is_some()),
                reason: if result.adapters.is_empty() {
                    "linux_gpu_unavailable".into()
                } else {
                    "linux_gpu_partial".into()
                },
                partial: true,
            },
        );
        status.insert(
            "gpu_details".into(),
            SourceStatus {
                available: result
                    .adapters
                    .iter()
                    .any(|a| a.nvml.is_some() || a.sensors.is_some()),
                reason: "linux_gpu_power_guard".into(),
                partial: true,
            },
        );
        result
    }
}
struct Worker {
    old: BTreeMap<(String, String), u64>,
    previous: Instant,
}
impl Worker {
    fn collect(&mut self, c: &Config) -> Gpu {
        let elapsed = self.previous.elapsed().as_nanos() as f64;
        self.previous = Instant::now();
        let mut current = BTreeMap::new();
        let mut engine_usage: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        let mut seen = HashSet::new();
        if let Ok(processes) = std::fs::read_dir("/proc") {
            for process in processes.flatten().take(65536) {
                if process
                    .file_name()
                    .to_str()
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_none()
                {
                    continue;
                }
                let Ok(fds) = std::fs::read_dir(process.path().join("fdinfo")) else {
                    continue;
                };
                for fd in fds.flatten().take(4096) {
                    let Ok(raw) = std::fs::read_to_string(fd.path()) else {
                        continue;
                    };
                    if !raw.contains("drm-driver:") {
                        continue;
                    }
                    let m: BTreeMap<_, _> = raw
                        .lines()
                        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim(), v.trim())))
                        .collect();
                    let Some(pci) = m.get("drm-pdev").copied() else {
                        continue;
                    };
                    let Some(client) = m.get("drm-client-id").copied() else {
                        continue;
                    };
                    if !seen.insert((pci.to_string(), client.to_string())) {
                        continue;
                    }
                    for (key, value) in &m {
                        if !key.starts_with("drm-engine-")
                            || key.starts_with("drm-engine-capacity-")
                        {
                            continue;
                        }
                        let Some(ticks) = value
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<u64>().ok())
                        else {
                            continue;
                        };
                        let history_key = (pci.to_string(), format!("{client}:{key}"));
                        if let Some(delta) = self
                            .old
                            .get(&history_key)
                            .and_then(|old| ticks.checked_sub(*old))
                            .filter(|_| elapsed > 0.0)
                        {
                            let capacity = m
                                .get(
                                    format!(
                                        "drm-engine-capacity-{}",
                                        key.trim_start_matches("drm-engine-")
                                    )
                                    .as_str(),
                                )
                                .and_then(|v| v.parse::<f64>().ok())
                                .filter(|v| *v > 0.0)
                                .unwrap_or(1.0);
                            *engine_usage
                                .entry(pci.into())
                                .or_default()
                                .entry(key.to_string())
                                .or_default() += delta as f64 / elapsed / capacity * 100.0;
                        }
                        current.insert(history_key, ticks);
                    }
                }
            }
        }
        self.old = current;
        let mut adapters = Vec::new();
        let mut pci_seen = HashSet::new();
        if let Ok(cards) = std::fs::read_dir("/sys/class/drm") {
            for card in cards.flatten() {
                let name = card.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with("card") || !name[4..].chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let device = card.path().join("device");
                let Ok(device) = device.canonicalize() else {
                    continue;
                };
                let Some(pci) = device.file_name().map(|v| v.to_string_lossy().into_owned()) else {
                    continue;
                };
                if !pci_seen.insert(pci.clone()) {
                    continue;
                }
                let read = |name: &str| {
                    std::fs::read_to_string(device.join(name))
                        .ok()
                        .map(|v| v.trim().to_owned())
                };
                let vendor = read("vendor")
                    .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(0);
                let usage = engine_usage
                    .get(&pci)
                    .and_then(|m| m.values().copied().max_by(f64::total_cmp))
                    .map(|v| v.clamp(0.0, 100.0));
                let active = read("power/runtime_status").as_deref() == Some("active");
                let busy = usage.is_some_and(|v| v > 0.5);
                let mut adapter = Adapter {
                    name: format!(
                        "{} ({pci})",
                        match vendor {
                            0x10de => "NVIDIA",
                            0x1002 => "AMD",
                            0x8086 => "Intel",
                            _ => "DRM GPU",
                        }
                    ),
                    vendor_id: vendor,
                    luid: pci.clone(),
                    usage_pct: usage,
                    state: if active {
                        if busy { "active" } else { "idle_d0" }
                    } else {
                        "unknown"
                    }
                    .into(),
                    ..Default::default()
                };
                if c.collection.gpu_details
                    && active
                    && busy
                    && read("power/runtime_status").as_deref() == Some("active")
                {
                    if vendor == 0x10de && std::env::var_os("NVML_MPS_DEVICE_ENUMERATION").is_none()
                    {
                        if let Ok(nvml) = Nvml::builder()
                            .flags(InitFlags::NO_GPUS | InitFlags::NO_ATTACH)
                            .init()
                        {
                            if let Ok(gpu) = nvml.device_by_pci_bus_id(pci.as_str()) {
                                let memory = gpu.memory_info().ok();
                                adapter.nvml = Some(NvmlMetrics {
                                    core_mhz: gpu.clock_info(Clock::Graphics).ok(),
                                    memory_mhz: gpu.clock_info(Clock::Memory).ok(),
                                    temperature_c: gpu.temperature(TemperatureSensor::Gpu).ok(),
                                    power_w: gpu.power_usage().ok().map(|v| v as f64 / 1000.0),
                                    vram_used_bytes: memory.as_ref().map(|m| m.used),
                                    vram_total_bytes: memory.map(|m| m.total),
                                    ..Default::default()
                                });
                            }
                        }
                    } else if vendor == 0x1002 || vendor == 0x8086 {
                        let mut sensor = NvmlMetrics::default();
                        sensor.vram_used_bytes =
                            read("mem_info_vram_used").and_then(|v| v.parse().ok());
                        sensor.vram_total_bytes =
                            read("mem_info_vram_total").and_then(|v| v.parse().ok());
                        if let Ok(entries) = std::fs::read_dir(device.join("hwmon")) {
                            for entry in entries.flatten().take(8) {
                                let number = |file: &str| {
                                    std::fs::read_to_string(entry.path().join(file))
                                        .ok()
                                        .and_then(|v| v.trim().parse::<f64>().ok())
                                };
                                sensor.temperature_c =
                                    number("temp1_input").map(|v| (v / 1000.0) as u32);
                                sensor.power_w = number("power1_average").map(|v| v / 1_000_000.0);
                            }
                        }
                        adapter.sensors = Some(sensor);
                    }
                }
                adapters.push(adapter);
            }
        }
        Gpu {
            adapters,
            nvml_status: "guarded_active_only".into(),
            ..Default::default()
        }
    }
}

use crate::{
    collectors::{boot_profiler, cpu_mem, disk, dpc, etw_frames, foreground, gpu_optimus, pdh},
    config::Config,
    model::*,
};
use anyhow::Result;
use std::time::Instant;
use windows::Win32::System::SystemInformation::GetTickCount64;

pub struct Sources {
    query: pdh::Query,
    cpu: cpu_mem::CpuMem,
    foreground: Option<foreground::Collector>,
    gpu: Option<gpu_optimus::Collector>,
    pub frames: etw_frames::Collector,
    boot: boot_profiler::Profiler,
    config: Config,
    pub warnings: Vec<String>,
    boot_was_active: bool,
}
impl Sources {
    pub fn new(config: &Config, boot_trace: bool) -> Result<Self> {
        crate::startup::phase("sources.planning_counters");
        let mut query = pdh::Query::new()?;
        let c = &config.collection;
        cpu_mem::register(&mut query, c);
        if c.disks {
            disk::register(&mut query);
        }
        if c.gpu_source() {
            gpu_optimus::register(&mut query);
        }
        if c.dpc {
            dpc::register(&mut query);
        }
        crate::startup::phase("sources.gpu_inventory");
        let gpu = c.gpu_source().then(|| gpu_optimus::Collector::new(config));
        let boot = boot_profiler::Profiler::new(
            unsafe { GetTickCount64() },
            now_ms(),
            if c.boot { config.boot_seconds } else { 0 },
            boot_trace,
        );
        let mut etw_config = config.clone();
        etw_config.collection.boot = boot.active();
        crate::startup::phase("sources.etw");
        let frames = etw_frames::Collector::new(&etw_config);
        let mut warnings = query.warnings.clone();
        if let Some(gpu) = &gpu {
            warnings.extend(gpu.warnings.iter().cloned());
        }
        warnings.extend(frames.warnings.iter().cloned());
        crate::startup::phase("sources.cpu_and_window");
        Ok(Self {
            query,
            cpu: cpu_mem::CpuMem::new(c),
            foreground: c
                .foreground_identity()
                .then(|| foreground::Collector::new(c.window_response)),
            gpu,
            frames,
            boot_was_active: boot.active(),
            boot,
            config: config.clone(),
            warnings,
        })
    }
    pub fn pause(&mut self) {
        self.frames.pause();
        if let Some(gpu) = &mut self.gpu {
            gpu.pause();
        }
        self.foreground = None;
    }
    pub fn collect(&mut self, start: Instant, interval_ms: u64) -> Sample {
        let tick = Instant::now();
        let c = &self.config.collection;
        self.warnings.retain(|warning| !warning.starts_with("PDH"));
        let values = match self.query.collect() {
            Ok(v) => v,
            Err(e) => {
                let error = format!("PDH: {e:#}");
                if !self.warnings.contains(&error) {
                    self.warnings.push(error);
                }
                pdh::Values::new()
            }
        };
        self.warnings.extend(self.query.warnings.iter().cloned());
        let (cpu, memory) = match self.cpu.collect(&values, c, self.boot.active()) {
            Ok(v) => v,
            Err(e) => {
                let error = format!("CPU/memory: {e:#}");
                if !self.warnings.contains(&error) {
                    self.warnings.push(error);
                }
                self.cpu.processes.clear();
                (Cpu::default(), Memory::default())
            }
        };
        let interactive = if c.foreground_identity() {
            Some(foreground::interactive())
        } else {
            None
        };
        let active = if interactive != Some(false) {
            self.foreground
                .as_mut()
                .map(|f| f.collect(c.window_response, self.config.privacy.window_titles))
                .unwrap_or_default()
        } else {
            Foreground::default()
        };
        let ts_ms = now_ms();
        let mut sample = Sample {
            ts_ms,
            elapsed_ms: start.elapsed().as_millis() as u64,
            uptime_ms: unsafe { GetTickCount64() },
            interval_ms,
            cpu,
            memory,
            disk: if c.disks || c.process_io {
                disk::collect(
                    &values,
                    if c.process_io {
                        &self.cpu.processes
                    } else {
                        &[]
                    },
                    c.process_limit,
                )
            } else {
                Disk::default()
            },
            frames: if c.frames {
                self.frames.collect(active.pid, &self.cpu.processes)
            } else {
                Frames {
                    status: "disabled".into(),
                    ..Default::default()
                }
            },
            foreground: active,
            gpu: self
                .gpu
                .as_mut()
                .map(|g| g.collect(&values, &self.cpu.processes, ts_ms))
                .unwrap_or_else(|| Gpu {
                    nvml_status: "disabled".into(),
                    ..Default::default()
                }),
            dpc: if c.dpc || c.dpc_events {
                dpc::collect(&values, &self.frames.kernel_status, &self.frames.kernel)
            } else {
                Dpc {
                    kernel_status: "disabled".into(),
                    ..Default::default()
                }
            },
            sources: Some(c.clone()),
            interactive,
            ..Default::default()
        };
        if c.boot && self.boot.active() {
            let starts = self
                .frames
                .kernel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain_starts();
            sample.boot = self.boot.collect(&sample, &self.cpu.processes, starts);
        }
        if self.boot_was_active && !self.boot.active() {
            let mut config = self.config.clone();
            config.collection.boot = false;
            self.frames.pause();
            self.frames = etw_frames::Collector::new(&config);
            self.boot_was_active = false;
        }
        sample.collection_ms = tick.elapsed().as_secs_f64() * 1000.0;
        sample
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_sources_register_no_counters_or_collectors() {
        let mut c = Config::default();
        c.collection.set_flags([false; 14]);
        let mut s = Sources::new(&c, false).unwrap();
        assert!(s.query.registered_keys().is_empty());
        assert!(s.gpu.is_none());
        assert!(s.foreground.is_none());
        assert!(!s.boot.active());
        assert_eq!(s.frames.status, "disabled");
        assert_eq!(s.frames.kernel_status, "disabled");
        let sample = s.collect(Instant::now(), 500);
        assert!(sample.cpu.usage_pct.is_none());
        assert!(sample.cpu.cores.is_empty());
        assert_eq!(sample.memory.total_bytes, 0);
        assert!(s.cpu.processes.is_empty());
        assert!(sample.disk.devices.is_empty());
    }
    #[test]
    fn memory_only_does_not_register_other_groups() {
        let mut c = Config::default();
        c.collection.set_flags([false; 14]);
        c.collection.memory = true;
        let s = Sources::new(&c, false).unwrap();
        assert!(
            s.query
                .registered_keys()
                .iter()
                .all(|k| !k.starts_with("core_")
                    && !k.starts_with("gpu_")
                    && !k.starts_with("disk_")
                    && *k != "dpc")
        );
        assert!(s.foreground.is_none());
        assert!(s.gpu.is_none());
    }
}

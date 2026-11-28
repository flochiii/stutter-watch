use crate::model::{Boot, BootProcess, Process};
use std::collections::HashMap;

pub struct Profiler {
    deadline_uptime: u64,
    started_ms: u64,
    known: HashMap<(u32, u64), BootProcess>,
    quiet_since: Option<u64>,
    stable: Option<u64>,
    finished: bool,
}
impl Profiler {
    pub fn active(&self) -> bool {
        !self.finished
    }
    pub fn new(uptime_ms: u64, started_ms: u64, seconds: u64, force: bool) -> Self {
        Self {
            deadline_uptime: if force {
                uptime_ms + seconds * 1000
            } else {
                seconds * 1000
            },
            started_ms,
            known: HashMap::new(),
            quiet_since: None,
            stable: None,
            finished: seconds == 0 || (!force && uptime_ms >= seconds * 1000),
        }
    }
    pub fn collect(
        &mut self,
        sample: &crate::model::Sample,
        processes: &[Process],
        starts: Vec<BootProcess>,
    ) -> Boot {
        if self.finished {
            return Boot {
                stable_at_ms: self.stable,
                ..Default::default()
            };
        }
        let mut changed = HashMap::new();
        for process in processes {
            let key = (process.pid, process.created_ms);
            if self.known.len() >= 4096 && !self.known.contains_key(&key) {
                continue;
            }
            let is_new = !self.known.contains_key(&key);
            let entry = self.known.entry(key).or_insert_with(|| BootProcess {
                pid: process.pid,
                created_ms: process.created_ms,
                first_seen_ms: sample.ts_ms,
                name: process.name.clone(),
                already_running: process.created_ms < self.started_ms,
                ..Default::default()
            });
            let io = process.io_read_bytes_sec.unwrap_or(0.0)
                + process.io_write_bytes_sec.unwrap_or(0.0);
            if is_new || process.cpu_pct > entry.peak_cpu_pct || io > entry.peak_io_bytes_sec {
                entry.peak_cpu_pct = entry.peak_cpu_pct.max(process.cpu_pct);
                entry.peak_io_bytes_sec = entry.peak_io_bytes_sec.max(io);
                changed.insert(key, entry.clone());
            }
        }
        // Keep short-lived processes observed by ETW even when they vanish between polls.
        for event in starts {
            if self
                .known
                .values()
                .any(|p| p.pid == event.pid && p.created_ms.abs_diff(event.created_ms) < 2000)
            {
                continue;
            }
            let key = (event.pid, event.created_ms);
            if self.known.len() < 4096 {
                self.known.insert(key, event.clone());
                changed.insert(key, event);
            }
        }
        let io: f64 = sample
            .disk
            .devices
            .iter()
            .map(|d| d.read_mb_sec.unwrap_or(0.0) + d.write_mb_sec.unwrap_or(0.0))
            .sum();
        let quiet = sample.cpu.usage_pct.is_some_and(|v| v < 10.0)
            && !sample.disk.devices.is_empty()
            && io < 1.0
            && sample
                .disk
                .devices
                .iter()
                .all(|d| d.queue.is_some_and(|v| v < 1.0));
        if quiet {
            let since = self.quiet_since.get_or_insert(sample.uptime_ms);
            if sample.uptime_ms.saturating_sub(*since) >= 15_000 && self.stable.is_none() {
                self.stable = Some(sample.ts_ms);
            }
        } else {
            self.quiet_since = None;
        }
        if sample.uptime_ms >= self.deadline_uptime {
            self.finished = true;
            // Final peaks, once, survive the end of the boot observation window.
            changed = self.known.clone();
            self.known.clear();
        }
        Boot{active:!self.finished,stable_at_ms:self.stable,processes:changed.into_values().collect(),
            observation:"500ms process snapshots + ETW starts when available; peaks of processes shorter than a sample may be unmeasured; stable = CPU<10%, disk<1MiB/s, queue<1 for 15s".into()}
    }
}

//! Read-only Linux counters. Parsers accept explicit roots for deterministic fixtures.
use crate::{config::Config, model::*};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Default)]
pub struct CpuTicks {
    pub total: u64,
    pub idle: u64,
}
pub fn cpu_ticks(line: &str) -> Option<CpuTicks> {
    let numbers: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (numbers.len() >= 4).then(|| CpuTicks {
        total: numbers.iter().sum(),
        idle: numbers[3] + numbers.get(4).copied().unwrap_or(0),
    })
}
pub fn cpu_usage(old: &CpuTicks, new: &CpuTicks) -> Option<f64> {
    let total = new.total.checked_sub(old.total)?;
    let idle = new.idle.checked_sub(old.idle)?;
    (total > 0 && idle <= total).then(|| 100.0 * (total - idle) as f64 / total as f64)
}
pub fn memory(text: &str) -> Option<Memory> {
    let values: BTreeMap<&str, u64> = text
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((
                k,
                v.split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)?,
            ))
        })
        .collect();
    let total = *values.get("MemTotal")?;
    let available = *values.get("MemAvailable")?;
    if total == 0 || available > total {
        return None;
    }
    Some(Memory {
        total_bytes: total,
        available_bytes: available,
        used_bytes: total - available,
        cache_bytes: values
            .get("Cached")
            .map(|v| (*v + values.get("SReclaimable").copied().unwrap_or(0)) as f64),
        commit_bytes: values.get("Committed_AS").map(|v| *v as f64),
        commit_limit_bytes: values.get("CommitLimit").map(|v| *v as f64),
        pagefile_used_bytes: values
            .get("SwapTotal")
            .zip(values.get("SwapFree"))
            .map(|(a, b)| a.saturating_sub(*b)),
        ..Default::default()
    })
}
#[derive(Clone, Debug)]
pub struct ProcessTicks {
    pub process: Process,
    pub ticks: u64,
    pub start: u64,
}
pub fn process_stat(
    text: &str,
    ticks_per_second: u64,
    page_size: u64,
    boot_ms: u64,
) -> Option<ProcessTicks> {
    let (pid, rest) = text.split_once(" (")?;
    let end = rest.rfind(") ")?;
    let name = rest[..end].to_owned();
    let fields: Vec<_> = rest[end + 2..].split_whitespace().collect();
    let n = |i: usize| fields.get(i)?.parse::<u64>().ok();
    let start = n(19)?;
    Some(ProcessTicks {
        ticks: n(11)?.checked_add(n(12)?)?,
        start,
        process: Process {
            pid: pid.parse().ok()?,
            parent_pid: n(1)? as u32,
            name,
            created_ms: boot_ms + start.saturating_mul(1000) / ticks_per_second.max(1),
            threads: n(17)? as u32,
            working_set_bytes: n(21)?.saturating_mul(page_size),
            ..Default::default()
        },
    })
}
#[derive(Clone, Debug, Default)]
pub struct DiskTicks {
    pub reads: u64,
    pub read_sectors: u64,
    pub read_ms: u64,
    pub writes: u64,
    pub write_sectors: u64,
    pub write_ms: u64,
    pub weighted_ms: u64,
}
pub fn disks(text: &str) -> BTreeMap<String, (String, DiskTicks)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            let n = |i: usize| f.get(i)?.parse().ok();
            Some((
                f.get(2)?.to_string(),
                (
                    format!("{}:{}", f.first()?, f.get(1)?),
                    DiskTicks {
                        reads: n(3)?,
                        read_sectors: n(5)?,
                        read_ms: n(6)?,
                        writes: n(7)?,
                        write_sectors: n(9)?,
                        write_ms: n(10)?,
                        weighted_ms: n(13)?,
                    },
                ),
            ))
        })
        .collect()
}
pub fn disk_delta(name: &str, a: &DiskTicks, b: &DiskTicks, interval_ms: u64) -> DiskDevice {
    let latency = |ops: Option<u64>, ms: Option<u64>| {
        ops.filter(|v| *v > 0)
            .zip(ms)
            .map(|(ops, ms)| ms as f64 / ops as f64)
    };
    let rate = |x: u64, y: u64| {
        y.checked_sub(x)
            .filter(|_| interval_ms > 0)
            .map(|v| v as f64 * 512.0 / 1_000_000.0 * 1000.0 / interval_ms as f64)
    };
    DiskDevice {
        name: name.into(),
        read_mb_sec: rate(a.read_sectors, b.read_sectors),
        write_mb_sec: rate(a.write_sectors, b.write_sectors),
        read_latency_ms: latency(
            b.reads.checked_sub(a.reads),
            b.read_ms.checked_sub(a.read_ms),
        ),
        write_latency_ms: latency(
            b.writes.checked_sub(a.writes),
            b.write_ms.checked_sub(a.write_ms),
        ),
        queue: b
            .weighted_ms
            .checked_sub(a.weighted_ms)
            .filter(|_| interval_ms > 0)
            .map(|v| v as f64 / interval_ms as f64),
    }
}
#[derive(Clone, Debug, Default)]
pub struct PressureTicks {
    some: Option<u64>,
    full: Option<u64>,
    avg10: Option<f64>,
}
pub fn pressure(text: &str) -> PressureTicks {
    let mut result = PressureTicks::default();
    for l in text.lines() {
        let mut f = l.split_whitespace();
        let kind = f.next();
        let fields: BTreeMap<_, _> = f.filter_map(|v| v.split_once('=')).collect();
        match kind {
            Some("some") => {
                result.some = fields.get("total").and_then(|v| v.parse().ok());
                result.avg10 = fields
                    .get("avg10")
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v));
            }
            Some("full") => result.full = fields.get("total").and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    result
}
pub fn pressure_delta(a: &PressureTicks, b: &PressureTicks, interval_ms: u64) -> PressureResource {
    let delta = |a: Option<u64>, b: Option<u64>| {
        a.zip(b)
            .and_then(|(a, b)| b.checked_sub(a))
            .filter(|_| (100..=3000).contains(&interval_ms))
            .map(|v| (v as f64 / interval_ms as f64 / 10.0).clamp(0.0, 100.0))
    };
    PressureResource {
        some_pct: delta(a.some, b.some),
        full_pct: delta(a.full, b.full),
        avg10: b.avg10,
    }
}

type ProcessHistory = BTreeMap<u32, (u64, u64, Option<(u64, u64)>)>;

pub struct Procfs {
    root: PathBuf,
    sys: PathBuf,
    old_cpu: BTreeMap<String, CpuTicks>,
    old_disks: BTreeMap<String, (String, DiskTicks)>,
    old_pressure: BTreeMap<String, PressureTicks>,
    old_processes: ProcessHistory,
    ticks: u64,
    pages: u64,
}
impl Procfs {
    pub fn new(root: PathBuf, sys: PathBuf, ticks: u64, pages: u64) -> Self {
        Self {
            root,
            sys,
            ticks,
            pages,
            old_cpu: BTreeMap::new(),
            old_disks: BTreeMap::new(),
            old_pressure: BTreeMap::new(),
            old_processes: BTreeMap::new(),
        }
    }
    pub fn collect(&mut self, c: &Config, elapsed_ms: u64, interval_ms: u64) -> Sample {
        let mut s = Sample {
            ts_ms: now_ms(),
            elapsed_ms,
            interval_ms,
            sources: Some(c.collection.clone()),
            ..Default::default()
        };
        let stat = std::fs::read_to_string(self.root.join("stat")).unwrap_or_default();
        let boot_ms = stat
            .lines()
            .find_map(|l| l.strip_prefix("btime "))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1000);
        s.uptime_ms = std::fs::read_to_string(self.root.join("uptime"))
            .ok()
            .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
            .map(|v| (v * 1000.0) as u64)
            .unwrap_or(0);
        let cpu: BTreeMap<_, _> = stat
            .lines()
            .filter(|l| l.starts_with("cpu"))
            .filter_map(|l| Some((l.split_whitespace().next()?.to_string(), cpu_ticks(l)?)))
            .collect();
        let total_delta = self
            .old_cpu
            .get("cpu")
            .zip(cpu.get("cpu"))
            .and_then(|(a, b)| b.total.checked_sub(a.total))
            .filter(|v| *v > 0);
        if c.collection.cpu_total {
            s.cpu.usage_pct = self
                .old_cpu
                .get("cpu")
                .zip(cpu.get("cpu"))
                .and_then(|(a, b)| cpu_usage(a, b));
        }
        if c.collection.cpu_cores {
            for (name, b) in cpu.iter().filter(|(n, _)| n.as_str() != "cpu") {
                s.cpu.cores.push(Core {
                    logical_processor: name.trim_start_matches("cpu").into(),
                    usage_pct: self.old_cpu.get(name).and_then(|a| cpu_usage(a, b)),
                    ..Default::default()
                });
            }
        }
        self.old_cpu = cpu;
        if c.collection.memory
            && let Some(mem) = std::fs::read_to_string(self.root.join("meminfo"))
                .ok()
                .and_then(|v| memory(&v))
        {
            s.memory = mem;
        }
        if c.collection.disks {
            let current =
                disks(&std::fs::read_to_string(self.root.join("diskstats")).unwrap_or_default());
            for (name, (device, b)) in &current {
                if name.starts_with("loop")
                    || name.starts_with("ram")
                    || self
                        .sys
                        .join("dev/block")
                        .join(device)
                        .join("partition")
                        .exists()
                {
                    continue;
                }
                if let Some((_, a)) = self.old_disks.get(name) {
                    s.disk.devices.push(disk_delta(name, a, b, interval_ms));
                }
            }
            self.old_disks = current;
        }
        if c.linux.pressure {
            let mut out = Pressure::default();
            for name in ["cpu", "memory", "io"] {
                if let Ok(raw) = std::fs::read_to_string(self.root.join("pressure").join(name)) {
                    let current = pressure(&raw);
                    let value = self
                        .old_pressure
                        .get(name)
                        .map(|a| pressure_delta(a, &current, interval_ms));
                    match name {
                        "cpu" => out.cpu = value,
                        "memory" => out.memory = value,
                        _ => out.io = value,
                    }
                    self.old_pressure.insert(name.into(), current);
                }
            }
            s.pressure = Some(out);
        }
        if c.collection.process_cpu || c.collection.process_memory || c.collection.process_io {
            let mut current = BTreeMap::new();
            let mut rows = Vec::new();
            let mut io_rows = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&self.root) {
                for entry in entries.flatten().take(65536) {
                    let Some(pid) = entry
                        .file_name()
                        .to_str()
                        .and_then(|v| v.parse::<u32>().ok())
                    else {
                        continue;
                    };
                    let Some(mut value) = std::fs::read_to_string(entry.path().join("stat"))
                        .ok()
                        .and_then(|v| process_stat(&v, self.ticks, self.pages, boot_ms))
                    else {
                        continue;
                    };
                    let old = self
                        .old_processes
                        .get(&pid)
                        .filter(|(start, _, _)| *start == value.start);
                    let io = if c.collection.process_io {
                        std::fs::read_to_string(entry.path().join("io"))
                            .ok()
                            .and_then(|raw| {
                                let m: BTreeMap<_, _> = raw
                                    .lines()
                                    .filter_map(|l| {
                                        let (k, v) = l.split_once(':')?;
                                        Some((k, v.trim().parse::<u64>().ok()?))
                                    })
                                    .collect();
                                Some((*m.get("read_bytes")?, *m.get("write_bytes")?))
                            })
                    } else {
                        None
                    };
                    if let Some(delta) = old
                        .and_then(|(_, ticks, _)| value.ticks.checked_sub(*ticks))
                        .zip(total_delta)
                    {
                        value.process.cpu_pct =
                            (delta.0 as f64 / delta.1 as f64 * 100.0).clamp(0.0, 100.0);
                    }
                    if let Some(((ar, aw), (br, bw))) =
                        old.and_then(|v| v.2).zip(io).filter(|_| interval_ms > 0)
                        && let Some((r, w)) = br.checked_sub(ar).zip(bw.checked_sub(aw))
                    {
                        value.process.io_read_bytes_sec =
                            Some(r as f64 * 1000.0 / interval_ms as f64);
                        value.process.io_write_bytes_sec =
                            Some(w as f64 * 1000.0 / interval_ms as f64);
                        io_rows.push(value.process.clone());
                    }
                    current.insert(pid, (value.start, value.ticks, io));
                    rows.push(value.process);
                }
            }
            self.old_processes = current;
            let top = |rows: &[Process], metric: fn(&Process) -> f64| {
                let mut rows = rows.to_vec();
                rows.sort_unstable_by(|a, b| metric(b).total_cmp(&metric(a)));
                rows.truncate(c.collection.process_limit);
                rows
            };
            if c.collection.process_cpu {
                s.cpu.top = top(&rows, |p| p.cpu_pct);
            }
            if c.collection.process_memory {
                s.memory.top = top(&rows, |p| p.working_set_bytes as f64);
            }
            if c.collection.process_io {
                s.disk.top = top(&io_rows, |p| {
                    p.io_read_bytes_sec.unwrap_or(0.0) + p.io_write_bytes_sec.unwrap_or(0.0)
                });
            }
        }
        for name in ["frames", "dpc", "dpc_events", "boot", "window_response"] {
            s.source_status.insert(
                name.into(),
                SourceStatus {
                    available: false,
                    reason: "linux_platform_source".into(),
                    partial: false,
                },
            );
        }
        for (name, available) in [
            ("cpu_total", s.cpu.usage_pct.is_some()),
            ("memory", s.memory.total_bytes > 0),
            ("disks", !s.disk.devices.is_empty()),
            (
                "pressure",
                s.pressure
                    .as_ref()
                    .is_some_and(|p| p.cpu.is_some() || p.memory.is_some() || p.io.is_some()),
            ),
        ] {
            s.source_status.insert(
                name.into(),
                SourceStatus {
                    available,
                    reason: if available {
                        String::new()
                    } else {
                        "linux_initializing_or_unavailable".into()
                    },
                    partial: false,
                },
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_excludes_double_counted_guest_and_handles_resets() {
        let a = cpu_ticks("cpu 10 0 10 80 0 0 0 0 8 0").unwrap();
        assert_eq!(a.total, 100);
        let b = cpu_ticks("cpu 20 0 20 160 0 0 0 0 16 0").unwrap();
        assert_eq!(cpu_usage(&a, &b), Some(20.0));
        assert!(cpu_usage(&b, &a).is_none());
    }
    #[test]
    fn memory_requires_available_and_converts_kib() {
        let m =
            memory("MemTotal: 1000 kB\nMemAvailable: 250 kB\nSwapTotal: 100 kB\nSwapFree: 20 kB")
                .unwrap();
        assert_eq!(m.used_bytes, 750 * 1024);
        assert_eq!(m.pagefile_used_bytes, Some(80 * 1024));
        assert!(memory("MemTotal: 1000 kB").is_none());
    }
    #[test]
    fn pressure_uses_total_deltas_and_rejects_clock_gaps() {
        let a = pressure("some avg10=1.00 total=100000\nfull avg10=0 total=1000");
        let b = pressure("some avg10=2.00 total=200000\nfull avg10=0 total=26000");
        let p = pressure_delta(&a, &b, 500);
        assert_eq!(p.some_pct, Some(20.0));
        assert_eq!(p.full_pct, Some(5.0));
        assert!(pressure_delta(&b, &a, 500).some_pct.is_none());
        assert!(pressure_delta(&a, &b, 6000).full_pct.is_none());
    }
    #[test]
    fn disk_latency_is_per_completed_operation() {
        let a = DiskTicks::default();
        let b = DiskTicks {
            reads: 2,
            read_ms: 100,
            read_sectors: 2048,
            weighted_ms: 250,
            ..Default::default()
        };
        let d = disk_delta("disk", &a, &b, 500);
        assert_eq!(d.read_latency_ms, Some(50.0));
        assert_eq!(d.queue, Some(0.5));
        assert!(d.write_latency_ms.is_none());
        assert!(disk_delta("disk", &b, &a, 500).read_mb_sec.is_none());
    }
}

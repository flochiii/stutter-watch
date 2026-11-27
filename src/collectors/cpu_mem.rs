use super::pdh::{Query, Readings, Values};
use crate::{
    config::Collection,
    model::{Core, Cpu, Memory, Process},
};
use anyhow::{Result, bail};
use std::{collections::HashMap, mem::size_of, time::Instant};
use windows::Win32::{
    Foundation::*,
    System::{ProcessStatus::*, SystemInformation::*, Threading::*},
};
use windows::core::BOOL;

#[repr(C)]
#[derive(Clone, Copy)]
struct UnicodeString {
    length: u16,
    maximum: u16,
    buffer: *const u16,
}

// SYSTEM_PROCESS_INFORMATION, documented NT layout through the six I/O counters.
// Variable-length SYSTEM_THREAD_INFORMATION entries follow this fixed header.
#[repr(C)]
struct NtProcess {
    next: u32,
    threads: u32,
    private_ws: i64,
    hard_faults: u32,
    thread_highwater: u32,
    cycles: u64,
    created: i64,
    user: i64,
    kernel: i64,
    name: UnicodeString,
    priority: i32,
    pid: usize,
    parent: usize,
    handles: u32,
    session: u32,
    key: usize,
    peak_virtual: usize,
    virtual_size: usize,
    faults: u32,
    peak_ws: usize,
    ws: usize,
    paged_peak: usize,
    paged: usize,
    nonpaged_peak: usize,
    nonpaged: usize,
    pagefile: usize,
    pagefile_peak: usize,
    private_bytes: usize,
    read_ops: i64,
    write_ops: i64,
    other_ops: i64,
    read_bytes: i64,
    write_bytes: i64,
    other_bytes: i64,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        buffer: *mut std::ffi::c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
}

#[derive(Clone, Copy)]
struct Previous {
    created: i64,
    cpu: i64,
    read: i64,
    write: i64,
    faults: u32,
    hard_faults: u32,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NtProcess>() == 256);
    assert!(std::mem::offset_of!(NtProcess, pid) == 0x50);
    assert!(std::mem::offset_of!(NtProcess, ws) == 0x90);
    assert!(std::mem::offset_of!(NtProcess, private_bytes) == 0xc8);
};

pub struct CpuMem {
    buffer: Vec<u64>,
    previous: HashMap<u32, Previous>,
    last: Instant,
    system_times: Option<(u64, u64)>,
    efficiency: HashMap<String, u8>,
    pub processes: Vec<Process>,
}

pub fn register(query: &mut Query, selection: &Collection) {
    for (key, path) in [
        ("core_cpu", r"\Processor Information(*)\% Processor Time"),
        ("core_mhz", r"\Processor Information(*)\Processor Frequency"),
        (
            "core_perf",
            r"\Processor Information(*)\% Processor Performance",
        ),
        ("standby_core", r"\Memory\Standby Cache Core Bytes"),
        (
            "standby_normal",
            r"\Memory\Standby Cache Normal Priority Bytes",
        ),
        ("standby_reserve", r"\Memory\Standby Cache Reserve Bytes"),
        ("cache", r"\Memory\Cache Bytes"),
        ("commit", r"\Memory\Committed Bytes"),
        ("commit_limit", r"\Memory\Commit Limit"),
        ("page_reads", r"\Memory\Page Reads/sec"),
        ("pages_input", r"\Memory\Pages Input/sec"),
    ] {
        if (key.starts_with("core_") && selection.cpu_cores)
            || (!key.starts_with("core_") && selection.memory)
        {
            query.add(key, path);
        }
    }
}

impl CpuMem {
    pub fn new(selection: &Collection) -> Self {
        let mut efficiency = HashMap::new();
        // Query topology once, without changing thread affinity or parking policy.
        if selection.cpu_cores {
            unsafe {
                let mut length = 0;
                let _ = GetSystemCpuSetInformation(None, 0, &mut length, None, None);
                if length > 0 {
                    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
                    if GetSystemCpuSetInformation(
                        Some(buffer.as_mut_ptr().cast()),
                        length,
                        &mut length,
                        None,
                        None,
                    )
                    .as_bool()
                    {
                        let mut offset = 0usize;
                        while offset + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= length as usize {
                            let info = &*buffer
                                .as_ptr()
                                .cast::<u8>()
                                .add(offset)
                                .cast::<SYSTEM_CPU_SET_INFORMATION>();
                            if info.Size == 0 {
                                break;
                            }
                            if info.Type == CpuSetInformation {
                                let cpu = info.Anonymous.CpuSet;
                                efficiency.insert(
                                    format!("{},{}", cpu.Group, cpu.LogicalProcessorIndex),
                                    cpu.EfficiencyClass,
                                );
                            }
                            offset += info.Size as usize;
                        }
                    }
                }
            }
        }
        Self {
            buffer: Vec::new(),
            previous: HashMap::new(),
            last: Instant::now(),
            system_times: None,
            efficiency,
            processes: Vec::new(),
        }
    }

    pub fn collect(
        &mut self,
        values: &Values,
        selection: &Collection,
        boot_active: bool,
    ) -> Result<(Cpu, Memory)> {
        if selection.process_snapshot(boot_active) {
            self.refresh_processes(selection, boot_active)?;
        } else {
            self.processes.clear();
            self.previous.clear();
            self.buffer.clear();
            self.buffer.shrink_to_fit();
        }
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let usage = if selection.cpu_total
            && unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }
                .is_ok()
        {
            let idle = filetime(idle);
            let total = filetime(kernel) + filetime(user);
            let usage = self.system_times.and_then(|(old_idle, old_total)| {
                let delta = total.saturating_sub(old_total);
                (delta > 0)
                    .then(|| (1.0 - idle.saturating_sub(old_idle) as f64 / delta as f64) * 100.0)
            });
            self.system_times = Some((idle, total));
            usage.map(|value| value.clamp(0.0, 100.0))
        } else {
            None
        };
        // Keep every logical processor even when PDH returns an invalid sample for
        // a parked core. Missing readings must stay null, not silently remove a core.
        let mut names: std::collections::BTreeSet<String> =
            self.efficiency.keys().cloned().collect();
        for (name, _) in values.get("core_cpu").into_iter().flatten() {
            if !name.contains("Total") {
                names.insert(name.clone());
            }
        }
        let cores = names
            .into_iter()
            .map(|name| {
                let mhz = values.value("core_mhz", &name);
                let perf = values.value("core_perf", &name);
                Core {
                    efficiency_class: self.efficiency.get(&name).copied(),
                    usage_pct: values.value("core_cpu", &name).map(|v| v.min(100.0)),
                    frequency_mhz: mhz,
                    effective_frequency_mhz: mhz.zip(perf).map(|(m, p)| m * p / 100.0),
                    performance_pct: perf,
                    logical_processor: name,
                }
            })
            .collect();
        let mut memory = Memory::default();
        if selection.memory {
            let mut state = MEMORYSTATUSEX {
                dwLength: size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            unsafe {
                GlobalMemoryStatusEx(&mut state)?;
            }
            memory = Memory {
                total_bytes: state.ullTotalPhys,
                available_bytes: state.ullAvailPhys,
                used_bytes: state.ullTotalPhys.saturating_sub(state.ullAvailPhys),
                standby_bytes: values
                    .value("standby_core", "")
                    .zip(values.value("standby_normal", ""))
                    .zip(values.value("standby_reserve", ""))
                    .map(|((a, b), c)| a + b + c),
                cache_bytes: values.value("cache", ""),
                commit_bytes: values.value("commit", ""),
                commit_limit_bytes: values.value("commit_limit", ""),
                page_reads_sec: values.value("page_reads", ""),
                pages_input_sec: values.value("pages_input", ""),
                ..Default::default()
            };
            let mut pagefile_bytes = 0u64;
            let mut found = false;
            let mut context = (&mut pagefile_bytes, &mut found);
            // SAFETY: callback is synchronous and the context lives until enumeration returns.
            unsafe {
                let _ = EnumPageFilesW(
                    Some(pagefile_callback),
                    (&mut context as *mut (&mut u64, &mut bool)).cast(),
                );
            }
            memory.pagefile_used_bytes = found.then_some(pagefile_bytes);
        }
        if selection.process_memory {
            memory.top = top_n(
                &self.processes,
                |p| p.working_set_bytes as f64,
                selection.process_limit,
            );
        }
        Ok((
            Cpu {
                usage_pct: usage,
                cores,
                top: if selection.process_cpu {
                    top_n(&self.processes, |p| p.cpu_pct, selection.process_limit)
                } else {
                    Vec::new()
                },
            },
            memory,
        ))
    }

    fn refresh_processes(&mut self, selection: &Collection, boot_active: bool) -> Result<()> {
        if self.buffer.is_empty() {
            self.buffer.resize(8192, 0);
        }
        let cpu_enabled = selection.process_cpu || (selection.boot && boot_active);
        let io_enabled = selection.process_io || (selection.boot && boot_active);
        let mut needed = 0;
        loop {
            // SAFETY: naturally aligned reusable buffer, NT reports required bytes on resize.
            let status = unsafe {
                NtQuerySystemInformation(
                    5,
                    self.buffer.as_mut_ptr().cast(),
                    (self.buffer.len() * 8) as u32,
                    &mut needed,
                )
            };
            if status == 0 {
                break;
            }
            if status as u32 != 0xc0000004 || needed > 16 * 1024 * 1024 {
                bail!("NtQuerySystemInformation: 0x{status:08x}");
            }
            self.buffer
                .resize((needed as usize + 64 * 1024).div_ceil(8), 0);
        }
        let now = Instant::now();
        let seconds = now.duration_since(self.last).as_secs_f64().max(0.001);
        self.last = now;
        let cpus = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1) as f64;
        self.processes.clear();
        let mut next_previous = HashMap::with_capacity(self.previous.len() + 16);
        let mut offset = 0usize;
        let base = self.buffer.as_ptr() as usize;
        let end = base + needed as usize;
        while offset + size_of::<NtProcess>() <= needed as usize {
            // SAFETY: bounds checked header and each NextEntryOffset is validated below.
            let p = unsafe {
                &*self
                    .buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<NtProcess>()
            };
            let pid = p.pid as u32;
            if pid != 0 {
                let current = Previous {
                    created: p.created,
                    cpu: if cpu_enabled { p.user + p.kernel } else { 0 },
                    read: if io_enabled { p.read_bytes } else { 0 },
                    write: if io_enabled { p.write_bytes } else { 0 },
                    faults: if selection.process_memory {
                        p.faults
                    } else {
                        0
                    },
                    hard_faults: if selection.process_memory {
                        p.hard_faults
                    } else {
                        0
                    },
                };
                let prior = self
                    .previous
                    .get(&pid)
                    .filter(|old| old.created == p.created);
                let name_ptr = p.name.buffer as usize;
                let name =
                    if name_ptr >= base && name_ptr.saturating_add(p.name.length as usize) <= end {
                        // SAFETY: NT string points within this live buffer, length is bytes.
                        String::from_utf16_lossy(unsafe {
                            std::slice::from_raw_parts(p.name.buffer, p.name.length as usize / 2)
                        })
                    } else {
                        format!("pid-{pid}")
                    };
                let delta = |a: i64, b: i64| a.saturating_sub(b).max(0) as f64 / seconds;
                self.processes.push(Process {
                    pid,
                    parent_pid: p.parent as u32,
                    created_ms: nt_time_ms(p.created),
                    name,
                    cpu_pct: prior
                        .map(|old| delta(current.cpu, old.cpu) / 100_000.0 / cpus)
                        .unwrap_or(0.0)
                        .clamp(0.0, 100.0),
                    threads: if cpu_enabled { p.threads } else { 0 },
                    base_priority: if cpu_enabled { p.priority } else { 0 },
                    working_set_bytes: if selection.process_memory {
                        p.ws as u64
                    } else {
                        0
                    },
                    private_bytes: if selection.process_memory {
                        p.private_bytes as u64
                    } else {
                        0
                    },
                    io_read_bytes_sec: prior
                        .map(|old| delta(current.read, old.read))
                        .unwrap_or(0.0),
                    io_write_bytes_sec: prior
                        .map(|old| delta(current.write, old.write))
                        .unwrap_or(0.0),
                    page_faults_sec: prior
                        .map(|old| current.faults.wrapping_sub(old.faults) as f64 / seconds)
                        .unwrap_or(0.0),
                    hard_page_faults_sec: prior
                        .filter(|_| selection.process_memory)
                        .map(|old| p.hard_faults.wrapping_sub(old.hard_faults) as f64 / seconds),
                });
                next_previous.insert(pid, current);
            }
            if p.next == 0 {
                break;
            }
            if !(p.next as usize).is_multiple_of(8) || (p.next as usize) < size_of::<NtProcess>() {
                bail!("Invalid NT process list offset");
            }
            offset += p.next as usize;
        }
        self.previous = next_previous;
        Ok(())
    }
}

unsafe extern "system" fn pagefile_callback(
    context: *mut std::ffi::c_void,
    info: *mut ENUM_PAGE_FILE_INFORMATION,
    _: windows::core::PCWSTR,
) -> BOOL {
    // SAFETY: EnumPageFiles supplies the documented structures and caller's live context.
    unsafe {
        let (bytes, found) = &mut *context.cast::<(&mut u64, &mut bool)>();
        let mut system = SYSTEM_INFO::default();
        GetSystemInfo(&mut system);
        **bytes += (*info).TotalInUse as u64 * system.dwPageSize as u64;
        **found = true;
    }
    BOOL(1)
}

pub fn filetime(time: FILETIME) -> u64 {
    ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
}
pub fn nt_time_ms(time: i64) -> u64 {
    (time.max(0) as u64 / 10_000).saturating_sub(11_644_473_600_000)
}

pub fn top(processes: &[Process], metric: impl Fn(&Process) -> f64) -> Vec<Process> {
    top_n(processes, metric, 5)
}
pub fn top_n(
    processes: &[Process],
    metric: impl Fn(&Process) -> f64,
    limit: usize,
) -> Vec<Process> {
    let mut refs: Vec<_> = processes.iter().collect();
    refs.sort_unstable_by(|a, b| metric(b).total_cmp(&metric(a)));
    refs.into_iter().take(limit).cloned().collect()
}

pub fn own_usage() -> Result<(u64, u64)> {
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    let (mut created, mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        let process = GetCurrentProcess();
        GetProcessMemoryInfo(
            process,
            &mut memory,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )?;
        GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user)?;
    }
    Ok((
        memory.WorkingSetSize as u64,
        filetime(kernel) + filetime(user),
    ))
}

pub fn own_peak_working_set() -> Result<u64> {
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut memory,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )?;
    }
    Ok(memory.PeakWorkingSetSize as u64)
}

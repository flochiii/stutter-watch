//! Deterministic observations, shared by recording and graph annotations.
use crate::{config::Config, model::Sample};
pub const CPU_BUSY: u16 = 1;
pub const RAM_BUSY: u16 = 2;
pub const RAM_LOW: u16 = 4;
pub const PRESENT: u16 = 8;
pub const WINDOW: u16 = 16;
pub const DISK: u16 = 32;
pub const DPC: u16 = 64;
pub const DRIVER: u16 = 128;
pub const DWM: u16 = 256;
pub const CPU_STALL: u16 = 512;
pub const MEMORY_STALL: u16 = 1024;
pub const IO_STALL: u16 = 2048;
pub const DELAYS: u16 = PRESENT | WINDOW | DWM;

pub fn conclusion_key(flags: u16) -> &'static str {
    if flags & DELAYS != 0 {
        "analysis_delay_detected"
    } else if flags & (RAM_LOW | DISK | DPC | DRIVER | MEMORY_STALL | IO_STALL) != 0 {
        "analysis_resource_alert"
    } else if flags & (CPU_BUSY | RAM_BUSY | CPU_STALL) != 0 {
        "analysis_pressure_only"
    } else {
        "analysis_no_delay"
    }
}

pub fn signals(c: &Config, s: &Sample) -> u16 {
    if s.interactive == Some(false) || s.interval_ms > 3_000 {
        return 0;
    }
    let ram_pct = (s.memory.total_bytes > 0)
        .then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0);
    let low_available = (s.memory.total_bytes / 20).clamp(256 * 1024 * 1024, 1024 * 1024 * 1024);
    let mut flags = 0;
    if let Some(p) = &s.pressure {
        if p.cpu
            .as_ref()
            .and_then(|v| v.some_pct)
            .is_some_and(|v| v >= 10.0)
        {
            flags |= CPU_STALL;
        }
        if p.memory
            .as_ref()
            .and_then(|v| v.full_pct)
            .is_some_and(|v| v >= 5.0)
        {
            flags |= MEMORY_STALL;
        }
        if p.io
            .as_ref()
            .and_then(|v| v.full_pct)
            .is_some_and(|v| v >= 5.0)
        {
            flags |= IO_STALL;
        }
    }
    for (condition, flag) in [
        (s.cpu.usage_pct.is_some_and(|v| v >= 95.0), CPU_BUSY),
        (ram_pct.is_some_and(|v| v >= 90.0), RAM_BUSY),
        (
            ram_pct.is_some_and(|v| v >= 95.0) && s.memory.available_bytes < low_available,
            RAM_LOW,
        ),
        (
            s.frames.present_work_ms.is_some_and(|v| v > c.frame_ms),
            PRESENT,
        ),
        (s.frames.dwm_work_ms.is_some_and(|v| v > c.dwm_ms), DWM),
        (
            (s.foreground.responsive == Some(false) && s.foreground.unresponsive_ms > c.hung_ms)
                || s.foreground.response_peak_ms.is_some_and(|v| v > c.hung_ms)
                || s.foreground
                    .response_events
                    .iter()
                    .any(|e| e.duration_ms > c.hung_ms && s.ts_ms.abs_diff(e.ts_ms) < 3_000),
            WINDOW,
        ),
        (
            s.disk.devices.iter().any(|d| {
                d.read_latency_ms.is_some_and(|v| v > c.disk_latency_ms)
                    || d.write_latency_ms.is_some_and(|v| v > c.disk_latency_ms)
            }),
            DISK,
        ),
        (
            s.dpc.time_pct.is_some_and(|v| v > c.dpc_pct)
                || s.dpc.interrupt_pct.is_some_and(|v| v > c.dpc_pct),
            DPC,
        ),
        (
            s.dpc.spikes.iter().any(|event| {
                event.duration_ms > c.driver_spike_ms
                    && s.ts_ms.abs_diff(event.ts_ms) <= s.interval_ms.max(c.sample_ms).min(3_000)
            }),
            DRIVER,
        ),
    ] {
        if condition {
            flags |= flag;
        }
    }
    flags
}
pub fn reasons(c: &Config, s: &Sample) -> Vec<&'static str> {
    if s.interactive == Some(false) || s.interval_ms > 3_000 {
        return Vec::new();
    }
    let flags = signals(c, s);
    let mut reasons: Vec<_> = [
        (PRESENT, "present_block"),
        (DWM, "dwm_stall"),
        (WINDOW, "ui_hang"),
        (DISK, "disk_latency"),
    ]
    .into_iter()
    .filter_map(|(bit, reason)| (flags & bit != 0).then_some(reason))
    .collect();
    if s.disk
        .devices
        .iter()
        .any(|d| d.queue.is_some_and(|v| v > c.disk_queue))
    {
        reasons.push("disk_queue");
    }
    reasons.extend(
        [
            (DPC, "dpc_interrupt"),
            (DRIVER, "driver_spike"),
            (RAM_LOW, "memory_pressure"),
            (MEMORY_STALL, "memory_stall"),
            (IO_STALL, "io_stall"),
        ]
        .into_iter()
        .filter_map(|(bit, reason)| (flags & bit != 0).then_some(reason)),
    );
    if !s.foreground.fullscreen
        && s.gpu
            .transitions
            .iter()
            .any(|t| t.from == "sleeping" && matches!(t.to.as_str(), "active" | "idle_d0"))
    {
        reasons.push("dgpu_wakeup");
    }
    reasons
}
pub fn severity(flags: u16, series: &[usize]) -> u8 {
    let mask = series.iter().fold(0, |mask, &metric| {
        mask | match metric {
            0 => CPU_BUSY,
            1 => RAM_BUSY | RAM_LOW,
            2 => DISK,
            3 => PRESENT | DWM,
            4 => WINDOW,
            5 | 6 => DPC | DRIVER,
            _ => 0,
        }
    });
    if flags & mask & !(CPU_BUSY | RAM_BUSY) != 0 {
        2
    } else if flags & mask != 0 {
        1
    } else {
        0
    }
}
pub fn reason_key(reason: &str) -> &str {
    match reason {
        "present_block" => "event_present",
        "dwm_stall" => "event_dwm",
        "ui_hang" => "event_window",
        "disk_latency" => "event_disk",
        "disk_queue" => "event_queue",
        "dpc_interrupt" => "event_dpc",
        "driver_spike" => "event_driver",
        "dgpu_wakeup" => "event_gpu",
        "memory_pressure" => "event_memory",
        "memory_stall" => "linux_memory_stall",
        "io_stall" => "linux_io_stall",
        "manual_mark" => "event_manual",
        "self_test" => "event_test",
        "shutdown" => "event_shutdown",
        _ => reason,
    }
}
pub fn reason_flag(reason: &str) -> u16 {
    match reason {
        "present_block" => PRESENT,
        "dwm_stall" => DWM,
        "ui_hang" => WINDOW,
        "disk_latency" => DISK,
        "dpc_interrupt" => DPC,
        "driver_spike" => DRIVER,
        "memory_pressure" => RAM_LOW,
        "memory_stall" => MEMORY_STALL,
        "io_stall" => IO_STALL,
        _ => 0,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn high_load_is_pressure_and_never_a_measured_delay() {
        let c = Config::default();
        let mut s = Sample::default();
        s.cpu.usage_pct = Some(100.0);
        s.memory.total_bytes = 16 * 1024 * 1024 * 1024;
        s.memory.available_bytes = 1024 * 1024 * 1024;
        s.memory.used_bytes = s.memory.total_bytes - s.memory.available_bytes;
        let flags = signals(&c, &s);
        assert_eq!(severity(flags, &[0, 1]), 1);
        assert_eq!(flags & DELAYS, 0);
        assert!(reasons(&c, &s).is_empty());
        s.memory.available_bytes = 128 * 1024 * 1024;
        s.memory.used_bytes = s.memory.total_bytes - s.memory.available_bytes;
        assert!(reasons(&c, &s).contains(&"memory_pressure"));
        assert_eq!(severity(signals(&c, &s), &[1]), 2);
    }
    #[test]
    fn only_measured_work_and_current_responses_mark_delays() {
        let c = Config::default();
        let mut s = Sample::default();
        s.frames.frame_time_ms = Some(5000.0);
        assert_eq!(signals(&c, &s) & DELAYS, 0);
        s.foreground.response_peak_ms = Some(c.hung_ms + 1);
        assert_eq!(severity(signals(&c, &s), &[4]), 2);
        assert_eq!(severity(signals(&c, &s), &[0]), 0);
        s.interactive = Some(false);
        assert_eq!(signals(&c, &s), 0);
        s.interactive = Some(true);
        s.interval_ms = 20_000;
        assert_eq!(signals(&c, &s), 0);
    }
}

use super::{
    dpc::{self, KernelData},
    pdh::wide,
};
use crate::{
    config::Config,
    model::{Frames, Process},
};
use anyhow::{Result, bail};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            Diagnostics::Etw::*,
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            Threading::*,
        },
    },
    core::{GUID, PCWSTR, PWSTR},
};

const DXGI: GUID = GUID::from_u128(0xca11c036_0102_4a2d_a6ad_f03cfed5d3c9);
const DWM: GUID = GUID::from_u128(0x9e9bba3c_2e38_40cb_99f4_9e8281425164);
const DXGK: GUID = GUID::from_u128(0x802ec45a_1e99_4b83_9920_87c98277ba9d);
const WIN32K: GUID = GUID::from_u128(0x8c416c79_d49b_4f01_a467_e56d3aa8234c);

pub fn qpc() -> i64 {
    let mut count = 0;
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
    }
    count
}
pub fn frequency() -> i64 {
    let mut count = 0;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut count);
    }
    count.max(1)
}

#[derive(Default)]
struct Chain {
    source: &'static str,
    last: i64,
    times: VecDeque<(i64, f64)>,
    maximum: Option<f64>,
}
struct Start {
    time: i64,
    chain: u64,
    pid: u32,
}
#[derive(Default)]
struct FrameState {
    targets: HashSet<u32>,
    foreground: u32,
    chains: HashMap<(u32, u64), Chain>,
    starts: HashMap<u32, Start>,
    dwm_last: i64,
    dwm_gap: Option<f64>,
    dwm_start: HashMap<u32, i64>,
    dwm_work: Option<f64>,
    tokens: HashMap<(u64, u64, u64), bool>,
    dropped: u64,
    token_seen: bool,
    present_work: Option<f64>,
}

struct Context {
    frames: Arc<Mutex<FrameState>>,
    kernel: Arc<Mutex<KernelData>>,
    lost: Arc<AtomicU32>,
    freq: i64,
    qpc_origin: i64,
    utc_origin: u64,
    driver_threshold: f64,
    driver_events: bool,
    process_events: bool,
}

struct Session {
    control: CONTROLTRACE_HANDLE,
    consumer: PROCESSTRACE_HANDLE,
    properties: Vec<u64>,
    thread: Option<JoinHandle<()>>,
    _context: Box<Context>,
    kernel: bool,
    result: Arc<AtomicU32>,
    warnings: Vec<String>,
}
impl Session {
    fn start(name: &str, kernel: bool, context: Box<Context>) -> Result<Self> {
        let name_w = wide(name);
        let name_offset = size_of::<EVENT_TRACE_PROPERTIES>();
        let mut properties = vec![0u64; (name_offset + name_w.len() * 2).div_ceil(8)];
        let ptr = properties.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        // SAFETY: aligned buffer includes EVENT_TRACE_PROPERTIES followed by its UTF-16 name.
        unsafe {
            (*ptr).Wnode.BufferSize = (properties.len() * 8) as u32;
            (*ptr).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*ptr).Wnode.ClientContext = 1; // QPC clock for all timestamps including DPC InitialTime.
            (*ptr).Wnode.Guid = GUID::from_u128(
                0x97de5c74_1686_4340_af58_3524e6583000 | (crate::model::now_ms() as u128 & 0xfff),
            );
            (*ptr).BufferSize = 64;
            (*ptr).MinimumBuffers = 2;
            (*ptr).MaximumBuffers = 16;
            (*ptr).FlushTimer = 1;
            (*ptr).LogFileMode = EVENT_TRACE_REAL_TIME_MODE
                | if kernel {
                    EVENT_TRACE_SYSTEM_LOGGER_MODE
                } else {
                    EVENT_TRACE_NO_PER_PROCESSOR_BUFFERING
                };
            if kernel {
                (*ptr).EnableFlags = if context.driver_events {
                    EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT | EVENT_TRACE_FLAG_IMAGE_LOAD
                } else {
                    EVENT_TRACE_FLAG(0)
                } | if context.process_events {
                    EVENT_TRACE_FLAG_PROCESS
                } else {
                    EVENT_TRACE_FLAG(0)
                };
            }
            (*ptr).LoggerNameOffset = name_offset as u32;
            std::ptr::copy_nonoverlapping(
                name_w.as_ptr(),
                properties
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(name_offset)
                    .cast::<u16>(),
                name_w.len(),
            );
        }
        let mut control = CONTROLTRACE_HANDLE::default();
        let status = unsafe { StartTraceW(&mut control, PCWSTR(name_w.as_ptr()), ptr) };
        if status != ERROR_SUCCESS {
            bail!(
                "ETW {name}: {status:?} (administrator or Performance Log Users may be required)"
            );
        }
        let mut session = Self {
            control,
            consumer: PROCESSTRACE_HANDLE::default(),
            properties,
            thread: None,
            _context: context,
            kernel,
            result: Arc::new(AtomicU32::new(u32::MAX)),
            warnings: Vec::new(),
        };
        if !kernel {
            let mut enabled = 0;
            for (guid, keywords, ids) in [
                (DXGI, 0x8000000000000002, &[42, 43, 55, 56][..]),
                (DWM, 0x8000000000000080, &[15, 16][..]),
                (DXGK, 0x4000000008000001, &[184][..]),
                (WIN32K, 0x8000000400001000, &[201, 301][..]),
            ] {
                match session.enable(&guid, keywords, ids) {
                    Ok(()) => enabled += 1,
                    Err(e) => session.warnings.push(format!("{e:#}")),
                }
            }
            if enabled == 0 {
                bail!(
                    "No frame providers enabled: {}",
                    session.warnings.join("; ")
                );
            }
        }
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: PWSTR(name_w.as_ptr() as *mut _),
            Context: (&mut *session._context as *mut Context).cast(),
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        logfile.Anonymous2.EventRecordCallback = Some(callback);
        logfile.BufferCallback = Some(buffer_callback);
        session.consumer = unsafe { OpenTraceW(&mut logfile) };
        if session.consumer.Value == u64::MAX {
            bail!("OpenTraceW: {}", windows::core::Error::from_thread());
        }
        let handle = session.consumer;
        let result = Arc::clone(&session.result);
        session.thread = Some(
            thread::Builder::new()
                .name(if kernel { "etw-kernel" } else { "etw-frames" }.into())
                .stack_size(256 * 1024)
                .spawn(move || {
                    // SAFETY: Session owns the handle/context and joins before freeing either.
                    unsafe {
                        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                        let status = ProcessTrace(&[handle], None, None);
                        result.store(status.0, Ordering::Relaxed);
                    }
                })?,
        );
        Ok(session)
    }
    fn enable(&mut self, provider: &GUID, keywords: u64, ids: &[u16]) -> Result<()> {
        // EVENT_FILTER_EVENT_ID's variable-sized EventId array starts after its 4-byte header.
        let mut filter = vec![0u64; (4 + ids.len() * 2).div_ceil(8)];
        let ptr = filter.as_mut_ptr().cast::<u8>();
        unsafe {
            ptr.write(1);
            ptr.add(2).cast::<u16>().write(ids.len() as u16);
            std::ptr::copy_nonoverlapping(ids.as_ptr(), ptr.add(4).cast::<u16>(), ids.len());
        }
        let mut descriptor = EVENT_FILTER_DESCRIPTOR {
            Ptr: ptr as u64,
            Size: (4 + ids.len() * 2) as u32,
            Type: EVENT_FILTER_TYPE_EVENT_ID,
        };
        let params = ENABLE_TRACE_PARAMETERS {
            Version: 2,
            EnableFilterDesc: &mut descriptor,
            FilterDescCount: 1,
            ..Default::default()
        };
        let status = unsafe {
            EnableTraceEx2(
                self.control,
                provider,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                5,
                keywords,
                0,
                0,
                Some(&params),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("EnableTraceEx2 {provider:?}: {status:?}");
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = ControlTraceW(
                self.control,
                PCWSTR::null(),
                self.properties.as_mut_ptr().cast(),
                EVENT_TRACE_CONTROL_STOP,
            );
            if self.consumer.Value != 0 && self.consumer.Value != u64::MAX {
                let _ = CloseTrace(self.consumer);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct Collector {
    frames: Arc<Mutex<FrameState>>,
    pub kernel: Arc<Mutex<KernelData>>,
    _sessions: Vec<Session>,
    lost: Arc<AtomicU32>,
    freq: i64,
    pub status: String,
    pub kernel_status: String,
    pub warnings: Vec<String>,
}
impl Collector {
    pub fn pause(&mut self) {
        self._sessions.clear();
        self.status = "paused".into();
        self.kernel_status = "paused".into();
    }
    pub fn new(config: &Config) -> Self {
        let frames = Arc::new(Mutex::new(FrameState::default()));
        let kernel = Arc::new(Mutex::new(if config.collection.dpc_events {
            KernelData::new()
        } else {
            KernelData::default()
        }));
        let lost = Arc::new(AtomicU32::new(0));
        let freq = frequency();
        let mut this = Self {
            frames,
            kernel,
            _sessions: Vec::new(),
            lost,
            freq,
            status: "disabled".into(),
            kernel_status: "disabled".into(),
            warnings: Vec::new(),
        };
        let qpc_origin = qpc();
        let utc_origin = crate::model::now_ms();
        for (enabled, is_kernel) in [
            (config.collection.frames, false),
            (config.collection.dpc_events || config.collection.boot, true),
        ] {
            if !enabled {
                continue;
            }
            let context = Box::new(Context {
                frames: Arc::clone(&this.frames),
                kernel: Arc::clone(&this.kernel),
                lost: Arc::clone(&this.lost),
                freq,
                qpc_origin,
                utc_origin,
                driver_threshold: config.driver_spike_ms,
                driver_events: config.collection.dpc_events,
                process_events: config.collection.boot,
            });
            let name = format!(
                "stutter-watch-{}-{}-{}",
                if is_kernel { "kernel" } else { "frames" },
                std::process::id(),
                utc_origin
            );
            match Session::start(&name, is_kernel, context) {
                Ok(session) => {
                    let degraded = !session.warnings.is_empty();
                    this.warnings.extend(session.warnings.iter().cloned());
                    this._sessions.push(session);
                    if is_kernel {
                        this.kernel_status = "active".into();
                    } else {
                        this.status = if degraded {
                            "active (some providers unavailable)"
                        } else {
                            "active"
                        }
                        .into();
                    }
                }
                Err(e) => {
                    let warning = format!(
                        "{} unavailable: {e:#}",
                        if is_kernel { "Kernel ETW" } else { "Frame ETW" }
                    );
                    this.warnings.push(warning.clone());
                    if is_kernel {
                        this.kernel_status = warning;
                    } else {
                        this.status = warning;
                    }
                }
            }
        }
        this
    }
    pub fn collect(&mut self, pid: u32, processes: &[Process]) -> Frames {
        for session in &self._sessions {
            let result = session.result.load(Ordering::Relaxed);
            if result != u32::MAX {
                let status =
                    format!("ETW consumer stopped (status {result}); PDH collection continues");
                if session.kernel {
                    self.kernel_status = status;
                } else {
                    self.status = status;
                }
            }
        }
        let mut state = self.frames.lock().unwrap_or_else(|p| p.into_inner());
        if state.foreground != pid {
            state.foreground = pid;
            state.chains.clear();
            state.starts.clear();
            state.tokens.clear();
            state.token_seen = false;
            state.dropped = 0;
            state.present_work = None;
        }
        let mut targets = HashSet::from([pid]);
        for _ in 0..8 {
            let old = targets.len();
            for p in processes {
                if targets.contains(&p.parent_pid) {
                    targets.insert(p.pid);
                }
            }
            if targets.len() == old {
                break;
            }
        }
        state.targets = targets;
        let cutoff = qpc() - self.freq * 30;
        for chain in state.chains.values_mut() {
            while chain.times.front().is_some_and(|(ts, _)| *ts < cutoff) {
                chain.times.pop_front();
            }
        }
        let best = state
            .chains
            .iter()
            .filter(|(_, chain)| !chain.times.is_empty() && qpc() - chain.last < self.freq * 2)
            .max_by_key(|(_, chain)| (chain.source == "DXGI", chain.times.len()))
            .map(|(key, _)| *key);
        let mut output=Frames{status:self.status.clone(),source:"DXGI successful Present cadence (foreground process family); discarded tokens cover composed flip only".into(),pid,
            events_lost:self.lost.load(Ordering::Relaxed),dwm_max_gap_ms:state.dwm_gap.take(),dwm_work_ms:state.dwm_work.take(),..Default::default()};
        if let Some(key) = best {
            output.pid = key.0;
            let chain = state.chains.get_mut(&key).expect("existing chain");
            output.source = format!(
                "{} Present cadence (foreground process family); composed-flip discard-token coverage only",
                chain.source
            );
            // Do not report stale FPS for an application that stopped presenting.
            if qpc() - chain.last < self.freq * 2 {
                let mut times: Vec<_> = chain.times.iter().map(|(_, t)| *t).collect();
                output.frame_time_ms = times.last().copied();
                output.max_frame_time_ms = chain.maximum.take();
                if !times.is_empty() {
                    output.fps = Some(1000.0 / (times.iter().sum::<f64>() / times.len() as f64));
                }
                times.sort_by(|a, b| b.total_cmp(a));
                output.low_1pct_fps = tail_fps(&times, 0.01);
                output.low_01pct_fps = tail_fps(&times, 0.001);
            } else {
                chain.maximum = None;
            }
        }
        output.dropped_frames = state.token_seen.then_some(state.dropped);
        output.present_work_ms = state.present_work.take();
        for chain in state.chains.values_mut() {
            chain.maximum = None;
        }
        state.dropped = 0;
        output
    }
}

fn tail_fps(times: &[f64], fraction: f64) -> Option<f64> {
    if times.is_empty() {
        return None;
    }
    let n = (times.len() as f64 * fraction).ceil() as usize;
    Some(1000.0 / (times[..n].iter().sum::<f64>() / n as f64))
}

unsafe extern "system" fn buffer_callback(log: *mut EVENT_TRACE_LOGFILEW) -> u32 {
    // SAFETY: ETW supplies the live log and Context originally registered by Session.
    unsafe {
        if let Some(log) = log.as_ref()
            && let Some(context) = log.Context.cast::<Context>().as_ref()
        {
            context.lost.fetch_max(log.EventsLost, Ordering::Relaxed);
        }
    }
    1
}
unsafe extern "system" fn callback(record: *mut EVENT_RECORD) {
    // Never panic across an FFI callback; poisoned state is recovered and payloads bounded.
    let Some(event) = (unsafe { record.as_ref() }) else {
        return;
    };
    let Some(context) = (unsafe { event.UserContext.cast::<Context>().as_ref() }) else {
        return;
    };
    let h = &event.EventHeader;
    let time = h.TimeStamp;
    let id = h.EventDescriptor.Id;
    if h.ProviderId != DXGI && h.ProviderId != DWM && h.ProviderId != DXGK && h.ProviderId != WIN32K
    {
        // Providers are enabled independently; do not decode unselected event groups.
        if !context.driver_events && !context.process_events {
            return;
        }
        dpc::handle_event(
            event,
            &context.kernel,
            context.freq,
            context.qpc_origin,
            context.utc_origin,
            context.driver_threshold,
        );
        return;
    }
    let mut state = context.frames.lock().unwrap_or_else(|p| p.into_inner());
    if h.ProviderId == DWM {
        if h.EventDescriptor.Task == 13 && h.EventDescriptor.Opcode == 1 {
            if state.dwm_last > 0 {
                let gap = (time - state.dwm_last) as f64 * 1000.0 / context.freq as f64;
                if gap > 0.0 {
                    state.dwm_gap = Some(state.dwm_gap.unwrap_or(0.0).max(gap));
                }
            }
            state.dwm_last = time;
            state.dwm_start.insert(h.ThreadId, time);
        } else if h.EventDescriptor.Task == 13
            && h.EventDescriptor.Opcode == 2
            && let Some(start) = state.dwm_start.remove(&h.ThreadId)
        {
            let ms = (time - start) as f64 * 1000.0 / context.freq as f64;
            if ms >= 0.0 {
                state.dwm_work = Some(state.dwm_work.unwrap_or(0.0).max(ms));
            }
        }
        return;
    }
    if h.ProviderId == DXGI && state.targets.contains(&h.ProcessId) {
        if matches!(id, 42 | 55) {
            let Some((chain, flags)) = dxgi_start(event) else {
                return;
            };
            if flags & 1 != 0 {
                return;
            }
            {
                if state.starts.len() > 256 {
                    state.starts.clear();
                }
                state.starts.insert(
                    h.ThreadId,
                    Start {
                        time,
                        chain,
                        pid: h.ProcessId,
                    },
                );
            }
        } else if matches!(id, 43 | 56)
            && let Some(start) = state.starts.remove(&h.ThreadId)
        {
            let Some(result) = dxgi_result(event) else {
                return;
            };
            if (result as i32) < 0 || result == 0x087a0001 {
                return;
            }
            let work = (time - start.time) as f64 * 1000.0 / context.freq as f64;
            if (0.0..60_000.0).contains(&work) {
                state.present_work = Some(state.present_work.unwrap_or(0.0).max(work));
            }
            if state.chains.len() >= 8 && !state.chains.contains_key(&(start.pid, start.chain)) {
                state.chains.clear();
            }
            let chain = state.chains.entry((start.pid, start.chain)).or_default();
            chain.source = "DXGI";
            if chain.last > 0 {
                let ms = (start.time - chain.last) as f64 * 1000.0 / context.freq as f64;
                if ms > 0.0 {
                    chain.times.push_back((time, ms));
                    chain.maximum = Some(chain.maximum.unwrap_or(0.0).max(ms));
                }
                while chain.times.len() > 4096 {
                    chain.times.pop_front();
                }
            }
            chain.last = start.time;
        }
    } else if h.ProviderId == DXGK && id == 184 && state.targets.contains(&h.ProcessId) {
        // Kernel-present fallback also sees non-DXGI APIs (e.g. Vulkan/OpenGL).
        // This still measures submission cadence, not confirmed scan-out.
        if let Some(window) = property_u64(event, "hWindow") {
            let key = (h.ProcessId, window);
            if state.chains.len() >= 8 && !state.chains.contains_key(&key) {
                return;
            }
            let chain = state.chains.entry(key).or_default();
            if chain.source == "DXGI" {
                return;
            }
            chain.source = "DxgKrnl";
            if chain.last > 0 {
                let ms = (time - chain.last) as f64 * 1000.0 / context.freq as f64;
                if ms > 0.0 {
                    chain.times.push_back((time, ms));
                    chain.maximum = Some(chain.maximum.unwrap_or(0.0).max(ms));
                }
            }
            chain.last = time;
            while chain.times.len() > 4096 {
                chain.times.pop_front();
            }
        }
    } else if h.ProviderId == WIN32K && id == 201 && state.targets.contains(&h.ProcessId) {
        if let Some(key) = token_key(event) {
            if state.tokens.len() >= 4096 {
                state.tokens.clear();
            }
            state.tokens.insert(key, false);
            state.token_seen = true;
        }
    } else if h.ProviderId == WIN32K
        && id == 301
        && let Some(key) = token_key(event)
        && state.tokens.contains_key(&key)
    {
        match property_u64(event, "NewState") {
            Some(6) => {
                if state.tokens.remove(&key) == Some(false) {
                    state.dropped += 1;
                }
            }
            Some(3) => {
                state.tokens.insert(key, true);
            }
            Some(5) => {
                state.tokens.remove(&key);
            }
            _ => {}
        }
    }
}

fn token_key(event: &EVENT_RECORD) -> Option<(u64, u64, u64)> {
    Some((
        property_u64(event, "CompositionSurfaceLuid")?,
        property_u64(event, "PresentCount")?,
        property_u64(event, "BindId")?,
    ))
}

fn payload(event: &EVENT_RECORD) -> Option<&[u8]> {
    if event.UserData.is_null() {
        return None;
    }
    // ETW guarantees this borrowed payload is live for the duration of its callback.
    Some(unsafe {
        std::slice::from_raw_parts(event.UserData.cast(), event.UserDataLength as usize)
    })
}

fn dxgi_start(event: &EVENT_RECORD) -> Option<(u64, u64)> {
    // Version 0 schemas were checked against the installed DXGI manifest.
    // Decode the hot path directly; unknown versions use TDH's schema decoder.
    if event.EventHeader.EventDescriptor.Version == 0 {
        let bytes = payload(event)?;
        let pointer = if event.EventHeader.Flags & EVENT_HEADER_FLAG_32_BIT_HEADER as u16 != 0 {
            4
        } else {
            8
        };
        if bytes.len() < pointer + 8 {
            return None;
        }
        let chain = if pointer == 4 {
            u32::from_le_bytes(bytes[..4].try_into().ok()?) as u64
        } else {
            u64::from_le_bytes(bytes[..8].try_into().ok()?)
        };
        let flags = u32::from_le_bytes(bytes[pointer..pointer + 4].try_into().ok()?) as u64;
        Some((chain, flags))
    } else {
        Some((
            property_u64(event, "pIDXGISwapChain")?,
            property_u64(event, "Flags")?,
        ))
    }
}
fn dxgi_result(event: &EVENT_RECORD) -> Option<u32> {
    if event.EventHeader.EventDescriptor.Version == 0 {
        Some(u32::from_le_bytes(
            payload(event)?.get(..4)?.try_into().ok()?,
        ))
    } else {
        property_u64(event, "Result").map(|r| r as u32)
    }
}

pub fn property(event: &EVENT_RECORD, name: &str) -> Option<Vec<u8>> {
    let name = wide(name);
    let descriptor = PROPERTY_DATA_DESCRIPTOR {
        PropertyName: name.as_ptr() as u64,
        ArrayIndex: u32::MAX,
        ..Default::default()
    };
    let mut length = 0;
    unsafe {
        if TdhGetPropertySize(event, None, &[descriptor], &mut length) != 0 || length > 64 * 1024 {
            return None;
        }
    }
    let mut bytes = vec![0u8; length as usize];
    (unsafe { TdhGetProperty(event, None, &[descriptor], &mut bytes) } == 0).then_some(bytes)
}
pub fn property_u64(event: &EVENT_RECORD, name: &str) -> Option<u64> {
    let bytes = property(event, name)?;
    match bytes.len() {
        1 => Some(bytes[0] as u64),
        2 => Some(u16::from_le_bytes(bytes.try_into().ok()?) as u64),
        4 => Some(u32::from_le_bytes(bytes.try_into().ok()?) as u64),
        8 => Some(u64::from_le_bytes(bytes.try_into().ok()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lows_use_mean_of_slowest_frames() {
        assert_eq!(tail_fps(&[], 0.01), None);
        assert_eq!(tail_fps(&[100.0, 50.0, 10.0], 0.5), Some(1000.0 / 75.0));
    }

    #[test]
    fn dxgi_callback_measures_cadence_and_ignores_present_test() {
        let frames = Arc::new(Mutex::new(FrameState::default()));
        frames.lock().unwrap().targets.insert(123);
        let mut context = Context {
            frames: Arc::clone(&frames),
            kernel: Arc::new(Mutex::new(KernelData::default())),
            lost: Arc::new(AtomicU32::new(0)),
            freq: 1000,
            qpc_origin: 0,
            utc_origin: 0,
            driver_threshold: 2.0,
            driver_events: true,
            process_events: true,
        };
        let mut start = Vec::new();
        start.extend_from_slice(&1234u64.to_le_bytes());
        start.extend_from_slice(&0u32.to_le_bytes());
        start.extend_from_slice(&1u32.to_le_bytes());
        let mut stop = 0u32.to_le_bytes();
        let mut event = EVENT_RECORD {
            UserContext: (&mut context as *mut Context).cast(),
            ..Default::default()
        };
        event.EventHeader.ProviderId = DXGI;
        event.EventHeader.ProcessId = 123;
        event.EventHeader.ThreadId = 7;
        for timestamp in [100, 116, 232] {
            event.EventHeader.TimeStamp = timestamp;
            event.EventHeader.EventDescriptor.Id = 42;
            event.UserData = start.as_mut_ptr().cast();
            event.UserDataLength = start.len() as u16;
            unsafe {
                callback(&mut event);
            }
            event.EventHeader.EventDescriptor.Id = 43;
            event.UserData = stop.as_mut_ptr().cast();
            event.UserDataLength = 4;
            unsafe {
                callback(&mut event);
            }
        }
        let state = frames.lock().unwrap();
        let chain = &state.chains[&(123, 1234)];
        assert_eq!(chain.maximum, Some(116.0));
        assert_eq!(chain.times.len(), 2);
        drop(state);
        start[8..12].copy_from_slice(&1u32.to_le_bytes());
        event.EventHeader.EventDescriptor.Id = 42;
        event.UserData = start.as_mut_ptr().cast();
        event.UserDataLength = start.len() as u16;
        unsafe {
            callback(&mut event);
        }
        assert!(frames.lock().unwrap().starts.is_empty());
    }

    #[test]
    fn dxgi_decoder_supports_32bit_and_rejects_short_payloads() {
        let mut bytes = [0u8; 12];
        bytes[..4].copy_from_slice(&0x1234u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        let mut event = EVENT_RECORD {
            UserData: bytes.as_mut_ptr().cast(),
            UserDataLength: 12,
            ..Default::default()
        };
        event.EventHeader.Flags = EVENT_HEADER_FLAG_32_BIT_HEADER as u16;
        assert_eq!(dxgi_start(&event), Some((0x1234, 7)));
        event.UserDataLength = 3;
        assert_eq!(dxgi_start(&event), None);
        assert_eq!(dxgi_result(&event), None);
    }
}

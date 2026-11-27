use anyhow::{Result, bail};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::JoinHandle,
    time::Duration,
};
use windows::{Win32::System::Performance::*, core::PCWSTR};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub type Values = HashMap<&'static str, Vec<(String, f64)>>;

pub trait Readings {
    fn value(&self, key: &str, instance: &str) -> Option<f64>;
}
impl Readings for Values {
    fn value(&self, key: &str, instance: &str) -> Option<f64> {
        self.get(key)?
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(instance))
            .map(|(_, value)| *value)
    }
}

type Definition = (&'static str, String);
enum Reply {
    Ready(Vec<String>),
    Values(std::result::Result<Values, String>),
    Failed(String),
}

struct Worker {
    requests: SyncSender<()>,
    replies: Receiver<Reply>,
    cancelled: Arc<AtomicBool>,
    _thread: JoinHandle<()>,
    ready: bool,
    pending: bool,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // A Windows provider can block inside a native call. Do not join it on
        // the recorder/UI thread. After it returns, cancellation prevents any
        // further polling and the worker closes its own native handles.
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub struct Query {
    definitions: Vec<Definition>,
    worker: Option<Worker>,
    provider_warnings: Vec<String>,
    pub warnings: Vec<String>,
}

impl Query {
    pub fn new() -> Result<Self> {
        Ok(Self {
            definitions: Vec::new(),
            worker: None,
            provider_warnings: Vec::new(),
            warnings: Vec::new(),
        })
    }

    pub fn add(&mut self, key: &'static str, path: &str) {
        self.definitions.push((key, path.to_owned()));
    }

    fn start_with<F>(&mut self, initialize: F) -> Result<()>
    where
        F: FnOnce(Vec<Definition>) -> Result<NativeQuery> + Send + 'static,
    {
        let definitions = self.definitions.clone();
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::sync_channel(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&cancelled);
        let thread = std::thread::Builder::new()
            .name("pdh".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                unsafe {
                    use windows::Win32::System::Threading::*;
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }
                let mut query = match initialize(definitions) {
                    Ok(query) => query,
                    Err(error) => {
                        let _ = outgoing.send(Reply::Failed(format!("PDH unavailable: {error:#}")));
                        crate::startup::phase("pdh.unavailable");
                        return;
                    }
                };
                if stopped.load(Ordering::Relaxed)
                    || outgoing.send(Reply::Ready(query.warnings.clone())).is_err()
                {
                    return;
                }
                crate::startup::phase("pdh.ready");
                while incoming.recv().is_ok() {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let values = query.collect().map_err(|error| format!("PDH: {error:#}"));
                    if outgoing.send(Reply::Values(values)).is_err() {
                        break;
                    }
                }
            })?;
        self.worker = Some(Worker {
            requests,
            replies,
            cancelled,
            _thread: thread,
            ready: false,
            pending: false,
        });
        self.warnings =
            vec!["PDH counters are initializing; dependent measurements are unavailable".into()];
        Ok(())
    }

    pub fn collect(&mut self) -> Result<Values> {
        if self.definitions.is_empty() {
            return Ok(Values::new());
        }
        if self.worker.is_none() {
            self.start_with(|definitions| {
                crate::startup::phase("pdh.opening");
                let mut query = NativeQuery::new()?;
                for (key, path) in definitions {
                    crate::startup::phase(&format!("pdh.registering.{key}"));
                    query.add(key, &path);
                }
                crate::startup::phase("pdh.priming");
                // Prime rate counters here: a cold provider must not block the
                // tray, direct CPU/RAM sampling, or the recorder's first writes.
                let _ = query.collect();
                Ok(query)
            })?;
        }
        let worker = self.worker.as_mut().expect("started PDH worker");
        loop {
            match worker.replies.try_recv() {
                Ok(Reply::Ready(warnings)) => {
                    self.provider_warnings = warnings;
                    self.warnings.clone_from(&self.provider_warnings);
                    worker.ready = true;
                }
                Ok(Reply::Values(_)) => {
                    // This request exceeded the previous sample's deadline.
                    // Discard it instead of assigning stale counters to now.
                    worker.pending = false;
                }
                Ok(Reply::Failed(error)) => {
                    self.warnings = vec![error];
                    worker.ready = false;
                    return Ok(Values::new());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self
                        .warnings
                        .iter()
                        .any(|w| w.starts_with("PDH unavailable"))
                    {
                        self.warnings = vec!["PDH unavailable: provider worker stopped".into()];
                    }
                    return Ok(Values::new());
                }
            }
        }
        if !worker.ready || worker.pending {
            return Ok(Values::new());
        }
        worker.requests.try_send(())?;
        worker.pending = true;
        match worker.replies.recv_timeout(Duration::from_millis(50)) {
            Ok(Reply::Values(Ok(values))) => {
                worker.pending = false;
                self.warnings.clone_from(&self.provider_warnings);
                Ok(values)
            }
            Ok(Reply::Values(Err(error))) => {
                worker.pending = false;
                self.warnings.clone_from(&self.provider_warnings);
                self.warnings.push(error);
                Ok(Values::new())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.warnings.clone_from(&self.provider_warnings);
                self.warnings
                    .push("PDH collection exceeded 50 ms; measurements omitted".into());
                Ok(Values::new())
            }
            _ => {
                self.warnings = vec!["PDH unavailable: provider worker stopped".into()];
                Ok(Values::new())
            }
        }
    }

    #[cfg(test)]
    pub fn registered_keys(&self) -> Vec<&'static str> {
        self.definitions.iter().map(|(key, _)| *key).collect()
    }
}

// Native query and counter handles are created, used, and closed on one worker.
// No unsafe Send/Sync implementation or cross-thread native handle access.
struct NativeQuery {
    handle: PDH_HQUERY,
    counters: Vec<(&'static str, PDH_HCOUNTER)>,
    buffer: Vec<u64>,
    pub warnings: Vec<String>,
}

impl NativeQuery {
    pub fn new() -> Result<Self> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: PDH writes one query handle to a valid out pointer.
        let status = unsafe { PdhOpenQueryW(None, 0, &mut handle) };
        if status != 0 {
            bail!("PdhOpenQueryW: 0x{status:08x}");
        }
        Ok(Self {
            handle,
            counters: Vec::new(),
            buffer: Vec::new(),
            warnings: Vec::new(),
        })
    }

    pub fn add(&mut self, key: &'static str, path: &str) {
        let path_w = wide(path);
        let mut counter = PDH_HCOUNTER::default();
        // English counters are localized by PDH, so Ukrainian/Russian Windows works too.
        // SAFETY: the query and NUL-terminated path live throughout this call.
        let status =
            unsafe { PdhAddEnglishCounterW(self.handle, PCWSTR(path_w.as_ptr()), 0, &mut counter) };
        if status == 0 {
            self.counters.push((key, counter));
        } else {
            self.warnings
                .push(format!("PDH unavailable: {path} (0x{status:08x})"));
        }
    }

    pub fn collect(&mut self) -> Result<Values> {
        if self.counters.is_empty() {
            return Ok(Values::new());
        }
        // SAFETY: this object owns the live query until Drop.
        let status = unsafe { PdhCollectQueryData(self.handle) };
        if status != 0 {
            bail!("PdhCollectQueryData: 0x{status:08x}");
        }
        let mut values = Values::with_capacity(self.counters.len());
        for &(key, handle) in &self.counters {
            let mut bytes = 0;
            let mut count = 0;
            // SAFETY: a null buffer requests the required size.
            unsafe {
                PdhGetFormattedCounterArrayW(
                    handle,
                    PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000),
                    &mut bytes,
                    &mut count,
                    None,
                );
            }
            if bytes == 0 || bytes > 8 * 1024 * 1024 {
                continue;
            }
            self.buffer.resize((bytes as usize).div_ceil(8), 0);
            let ptr = self
                .buffer
                .as_mut_ptr()
                .cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            // SAFETY: u64 storage has sufficient size/alignment; PDH initializes count entries.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    handle,
                    PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000),
                    &mut bytes,
                    &mut count,
                    Some(ptr),
                )
            };
            if status != 0 {
                continue;
            }
            let mut items = Vec::with_capacity(count as usize);
            // SAFETY: count is returned on success and contained in the owned buffer.
            for item in unsafe { std::slice::from_raw_parts(ptr, count as usize) } {
                if item.FmtValue.CStatus > 1 {
                    continue;
                }
                // SAFETY: requested PDH_FMT_DOUBLE, and the name belongs to the live PDH output buffer.
                let (number, name) = unsafe {
                    (
                        item.FmtValue.Anonymous.doubleValue,
                        item.szName.to_string().unwrap_or_default(),
                    )
                };
                if number.is_finite() {
                    items.push((name, number.max(0.0)));
                }
            }
            values.insert(key, items);
        }
        Ok(values)
    }
}

impl Drop for NativeQuery {
    fn drop(&mut self) {
        // SAFETY: uniquely owned handle, closed exactly once.
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn blocked_provider_start_does_not_block_sampling_or_shutdown() {
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (closed_tx, closed_rx) = mpsc::sync_channel(1);
        let mut query = Query::new().unwrap();
        query.add("test", r"\Memory\Available Bytes");
        query
            .start_with(move |_| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                let result = NativeQuery::new();
                closed_tx.send(()).unwrap();
                result
            })
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let tick = Instant::now();
        assert!(query.collect().unwrap().is_empty());
        assert!(query.warnings.iter().any(|w| w.contains("initializing")));
        drop(query);
        assert!(tick.elapsed() < Duration::from_millis(500));
        release_tx.send(()).unwrap();
        closed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

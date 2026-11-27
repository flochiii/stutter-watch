//! Small local startup diagnostics, independent of the telemetry writer.
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Instant,
};

static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();

#[derive(Serialize)]
struct Event {
    ts_ms: u64,
    elapsed_ms: u64,
    phase: String,
}

#[derive(Serialize)]
struct Trace {
    pid: u32,
    events: Vec<Event>,
    error: Option<String>,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    started: Option<Instant>,
}

pub fn begin(config: &Path) {
    let mut name = config.file_name().unwrap_or_default().to_os_string();
    name.push(".startup.json");
    let path = config.with_file_name(name);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = TRACE.set(Mutex::new(Trace {
        pid: std::process::id(),
        events: Vec::new(),
        error: None,
        path,
        started: Some(Instant::now()),
    }));
    phase("process.started");
}

pub fn phase(name: &str) {
    record(name, None);
}

pub fn failed(error: &str) {
    record("recorder.failed", Some(error));
}

fn record(name: &str, error: Option<&str>) {
    let Some(trace) = TRACE.get() else { return };
    let mut trace = trace.lock().unwrap_or_else(|p| p.into_inner());
    let elapsed_ms = trace.started.map_or(0, |t| t.elapsed().as_millis() as u64);
    if trace.events.len() >= 64 {
        trace.events.remove(1); // Retain the process start and most recent phases.
    }
    trace.events.push(Event {
        ts_ms: crate::model::now_ms(),
        elapsed_ms,
        phase: name.chars().take(160).collect(),
    });
    if let Some(error) = error {
        trace.error = Some(error.chars().take(2048).collect());
    }
    if let Ok(json) = serde_json::to_vec_pretty(&*trace) {
        let temporary = trace.path.with_extension("json.tmp");
        if std::fs::write(&temporary, json).is_ok() {
            let _ = std::fs::rename(temporary, &trace.path);
        }
    }
}

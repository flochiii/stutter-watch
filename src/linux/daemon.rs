use super::*;
use std::{
    collections::VecDeque,
    os::unix::net::UnixListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use stutter_watch::{detector::Detector, recorder::Recorder};
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}
pub type Shared = Arc<Mutex<Snapshot>>;
pub fn run(config: Config, path: &Path, seconds: Option<u64>, no_tray: bool) -> Result<()> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime(path)?.with_extension("lock"))?;
    lock.try_lock().context("Recorder already running")?;
    let endpoint = socket(path, false)?;
    if endpoint.exists() {
        std::fs::remove_file(&endpoint)?;
    }
    let listener = UnixListener::bind(&endpoint)?;
    listener.set_nonblocking(true)?;
    let shared = Arc::new(Mutex::new(Snapshot {
        observed_seconds: 0.0,
        config: config.clone(),
        latest: None,
        points: Vec::new(),
        recording: Status::default(),
        paused: false,
        integrations: Default::default(),
        error: String::new(),
    }));
    let (tx, rx) =
        std::sync::mpsc::sync_channel::<(String, std::sync::mpsc::Sender<Result<()>>)>(16);
    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    let state = shared.clone();
    let server = std::thread::spawn(move || {
        while alive.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
                    let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
                    let mut bytes = Vec::new();
                    if std::io::Read::by_ref(&mut socket)
                        .take(128)
                        .read_to_end(&mut bytes)
                        .is_err()
                    {
                        continue;
                    }
                    let request = String::from_utf8_lossy(&bytes);
                    let reply = if request == "ping" {
                        serde_json::json!({"ok":true})
                    } else if request.starts_with("snapshot ") {
                        let since = request
                            .trim_start_matches("snapshot ")
                            .trim()
                            .parse::<u64>()
                            .unwrap_or(0);
                        let mut s = state.lock().unwrap_or_else(|p| p.into_inner()).clone();
                        s.points.retain(|p| p.ts > since);
                        serde_json::to_value(s).unwrap_or_default()
                    } else {
                        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                        match tx.try_send((request.into_owned(), reply_tx)) {
                            Ok(()) => match reply_rx.recv_timeout(Duration::from_secs(3)) {
                                Ok(Ok(())) => serde_json::json!({"ok":true}),
                                Ok(Err(e)) => serde_json::json!({"error":e.to_string()}),
                                _ => serde_json::json!({"error":"Recorder control timeout"}),
                            },
                            Err(_) => serde_json::json!({"error":"Recorder control queue is full"}),
                        }
                    };
                    let _ = serde_json::to_writer(&mut socket, &reply);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(15))
                }
                Err(_) => break,
            }
        }
    });
    stutter_watch::startup::begin(path);
    stutter_watch::startup::phase("ipc.ready");
    unsafe {
        libc::signal(
            libc::SIGTERM,
            stop_signal as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGINT, stop_signal as *const () as libc::sighandler_t);
    }
    let mut c = config;
    let store = Recorder::new(&c)?;
    let mut detector = Detector::new(&c);
    let mut sampler = stutter_watch::procfs::Procfs::new(
        "/proc".into(),
        "/sys".into(),
        unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64,
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
    );
    let mut gpu = super::gpu::Collector::new();
    let integration_stop = running.clone();
    if !no_tray && seconds.is_none() {
        super::integrations::start(path.to_path_buf(), shared.clone(), integration_stop);
    }
    let restore = restore(&c);
    let mut history: VecDeque<Point> = restore.unwrap_or_default().into();
    let start = Instant::now();
    let mut observed_start = Instant::now();
    let mut previous = now_ms();
    let mut flush = Instant::now();
    let mut batch = Vec::new();
    let mut count = 0usize;
    let mut paused = false;
    store.append(Record::Session {
        ts_ms: now_ms(),
        version: env!("CARGO_PKG_VERSION").into(),
        warnings: Vec::new(),
        pid: std::process::id(),
        elevated: unsafe { libc::geteuid() } == 0,
        sample_ms: c.sample_ms,
        flush_seconds: c.flush_seconds,
        detailed_logs: c.history.mode == stutter_watch::config::HistoryMode::Lossless,
    });
    stutter_watch::startup::phase("recorder.running");
    while !STOP.load(Ordering::Relaxed)
        && !seconds.is_some_and(|s| start.elapsed() >= Duration::from_secs(s))
    {
        let tick = Instant::now();
        while let Ok((command, reply)) = rx.try_recv() {
            let result = (|| -> Result<()> {
                match command.as_str() {
                    "shutdown" => STOP.store(true, Ordering::Relaxed),
                    "mark" => {
                        if detector.mark("manual")
                            && let Some(p) = &detector.pending
                        {
                            store.incident(p);
                        }
                    }
                    "pause" => {
                        paused = !paused;
                        store.append(Record::Event {
                            ts_ms: now_ms(),
                            name: "pause".into(),
                            detail: paused.to_string(),
                        });
                        history.push_back(Point {
                            ts: now_ms(),
                            values: [None; 9],
                            alerts: 0,
                        });
                        if paused {
                            detector.shutdown_snapshot();
                            detector.clear_history();
                        }
                    }
                    "apply_tray" => {
                        let replacement = Config::load(path)?;
                        c.tray_display = replacement.tray_display;
                        c.tray = replacement.tray;
                        shared.lock().unwrap_or_else(|p| p.into_inner()).config = c.clone();
                    }
                    "show" => open_panel(path)?,
                    "apply" => {
                        let replacement = Config::load(path)?;
                        detector.shutdown_snapshot();
                        while let Some(p) = detector.take_ready(true) {
                            store.incident(&p);
                        }
                        store.samples(std::mem::take(&mut batch));
                        store.barrier()?;
                        c = replacement;
                        store.configure(&c);
                        observed_start = Instant::now();
                        detector = Detector::new(&c);
                    }
                    _ => anyhow::bail!("Unknown recorder command"),
                };
                Ok(())
            })();
            let _ = reply.send(result);
        }
        if !paused {
            let wall = now_ms();
            let interval = wall.saturating_sub(previous);
            previous = wall;
            let mut sample = sampler.collect(&c, start.elapsed().as_millis() as u64, interval);
            sample.interactive = super::integrations::interactive();
            if c.collection.gpu_source() {
                sample.gpu = gpu.collect(&c, &mut sample.source_status);
            }
            sample.collection_ms = tick.elapsed().as_secs_f64() * 1000.0;
            let sample = Arc::new(sample);
            history.push_back(Point::from_sample(&sample, &c));
            while history.len() > 6002
                || history
                    .front()
                    .is_some_and(|p| p.ts < wall.saturating_sub(600_000))
            {
                history.pop_front();
            }
            if detector.push(sample.clone())
                && let Some(p) = &detector.pending
            {
                store.incident(p);
            }
            while let Some(p) = detector.take_ready(false) {
                store.incident(&p);
            }
            batch.push(sample.clone());
            count += 1;
            let mut snapshot = shared.lock().unwrap_or_else(|p| p.into_inner());
            snapshot.latest = Some((*sample).clone());
            snapshot.observed_seconds = observed_start.elapsed().as_secs_f64();
            snapshot.points = history.iter().cloned().collect();
            snapshot.recording = store.status();
            snapshot.config = c.clone();
            snapshot.paused = paused;
        } else {
            shared.lock().unwrap_or_else(|p| p.into_inner()).paused = true;
            previous = now_ms();
        }
        if batch.len() >= 64 || flush.elapsed() >= Duration::from_secs(c.flush_seconds) {
            store.samples(std::mem::take(&mut batch));
            flush = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(c.sample_ms).saturating_sub(tick.elapsed()));
    }
    detector.shutdown_snapshot();
    while let Some(p) = detector.take_ready(true) {
        store.incident(&p);
    }
    store.samples(batch);
    store.barrier()?;
    running.store(false, Ordering::Relaxed);
    let _ = server.join();
    let _ = std::fs::remove_file(endpoint);
    if seconds.is_some() {
        let mut records = 0;
        let mut malformed = 0;
        for (file, _, _) in stutter_watch::storage::managed_files(&c.logs_dir)? {
            malformed += stutter_watch::report::read_records(&file, |_| records += 1)?;
        }
        let status = store.status();
        let receipt = serde_json::json!({"samples":count,"records_validated":records,"malformed_records":malformed,"omitted_jobs":status.omitted_jobs,"recording_error":status.error});
        std::fs::write(
            c.logs_dir.join("self-test-last.json"),
            serde_json::to_vec_pretty(&receipt)?,
        )?;
        println!("{receipt}");
        ensure!(
            count >= 2 && malformed == 0 && status.omitted_jobs == 0 && status.error.is_empty(),
            "Self-test did not complete cleanly"
        );
    }
    Ok(())
}
pub fn restore(c: &Config) -> Result<Vec<Point>> {
    let end = now_ms();
    let cutoff = end.saturating_sub(600_000);
    let mut files = stutter_watch::storage::managed_files(&c.logs_dir)?;
    files.retain(|(p, _, _)| p.parent() == Some(c.logs_dir.as_path()));
    files.sort_by_key(|(_, m, _)| std::cmp::Reverse(*m));
    let mut points = std::collections::BTreeMap::new();
    let mut bytes = 0;
    for (path, _, size) in files.into_iter().take(2048) {
        bytes += size;
        if bytes > 64 * 1024 * 1024 {
            break;
        }
        stutter_watch::report::read_records(&path, |r| match r {
            Record::Sample(s) if s.ts_ms >= cutoff && s.ts_ms <= end => {
                points.insert(s.ts_ms, Point::from_sample(&s, c));
            }
            Record::Summary(s) if s.end_ms >= cutoff => {
                for p in summary_points(&s, c) {
                    points
                        .entry(p.ts)
                        .and_modify(|old: &mut Point| {
                            old.alerts |= p.alerts;
                            for (old, new) in old.values.iter_mut().zip(p.values) {
                                if new.is_some() {
                                    *old = new;
                                }
                            }
                        })
                        .or_insert(p);
                }
            }
            Record::Event { ts_ms, name, .. } if name == "pause" && ts_ms >= cutoff => {
                points.insert(
                    ts_ms,
                    Point {
                        ts: ts_ms,
                        values: [None; 9],
                        alerts: 0,
                    },
                );
            }
            _ => {}
        })?;
        while points.len() > 6002 {
            points.pop_first();
        }
    }
    Ok(points.into_values().collect())
}

fn summary_points(s: &Summary, c: &Config) -> Vec<Point> {
    let mut points = Vec::new();
    for (key, stats) in &s.metrics {
        let index = match key.as_str() {
            "cpu_pct" => 0,
            "ram_used_pct" => 1,
            "pressure.cpu.some_pct" => 4,
            "pressure.memory.full_pct" => 5,
            "pressure.io.full_pct" => 6,
            k if k.starts_with("disk.") && k.ends_with(".read_latency_ms") => 2,
            k if k.starts_with("disk.") && k.ends_with(".write_latency_ms") => 3,
            k if k.starts_with("gpu.") && k.ends_with(".usage_pct") => 7,
            _ => continue,
        };
        for (ts, value) in [(stats.min_ts_ms, stats.min), (stats.max_ts_ms, stats.max)] {
            let mut values = [None; 9];
            values[index] = Some(value);
            let alerts = match index {
                0 if value >= 95.0 => stutter_watch::assessment::CPU_BUSY,
                1 if value >= 90.0 => stutter_watch::assessment::RAM_BUSY,
                2 | 3 if value > c.disk_latency_ms => stutter_watch::assessment::DISK,
                4 if value >= 10.0 => stutter_watch::assessment::CPU_STALL,
                5 if value >= 5.0 => stutter_watch::assessment::MEMORY_STALL,
                6 if value >= 5.0 => stutter_watch::assessment::IO_STALL,
                _ => 0,
            };
            points.push(Point {
                ts: if ts == 0 { s.start_ms } else { ts },
                values,
                alerts,
            });
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_recovery_keeps_short_disk_and_pressure_peaks() {
        let c = Config::default();
        let samples: Vec<_> = (0..10)
            .map(|i| Sample {
                ts_ms: 1_000 + i * 500,
                elapsed_ms: i * 500,
                interval_ms: 500,
                disk: Disk {
                    devices: vec![DiskDevice {
                        name: "test".into(),
                        read_latency_ms: Some(if i == 4 { 125.0 } else { 1.0 }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                pressure: Some(Pressure {
                    io: Some(PressureResource {
                        full_pct: Some(if i == 4 { 8.0 } else { 0.0 }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let summary =
            stutter_watch::storage::summarize(&samples.iter().collect::<Vec<_>>()).unwrap();
        let points = summary_points(&summary, &c);
        assert!(points.iter().any(|p| p.ts == 3_000
            && p.values[2] == Some(125.0)
            && p.alerts & stutter_watch::assessment::DISK != 0));
        assert!(points.iter().any(|p| p.ts == 3_000
            && p.values[6] == Some(8.0)
            && p.alerts & stutter_watch::assessment::IO_STALL != 0));
    }
}

use crate::{
    config::{Config, HistoryMode},
    detector::Pending,
    model::*,
};
use anyhow::{Context, Result, ensure};
use lz4_flex::frame::{BlockSize, FrameEncoder, FrameInfo};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, FileTimes, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub ordinary_bytes: u64,
    pub protected_bytes: u64,
    pub written_bytes: u64,
    pub free_bytes: u64,
}

pub struct Storage {
    pub root: PathBuf,
    rotate_bytes: u64,
    max_bytes: u64,
    retention: Duration,
    reserve_bytes: u64,
    pub written_files: HashSet<PathBuf>,
    pub track_writes: bool,
    mode: HistoryMode,
    summary_seconds: u64,
    protection_cache: HashMap<PathBuf, (SystemTime, u64, bool)>,
    serial: u64,
    pub usage: Usage,
}

impl Storage {
    pub fn new(config: &Config) -> Result<Self> {
        fs::create_dir_all(config.logs_dir.join("incidents"))?;
        fs::create_dir_all(config.logs_dir.join("protected"))?;
        let mut storage = Self {
            root: config.logs_dir.clone(),
            rotate_bytes: config.rotate_mb * 1024 * 1024,
            max_bytes: config.history.ordinary_budget_bytes,
            retention: Duration::from_secs(config.history.retention_hours * 3600),
            reserve_bytes: config.history.reserve_free_bytes,
            written_files: HashSet::new(),
            track_writes: false,
            mode: config.history.mode,
            summary_seconds: config.history.summary_seconds,
            protection_cache: HashMap::new(),
            serial: 0,
            usage: Usage::default(),
        };
        storage.maintain()?;
        storage.usage.free_bytes = free_bytes(&storage.root)?;
        Ok(storage)
    }
    pub fn append(&mut self, record: &Record) -> Result<()> {
        self.write_segment(std::iter::once(serde_json::to_vec(record)))
    }
    pub fn flush_samples(&mut self, samples: &[Arc<Sample>]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        if self.mode == HistoryMode::Lossless {
            self.write_segment(
                samples
                    .iter()
                    .map(|s| serde_json::to_vec(&Record::Sample(Box::new((**s).clone())))),
            )
        } else {
            let mut start = 0;
            while start < samples.len() {
                let end = (start + 1..samples.len())
                    .find(|&i| {
                        samples[i]
                            .elapsed_ms
                            .saturating_sub(samples[start].elapsed_ms)
                            >= self.summary_seconds * 1000
                    })
                    .unwrap_or(samples.len());
                let refs: Vec<_> = samples[start..end].iter().map(Arc::as_ref).collect();
                if let Some(summary) = summarize(&refs) {
                    self.append(&Record::Summary(Box::new(summary)))?;
                }
                start = end;
            }
            Ok(())
        }
    }
    fn check_space(&mut self) -> Result<()> {
        self.usage.free_bytes = free_bytes(&self.root)?;
        ensure!(
            self.usage.free_bytes > self.reserve_bytes + self.rotate_bytes,
            "Recording stopped: free disk space is below the configured reserve; collection continues in bounded RAM"
        );
        Ok(())
    }
    fn write_segment(
        &mut self,
        lines: impl IntoIterator<Item = serde_json::Result<Vec<u8>>>,
    ) -> Result<()> {
        self.check_space()?;
        let limit = self.rotate_bytes.min(1_048_576);
        let mut lines = lines.into_iter().peekable();
        while lines.peek().is_some() {
            self.serial += 1;
            let path = self.root.join(format!(
                "stutter-{}-{}-{:06}.jsonl.lz4",
                now_ms(),
                std::process::id(),
                self.serial
            ));
            let temp = path.with_extension("tmp");
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let info = FrameInfo::new()
                .block_size(BlockSize::Max64KB)
                .content_checksum(true);
            let mut writer = FrameEncoder::with_frame_info(info, file);
            let mut raw = 0;
            for line in lines.by_ref() {
                let bytes = line?;
                ensure!(
                    bytes.len() <= 4 * 1024 * 1024,
                    "Record exceeds bounded decoder limit"
                );
                writer.write_all(&bytes)?;
                writer.write_all(b"\n")?;
                raw += bytes.len() as u64 + 1;
                if raw >= limit {
                    break;
                }
            }
            let file = writer.finish()?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &path)?;
            self.usage.written_bytes += fs::metadata(&path)?.len();
            if self.track_writes {
                self.written_files.insert(path);
            }
        }
        self.maintain()
    }
    pub fn incident(&mut self, pending: &Pending) -> Result<PathBuf> {
        self.check_space()?;
        let directory = if pending.header.protected {
            "protected"
        } else {
            "incidents"
        };
        let path = self
            .root
            .join(directory)
            .join(format!("incident-{}.jsonl.lz4", pending.header.id));
        let tmp = path.with_extension("tmp");
        let old_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let info = FrameInfo::new()
            .block_size(BlockSize::Max64KB)
            .content_checksum(true);
        let mut writer = FrameEncoder::with_frame_info(info, BufWriter::new(File::create(&tmp)?));
        write_record(&mut writer, &Record::Incident(pending.header.clone()))?;
        for sample in &pending.samples {
            write_record(&mut writer, &Record::Sample(Box::new((**sample).clone())))?;
        }
        let mut writer = writer.finish()?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&tmp, &path)?;
        if !pending.header.protected {
            self.usage.written_bytes += fs::metadata(&path)?.len().saturating_sub(old_size);
        }
        if self.track_writes {
            self.written_files.insert(path.clone());
        }
        self.maintain()?;
        Ok(path)
    }
    pub fn maintain(&mut self) -> Result<()> {
        let now = SystemTime::now();
        let mut files = managed_files(&self.root)?;
        files.sort_by_key(|(_, modified, _)| *modified);
        let present: HashSet<_> = files.iter().map(|(p, _, _)| p.clone()).collect();
        self.protection_cache.retain(|p, _| present.contains(p));
        let protected: HashSet<_> = files
            .iter()
            .filter(|(path, modified, bytes)| {
                let cached = self
                    .protection_cache
                    .entry(path.clone())
                    .or_insert_with(|| (*modified, *bytes, is_protected(&self.root, path)));
                if cached.0 != *modified || cached.1 != *bytes {
                    *cached = (*modified, *bytes, is_protected(&self.root, path));
                }
                cached.2
            })
            .map(|(path, _, _)| path.clone())
            .collect();
        let mut total: u64 = files
            .iter()
            .filter(|(p, _, _)| !protected.contains(p))
            .map(|(_, _, s)| s)
            .sum();
        for (path, modified, size) in &files {
            if protected.contains(path) {
                continue;
            }
            if now.duration_since(*modified).unwrap_or_default() > self.retention
                || total > self.max_bytes
            {
                fs::remove_file(path)
                    .with_context(|| format!("Cannot rotate {}", path.display()))?;
                total = total.saturating_sub(*size);
            }
        }
        self.usage.ordinary_bytes = total;
        self.usage.protected_bytes = files
            .iter()
            .filter(|(p, _, _)| protected.contains(p))
            .map(|(_, _, s)| s)
            .sum();
        Ok(())
    }
}
fn is_protected(root: &Path, path: &Path) -> bool {
    if path.parent() == Some(root.join("protected").as_path()) {
        return true;
    }
    if path.parent() != Some(root.join("incidents").as_path()) {
        return false;
    }
    match incident_header(path) {
        Ok(header) => header.protected || header.triggers.iter().any(|t| t.reason == "manual_mark"),
        _ => true,
    }
}

pub fn decoded_reader(path: &Path) -> Result<Box<dyn Read>> {
    let file = File::open(path)?;
    Ok(if path.extension().is_some_and(|e| e == "lz4") {
        Box::new(lz4_flex::frame::FrameDecoder::new(file))
    } else {
        Box::new(file)
    })
}

pub fn incident_header(path: &Path) -> Result<Incident> {
    let mut header = String::new();
    BufReader::new(decoded_reader(path)?)
        .take(262_144)
        .read_line(&mut header)?;
    match serde_json::from_str::<Record>(&header)? {
        Record::Incident(header) => Ok(header),
        _ => anyhow::bail!("Missing incident header"),
    }
}

#[derive(Default, serde::Serialize)]
pub struct CompressionResult {
    pub files: usize,
    pub original_bytes: u64,
    pub compressed_bytes: u64,
}

/// Offline migration. The caller must hold the recorder's single-instance lock.
/// Verify every decoded byte and preserve modification times before removing a
/// source file. Unrecognized files, symlinks, and existing mismatched archives
/// are never replaced.
pub fn compress_history(root: &Path) -> Result<CompressionResult> {
    let mut result = CompressionResult::default();
    for (path, modified, size) in managed_files(root)? {
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let target = path.with_extension("jsonl.lz4");
        if !target.exists() {
            let temporary = path.with_extension(format!("jsonl.lz4.{}.tmp", std::process::id()));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let info = FrameInfo::new()
                .block_size(BlockSize::Max64KB)
                .content_checksum(true);
            let mut writer = FrameEncoder::with_frame_info(info, BufWriter::new(file));
            std::io::copy(&mut File::open(&path)?, &mut writer)?;
            let mut writer = writer.finish()?;
            writer.flush()?;
            writer
                .get_ref()
                .set_times(FileTimes::new().set_modified(modified))?;
            writer.get_ref().sync_all()?;
            drop(writer);
            ensure_same_bytes(&path, &temporary)?;
            ensure_unchanged(&path, modified, size)?;
            fs::rename(&temporary, &target)?;
        }
        ensure_same_bytes(&path, &target)?;
        ensure_unchanged(&path, modified, size)?;
        let compressed = fs::metadata(&target)?.len();
        fs::remove_file(&path)?;
        result.files += 1;
        result.original_bytes += size;
        result.compressed_bytes += compressed;
    }
    Ok(result)
}

fn ensure_unchanged(path: &Path, modified: SystemTime, size: u64) -> Result<()> {
    let current = fs::metadata(path)?;
    ensure!(
        current.modified()? == modified && current.len() == size,
        "Recording changed during compression"
    );
    Ok(())
}

fn ensure_same_bytes(source: &Path, compressed: &Path) -> Result<()> {
    let mut original = BufReader::new(File::open(source)?);
    // Temporary archives do not have the final .lz4 extension yet.
    let mut decoded = BufReader::new(lz4_flex::frame::FrameDecoder::new(File::open(compressed)?));
    loop {
        let left = original.fill_buf()?;
        let right = decoded.fill_buf()?;
        ensure!(
            !left.is_empty() || right.is_empty(),
            "Compressed recording has extra data"
        );
        ensure!(
            !right.is_empty() || left.is_empty(),
            "Compressed recording is truncated"
        );
        if left.is_empty() {
            break;
        }
        let count = left.len().min(right.len());
        ensure!(
            left[..count] == right[..count],
            "Compressed recording does not match original bytes"
        );
        original.consume(count);
        decoded.consume(count);
    }
    Ok(())
}

#[cfg(windows)]
pub fn free_bytes(path: &Path) -> Result<u64> {
    let absolute = path.canonicalize()?;
    let wide: Vec<u16> = absolute
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut available = 0;
    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::PCWSTR(wide.as_ptr()),
            Some(&mut available),
            None,
            None,
        )?;
    }
    Ok(available)
}

#[cfg(target_os = "linux")]
pub fn free_bytes(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(name.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let info = unsafe { info.assume_init() };
    Ok(info.f_bavail.saturating_mul(info.f_frsize))
}

pub fn managed_files(root: &Path) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
    let mut result = Vec::new();
    for dir in [
        root.to_path_buf(),
        root.join("incidents"),
        root.join("protected"),
    ] {
        if !dir.exists() {
            continue;
        }
        ensure!(
            !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
            "Log directory must not be a symlink: {}",
            dir.display()
        );
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_file()
                && !meta.file_type().is_symlink()
                && (name.ends_with(".jsonl") || name.ends_with(".jsonl.lz4"))
                && (name.starts_with("stutter-")
                    || name.starts_with("incident-")
                    || (name.starts_with("mangohud-")
                        && entry.path().parent() == Some(root.join("protected").as_path())))
            {
                result.push((entry.path(), meta.modified()?, meta.len()));
            }
        }
    }
    Ok(result)
}

fn write_record(writer: &mut impl Write, record: &Record) -> Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}

pub fn summarize(samples: &[&Sample]) -> Option<Summary> {
    let first = samples.first()?;
    let last = samples.last()?;
    let mut series: BTreeMap<String, Vec<(u64, f64)>> = BTreeMap::new();
    let mut add = |ts: u64, key: String, value: Option<f64>| {
        if let Some(value) = value.filter(|v| v.is_finite()) {
            series.entry(key).or_default().push((ts, value));
        }
    };
    for s in samples {
        for (key, value) in [
            ("cpu_pct", s.cpu.usage_pct),
            (
                "ram_used_pct",
                (s.memory.total_bytes > 0)
                    .then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0),
            ),
            (
                "window_response_ms",
                s.foreground.response_peak_ms.map(|v| v as f64),
            ),
            (
                "ram_used_bytes",
                (s.memory.total_bytes > 0).then_some(s.memory.used_bytes as f64),
            ),
            (
                "ram_available_bytes",
                (s.memory.total_bytes > 0).then_some(s.memory.available_bytes as f64),
            ),
            ("standby_bytes", s.memory.standby_bytes),
            ("cache_bytes", s.memory.cache_bytes),
            ("commit_bytes", s.memory.commit_bytes),
            ("commit_limit_bytes", s.memory.commit_limit_bytes),
            (
                "pagefile_used_bytes",
                s.memory.pagefile_used_bytes.map(|v| v as f64),
            ),
            ("page_reads_sec", s.memory.page_reads_sec),
            ("pages_input_sec", s.memory.pages_input_sec),
            ("dpc_pct", s.dpc.time_pct),
            ("interrupt_pct", s.dpc.interrupt_pct),
            ("dpcs_queued_sec", s.dpc.queued_sec),
            ("fps", s.frames.fps),
            ("frame_time_ms", s.frames.max_frame_time_ms),
            ("present_work_ms", s.frames.present_work_ms),
            ("dwm_gap_ms", s.frames.dwm_max_gap_ms),
            ("dwm_work_ms", s.frames.dwm_work_ms),
            ("collection_ms", Some(s.collection_ms)),
            ("sample_interval_ms", Some(s.interval_ms as f64)),
        ] {
            add(s.ts_ms, key.into(), value);
        }
        if let Some(p) = &s.pressure {
            for (name, resource) in [("cpu", &p.cpu), ("memory", &p.memory), ("io", &p.io)] {
                if let Some(resource) = resource {
                    add(
                        s.ts_ms,
                        format!("pressure.{name}.some_pct"),
                        resource.some_pct,
                    );
                    add(
                        s.ts_ms,
                        format!("pressure.{name}.full_pct"),
                        resource.full_pct,
                    );
                }
            }
        }
        for core in &s.cpu.cores {
            for (key, value) in [
                ("cpu_pct", core.usage_pct),
                ("mhz", core.frequency_mhz),
                ("effective_mhz", core.effective_frequency_mhz),
                ("performance_pct", core.performance_pct),
            ] {
                add(
                    s.ts_ms,
                    format!("core.{}.{key}", core.logical_processor),
                    value,
                );
            }
        }
        for disk in &s.disk.devices {
            for (key, value) in [
                ("read_mb_sec", disk.read_mb_sec),
                ("write_mb_sec", disk.write_mb_sec),
                ("read_latency_ms", disk.read_latency_ms),
                ("write_latency_ms", disk.write_latency_ms),
                ("queue", disk.queue),
            ] {
                add(s.ts_ms, format!("disk.{}.{key}", disk.name), value);
            }
        }
        for adapter in &s.gpu.adapters {
            add(
                s.ts_ms,
                format!("gpu.{}.usage_pct", adapter.luid),
                adapter.usage_pct,
            );
            let mut engine_totals: BTreeMap<&str, f64> = BTreeMap::new();
            for p in &adapter.processes {
                *engine_totals.entry(&p.engine).or_default() += p.usage_pct;
            }
            for (engine, value) in engine_totals {
                add(
                    s.ts_ms,
                    format!("gpu.{}.engine_group_pct.{engine}", adapter.luid),
                    Some(value.min(100.0)),
                );
            }
            if let Some(nvml) = adapter.nvml.as_ref().or(adapter.sensors.as_ref()) {
                for (key, value) in [
                    ("power_w", nvml.power_w),
                    (
                        "vram_pct",
                        nvml.vram_used_bytes
                            .zip(nvml.vram_total_bytes)
                            .filter(|(_, total)| *total > 0)
                            .map(|(used, total)| used as f64 / total as f64 * 100.0),
                    ),
                    ("temperature_c", nvml.temperature_c.map(|v| v as f64)),
                    ("core_mhz", nvml.core_mhz.map(|v| v as f64)),
                    ("memory_mhz", nvml.memory_mhz.map(|v| v as f64)),
                    ("vram_used_bytes", nvml.vram_used_bytes.map(|v| v as f64)),
                ] {
                    add(s.ts_ms, format!("gpu.{}.{key}", adapter.luid), value);
                }
            }
        }
    }
    let metrics = series
        .into_iter()
        .map(|(key, mut values)| {
            values.sort_by(|a, b| a.1.total_cmp(&b.1));
            let count = values.len();
            let stats = Stats {
                count,
                min_ts_ms: values[0].0,
                max_ts_ms: values[count - 1].0,
                min: values[0].1,
                avg: values.iter().map(|v| v.1).sum::<f64>() / count as f64,
                p95: values[((count as f64 * 0.95).ceil() as usize).saturating_sub(1)].1,
                max: values[count - 1].1,
            };
            (key, stats)
        })
        .collect();
    let combine = |select: fn(&Sample) -> &[Process], metric: fn(&Process) -> f64| {
        let mut map: BTreeMap<(u32, u64), Process> = BTreeMap::new();
        for p in samples.iter().flat_map(|s| select(s)) {
            let entry = map
                .entry((p.pid, p.created_ms))
                .or_insert_with(|| p.clone());
            if metric(p) > metric(entry) {
                *entry = p.clone();
            }
        }
        crate::model::top_processes(&map.into_values().collect::<Vec<_>>(), metric)
    };
    let mut boot = last.boot.clone();
    let mut processes = BTreeMap::new();
    for s in samples {
        for p in &s.boot.processes {
            processes.insert((p.pid, p.created_ms), p.clone());
        }
    }
    boot.processes = processes.into_values().collect();
    Some(Summary {
        start_ms: first.ts_ms,
        end_ms: last.ts_ms,
        samples: samples.len(),
        metrics,
        top_cpu: combine(|s| &s.cpu.top, |p| p.cpu_pct),
        top_memory: combine(|s| &s.memory.top, |p| p.working_set_bytes as f64),
        top_io: combine(
            |s| &s.disk.top,
            |p| p.io_read_bytes_sec.unwrap_or(0.0) + p.io_write_bytes_sec.unwrap_or(0.0),
        ),
        gpu_transitions: samples
            .iter()
            .flat_map(|s| s.gpu.transitions.clone())
            .collect(),
        boot,
        capabilities: vec![
            last.frames.status.clone(),
            last.dpc.kernel_status.clone(),
            last.gpu.nvml_status.clone(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_compression_verifies_exact_bytes_and_preserves_age_and_protection() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("incidents")).unwrap();
        let source = directory.path().join("incidents/incident-legacy.jsonl");
        let record = Record::Incident(Incident {
            id: "legacy".into(),
            triggers: vec![Trigger {
                reason: "manual_mark".into(),
                ts_ms: 1,
                foreground: Foreground::default(),
            }],
            ..Default::default()
        });
        let data = serde_json::to_string(&record).unwrap() + "\n  \n";
        fs::write(&source, &data).unwrap();
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(10 * 86400))
            .unwrap();
        let modified = fs::metadata(&source).unwrap().modified().unwrap();
        let result = compress_history(directory.path()).unwrap();
        assert_eq!(result.files, 1);
        assert!(!source.exists());
        let target = source.with_extension("jsonl.lz4");
        let mut restored = String::new();
        decoded_reader(&target)
            .unwrap()
            .read_to_string(&mut restored)
            .unwrap();
        assert_eq!(restored, data);
        assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), modified);
        assert!(is_protected(directory.path(), &target));
        assert_eq!(incident_header(&target).unwrap().id, "legacy");
        assert_eq!(compress_history(directory.path()).unwrap().files, 0);
    }
    #[test]
    fn compression_never_removes_original_when_existing_archive_does_not_match() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("stutter-old.jsonl");
        let target = source.with_extension("jsonl.lz4");
        fs::write(&source, b"original recording\n").unwrap();
        fs::write(&target, b"not a valid compressed recording").unwrap();
        assert!(compress_history(directory.path()).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original recording\n");
        assert_eq!(
            fs::read(&target).unwrap(),
            b"not a valid compressed recording"
        );
    }
    fn records(path: &Path) -> Vec<Record> {
        let mut rows = Vec::new();
        assert_eq!(
            crate::report::read_records(path, |r| rows.push(r)).unwrap(),
            0
        );
        rows
    }
    fn samples() -> Vec<Arc<Sample>> {
        (0..120)
            .map(|i| {
                Arc::new(Sample {
                    ts_ms: 1_000_000 + i * 500,
                    elapsed_ms: i * 500,
                    cpu: Cpu {
                        usage_pct: Some(if i == 73 {
                            250.1234567890123
                        } else {
                            f64::from_bits(0x3fd5555555555555)
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                })
            })
            .collect()
    }
    #[test]
    fn legacy_manual_dumps_are_also_protected() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("incidents")).unwrap();
        let path = temp.path().join("incidents/incident-old.jsonl");
        let header = Record::Incident(Incident {
            triggers: vec![Trigger {
                ts_ms: 1,
                reason: "manual_mark".into(),
                foreground: Foreground::default(),
            }],
            ..Default::default()
        });
        fs::write(&path, serde_json::to_string(&header).unwrap() + "\n").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(365 * 86400))
            .unwrap();
        let mut store = Storage::new(&Config {
            logs_dir: temp.path().into(),
            ..Default::default()
        })
        .unwrap();
        store.max_bytes = 1;
        store.maintain().unwrap();
        assert!(path.exists());
        assert!(store.usage.protected_bytes > 0);
        assert_eq!(store.usage.ordinary_bytes, 0);
    }
    #[test]
    fn lossless_round_trip_preserves_every_selected_value_and_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Storage::new(&Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        })
        .unwrap();
        let original = samples();
        store.flush_samples(&original).unwrap();
        let decoded: Vec<_> = managed_files(tmp.path())
            .unwrap()
            .iter()
            .flat_map(|(p, _, _)| records(p))
            .collect();
        assert_eq!(decoded.len(), original.len());
        for (record, sample) in decoded.into_iter().zip(original) {
            let Record::Sample(decoded) = record else {
                panic!()
            };
            assert_eq!(decoded.ts_ms, sample.ts_ms);
            assert_eq!(
                decoded.cpu.usage_pct.unwrap().to_bits(),
                sample.cpu.usage_pct.unwrap().to_bits()
            );
        }
    }
    #[test]
    fn summary_mode_keeps_short_peaks_counts_and_peak_time() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        };
        config.history.mode = HistoryMode::Summary;
        let mut store = Storage::new(&config).unwrap();
        store.flush_samples(&samples()).unwrap();
        let rows: Vec<_> = managed_files(tmp.path())
            .unwrap()
            .iter()
            .flat_map(|(p, _, _)| records(p))
            .collect();
        assert_eq!(rows.len(), 12);
        let mut counts = 0;
        let mut peak = 0.0f64;
        let mut peak_ts = 0;
        for row in rows {
            let Record::Summary(s) = row else { panic!() };
            let cpu = &s.metrics["cpu_pct"];
            counts += cpu.count;
            if cpu.max > peak {
                peak = cpu.max;
                peak_ts = cpu.max_ts_ms;
            }
            assert!(!s.metrics.contains_key("ram_used_bytes"));
        }
        assert_eq!(counts, 120);
        assert_eq!(peak, 250.1234567890123);
        assert_eq!(peak_ts, 1_036_500);
    }
    #[test]
    fn manual_incident_survives_both_size_and_age_rotation() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Storage::new(&Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        })
        .unwrap();
        let mut pending = Pending {
            header: Incident {
                id: "manual".into(),
                protected: true,
                ..Default::default()
            },
            samples: samples(),
            deadline_elapsed_ms: 0,
        };
        let path = store.incident(&pending).unwrap();
        pending.header.complete = true;
        store.incident(&pending).unwrap();
        assert!(matches!(&records(&path)[0],Record::Incident(h) if h.complete));
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(365 * 86400))
            .unwrap();
        store
            .append(&Record::Event {
                ts_ms: 1,
                name: "test".into(),
                detail: "x".repeat(70),
            })
            .unwrap();
        store.max_bytes = 1;
        store.retention = Duration::ZERO;
        store.maintain().unwrap();
        assert!(path.exists());
        assert_eq!(store.usage.ordinary_bytes, 0);
        assert!(store.usage.protected_bytes > 0);
    }
    #[test]
    fn independent_segments_rotate_and_keep_unrelated_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Storage::new(&Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        })
        .unwrap();
        store.rotate_bytes = 100;
        store.flush_samples(&samples()[..3]).unwrap();
        assert_eq!(managed_files(tmp.path()).unwrap().len(), 3);
        fs::write(tmp.path().join("user.txt"), "keep").unwrap();
        store.max_bytes = 1;
        store.maintain().unwrap();
        assert!(managed_files(tmp.path()).unwrap().is_empty());
        assert!(tmp.path().join("user.txt").exists());
    }
    #[test]
    fn reads_old_jsonl_and_rotates_old_ordinary_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stutter-old.jsonl");
        let mut old = serde_json::to_value(Record::Sample(Box::default())).unwrap();
        old.as_object_mut().unwrap().remove("sources");
        old.as_object_mut().unwrap().remove("interactive");
        fs::write(&path, serde_json::to_string(&old).unwrap() + "\n").unwrap();
        assert_eq!(records(&path).len(), 1);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(8 * 86400))
            .unwrap();
        let _ = Storage::new(&Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        })
        .unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn full_disk_reserve_stops_writing_without_deleting_protected_data() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = Config {
            logs_dir: tmp.path().into(),
            ..Default::default()
        };
        c.history.reserve_free_bytes = u64::MAX / 2;
        let mut store = Storage::new(&c).unwrap();
        assert!(store.flush_samples(&samples()).is_err());
        assert!(managed_files(tmp.path()).unwrap().is_empty());
    }
}

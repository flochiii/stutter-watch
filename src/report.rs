use crate::{config::Config, model::*, storage::managed_files};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

#[derive(Default)]
struct Analysis {
    incidents: usize,
    manual: usize,
    tests: usize,
    incomplete: usize,
    malformed: usize,
    samples: usize,
    reasons: BTreeMap<String, usize>,
    suspects: BTreeMap<String, usize>,
    correlations: BTreeMap<&'static str, usize>,
    drivers: BTreeMap<String, usize>,
    boot: BTreeMap<(u32, u64), BootProcess>,
    timeline: Vec<(u64, String)>,
    warnings: HashSet<String>,
    wakes: HashSet<(u64, String)>,
    stable: Option<u64>,
    seen_manual: HashSet<u64>,
}

pub fn generate(config: &Config, hours: u64, output: Option<&Path>) -> Result<PathBuf> {
    ensure!((1..=24 * 365).contains(&hours), "hours must be 1..8760");
    let end = now_ms();
    let start = end.saturating_sub(hours * 3_600_000);
    let mut analysis = Analysis::default();
    for (path, _, _) in managed_files(&config.logs_dir)? {
        analyze_file(&path, start, end, config, &mut analysis)?;
    }
    let path = output.map(Path::to_path_buf).unwrap_or_else(|| {
        config
            .logs_dir
            .join(format!("report_{}.md", time(end, "%Y-%m-%d")))
    });
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, render(&analysis, start, end, &config.language))?;
    Ok(path)
}
pub fn runtime_status(config: &Config) -> Result<serde_json::Value> {
    let mut files = managed_files(&config.logs_dir)?;
    files.retain(|(p, _, _)| {
        p.parent() == Some(config.logs_dir.as_path()) && p.extension().is_some_and(|e| e == "lz4")
    });
    files.sort_by_key(|(_, m, _)| std::cmp::Reverse(*m));
    let mut samples = 0;
    let mut latest: Option<Box<Sample>> = None;
    let mut elevated = None;
    let mut newest_session = 0;
    let mut malformed = 0;
    for (path, _, _) in files.into_iter().take(8) {
        malformed += read_records(&path, |record| match record {
            Record::Sample(sample) => {
                samples += 1;
                if latest.as_ref().is_none_or(|old| sample.ts_ms > old.ts_ms) {
                    latest = Some(sample);
                }
            }
            Record::Session {
                ts_ms,
                elevated: admin,
                ..
            } if ts_ms >= newest_session => {
                newest_session = ts_ms;
                elevated = Some(admin);
            }
            _ => {}
        })?;
    }
    Ok(
        serde_json::json!({"ram_total_bytes":latest.as_ref().map(|s|s.memory.total_bytes),"ram_used_bytes":latest.as_ref().map(|s|s.memory.used_bytes),"recent_samples":samples,"newest_sample_ms":latest.as_ref().map(|s|s.ts_ms),"elevated":elevated,"malformed_records":malformed,"frame_etw":latest.as_ref().map(|s|&s.frames.status),"kernel_etw":latest.as_ref().map(|s|&s.dpc.kernel_status),"nvml":latest.as_ref().map(|s|&s.gpu.nvml_status)}),
    )
}

pub fn read_records(path: &Path, mut visit: impl FnMut(Record)) -> Result<usize> {
    let input = crate::storage::decoded_reader(path)?;
    let mut reader = BufReader::new(input);
    let mut malformed = 0;
    let mut bytes = Vec::new();
    const LIMIT: u64 = 4 * 1024 * 1024;
    loop {
        bytes.clear();
        let count = reader
            .by_ref()
            .take(LIMIT + 1)
            .read_until(b'\n', &mut bytes)?;
        if count == 0 {
            break;
        }
        if count as u64 > LIMIT {
            while bytes.last() != Some(&b'\n') {
                bytes.clear();
                if reader.by_ref().take(LIMIT).read_until(b'\n', &mut bytes)? == 0 {
                    break;
                }
            }
            malformed += 1;
            continue;
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<Record>(&bytes) {
            Ok(record) => visit(record),
            Err(_) => malformed += 1,
        }
    }
    Ok(malformed)
}

fn analyze_file(
    path: &Path,
    start: u64,
    end: u64,
    config: &Config,
    a: &mut Analysis,
) -> Result<()> {
    let mut header: Option<Incident> = None;
    let mut candidates = HashSet::new();
    let mut correlations = HashSet::new();
    let mut counted = false;
    let malformed = read_records(path, |record| match record {
        Record::Incident(mut incident) => {
            incident
                .triggers
                .retain(|t| t.ts_ms >= start && t.ts_ms <= end);
            if incident.triggers.is_empty() {
                return;
            }
            let auto = incident
                .triggers
                .iter()
                .any(|t| !matches!(t.reason.as_str(), "manual_mark" | "self_test" | "shutdown"));
            a.incidents += usize::from(auto);
            a.incomplete += usize::from(!incident.complete);
            for t in &incident.triggers {
                match t.reason.as_str() {
                    "manual_mark" => {
                        if a.seen_manual.insert(t.ts_ms) {
                            a.manual += 1;
                        }
                    }
                    "self_test" => a.tests += 1,
                    "shutdown" => {}
                    _ => *a.reasons.entry(t.reason.clone()).or_default() += 1,
                }
            }
            let names = incident
                .triggers
                .iter()
                .map(|t| t.reason.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            a.timeline.push((
                incident.triggers[0].ts_ms,
                format!(
                    "{} — {}",
                    names,
                    escape(if config.privacy.window_titles {
                        &incident.triggers[0].foreground.title
                    } else {
                        &incident.triggers[0].foreground.program
                    })
                ),
            ));
            counted = auto || incident.triggers.iter().any(|t| t.reason == "manual_mark");
            header = Some(incident);
        }
        Record::Sample(sample) => {
            if !counted {
                return;
            }
            let Some(h) = &header else {
                return;
            };
            if !h
                .triggers
                .iter()
                .any(|t| t.ts_ms.abs_diff(sample.ts_ms) <= 1000)
            {
                return;
            }
            a.samples += 1;
            for p in &sample.cpu.top {
                if p.cpu_pct > 0.5 {
                    candidates.insert(format!("{} (CPU)", p.name));
                }
            }
            for p in &sample.disk.top {
                if p.io_read_bytes_sec.unwrap_or(0.0) + p.io_write_bytes_sec.unwrap_or(0.0)
                    > 1_048_576.0
                {
                    candidates.insert(format!("{} (I/O)", p.name));
                }
            }
            for adapter in &sample.gpu.adapters {
                for p in &adapter.processes {
                    if p.usage_pct > 10.0 {
                        candidates.insert(format!(
                            "{} ({})",
                            p.name.as_deref().unwrap_or("unknown"),
                            adapter.name
                        ));
                    }
                }
            }
            if sample.memory.page_reads_sec.is_some_and(|v| v > 50.0) {
                correlations.insert("Page Reads >50/s");
            }
            if sample.disk.devices.iter().any(|d| {
                d.read_latency_ms
                    .is_some_and(|v| v > config.disk_latency_ms)
                    || d.write_latency_ms
                        .is_some_and(|v| v > config.disk_latency_ms)
                    || d.queue.is_some_and(|v| v > config.disk_queue)
            }) {
                correlations.insert("Disk latency/queue");
            }
            if sample.dpc.time_pct.is_some_and(|v| v > config.dpc_pct)
                || sample.dpc.interrupt_pct.is_some_and(|v| v > config.dpc_pct)
                || sample.dpc.spikes.iter().any(|s| {
                    s.duration_ms > config.driver_spike_ms
                        && h.triggers.iter().any(|t| t.ts_ms.abs_diff(s.ts_ms) <= 1000)
                })
            {
                correlations.insert("DPC/ISR");
            }
            if sample
                .gpu
                .transitions
                .iter()
                .any(|t| t.from == "sleeping" && matches!(t.to.as_str(), "active" | "idle_d0"))
            {
                correlations.insert("NVIDIA wake transition");
            }
            for spike in sample
                .dpc
                .spikes
                .iter()
                .filter(|s| h.triggers.iter().any(|t| t.ts_ms.abs_diff(s.ts_ms) <= 1000))
            {
                let name = spike
                    .module
                    .clone()
                    .unwrap_or_else(|| format!("unresolved {}", spike.routine))
                    .rsplit('\\')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                *a.drivers.entry(name).or_default() += 1;
            }
        }
        Record::Mark(mark)
            if mark.ts_ms >= start && mark.ts_ms <= end && mark.reason == "manual_mark" =>
        {
            if a.seen_manual.insert(mark.ts_ms) {
                a.manual += 1;
            }
        }
        Record::Summary(summary) if summary.end_ms >= start && summary.start_ms <= end => {
            for warning in &summary.capabilities {
                if !warning.is_empty() {
                    a.warnings.insert(warning.clone());
                }
            }
            if summary.boot.stable_at_ms.is_some() {
                a.stable = summary.boot.stable_at_ms;
            }
            for p in summary.boot.processes {
                let key = (p.pid, p.created_ms);
                let entry = a.boot.entry(key).or_insert_with(|| p.clone());
                entry.peak_cpu_pct = entry.peak_cpu_pct.max(p.peak_cpu_pct);
                entry.peak_io_bytes_sec = entry.peak_io_bytes_sec.max(p.peak_io_bytes_sec);
                entry.first_seen_ms = entry.first_seen_ms.min(p.first_seen_ms);
            }
            for t in summary.gpu_transitions {
                if t.ts_ms >= start
                    && t.ts_ms <= end
                    && t.from == "sleeping"
                    && matches!(t.to.as_str(), "active" | "idle_d0")
                {
                    a.wakes.insert((t.ts_ms, t.adapter));
                }
            }
        }
        Record::Event {
            ts_ms,
            name,
            detail,
        } if ts_ms >= start
            && ts_ms <= end
            && matches!(
                name.as_str(),
                "warning" | "pdh_error" | "cpu_memory_error" | "recording_incomplete"
            ) =>
        {
            a.warnings.insert(format!("{name}: {detail}"));
        }
        Record::Session {
            ts_ms, warnings, ..
        } if ts_ms >= start && ts_ms <= end => {
            a.warnings.extend(warnings);
        }
        _ => {}
    })
    .with_context(|| format!("Read log {}", path.display()))?;
    a.malformed += malformed;
    for name in candidates {
        *a.suspects.entry(name).or_default() += 1;
    }
    for name in correlations {
        *a.correlations.entry(name).or_default() += 1;
    }
    Ok(())
}

fn time(ms: u64, format: &str) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms as i64)
        .unwrap_or_default()
        .format(format)
        .to_string()
}
fn escape(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}
fn ranked<K: Ord>(map: &BTreeMap<K, usize>) -> Vec<(&K, &usize)> {
    let mut rows: Vec<_> = map.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    rows
}

fn render(a: &Analysis, start: u64, end: u64, language: &str) -> String {
    let labels = crate::i18n::Text::new(language);
    let mut out = format!(
        "# {} — {}\n\nUTC: {} → {}\n\n",
        crate::branding::NAME,
        time(end, "%Y-%m-%d"),
        time(start, "%F %T"),
        time(end, "%F %T")
    );
    out.push_str(&labels.format(
        "report_summary",
        &[
            ("auto", a.incidents.to_string()),
            ("manual", a.manual.to_string()),
            ("tests", a.tests.to_string()),
            ("incomplete", a.incomplete.to_string()),
            ("wakes", a.wakes.len().to_string()),
            ("malformed", a.malformed.to_string()),
        ],
    ));
    let measured_delays: usize = ["present_block", "dwm_stall", "ui_hang"]
        .iter()
        .map(|key| a.reasons.get(*key).copied().unwrap_or(0))
        .sum();
    let memory_warnings = a.reasons.get("memory_pressure").copied().unwrap_or(0);
    out.push('\n');
    out.push_str(&labels.format(
        "report_assessment",
        &[
            ("delays", measured_delays.to_string()),
            ("memory", memory_warnings.to_string()),
        ],
    ));
    out.push('\n');
    out.push_str(labels.get("report_triggers"));
    for (name, count) in ranked(&a.reasons) {
        out.push_str(&format!(
            "| {} | {count} |\n",
            escape(labels.get(crate::assessment::reason_key(name)))
        ));
    }
    out.push('\n');
    out.push_str(labels.get("report_processes"));
    for (name, count) in ranked(&a.suspects).into_iter().take(8) {
        out.push_str(&format!("| {} | {count} |\n", escape(name)));
    }
    out.push('\n');
    out.push_str(labels.get("report_correlations"));
    for (name, count) in &a.correlations {
        let name = labels.get(match *name {
            "Page Reads >50/s" => "report_page_reads",
            "Disk latency/queue" => "report_disk_signal",
            "NVIDIA wake transition" => "report_gpu_wake",
            other => other,
        });
        out.push_str(&format!("| {name} | {count} |\n"));
    }
    for (name, count) in ranked(&a.drivers).into_iter().take(5) {
        let name = name
            .strip_prefix("unresolved ")
            .map(|n| format!("{} {n}", labels.get("report_unresolved")))
            .unwrap_or_else(|| name.clone());
        out.push_str(&format!(
            "| DPC/ISR: {} | {count} {} |\n",
            escape(&name),
            labels.get("analysis_events")
        ));
    }
    out.push('\n');
    out.push_str(labels.get("report_boot"));
    let mut boot: Vec<_> = a.boot.values().collect();
    boot.sort_by(|a, b| {
        (b.peak_io_bytes_sec + b.peak_cpu_pct * 1_048_576.0)
            .total_cmp(&(a.peak_io_bytes_sec + a.peak_cpu_pct * 1_048_576.0))
    });
    boot.truncate(10);
    boot.sort_by_key(|p| p.created_ms);
    for p in boot {
        out.push_str(&format!(
            "| {}{} | {} ({}) | {:.2} | {:.2} |\n",
            time(p.created_ms, "%T%.3f"),
            if p.already_running { " *" } else { "" },
            escape(&p.name),
            p.pid,
            p.peak_cpu_pct,
            p.peak_io_bytes_sec / 1_048_576.0
        ));
    }
    out.push_str(&format!("\n\\* {}\n", labels.get("report_boot_note")));
    if let Some(stable) = a.stable {
        out.push_str(&format!(
            "\n{}: {} UTC.\n",
            labels.get("report_stable"),
            time(stable, "%T")
        ));
    }
    out.push('\n');
    out.push_str(labels.get("report_recent"));
    let mut timeline = a.timeline.clone();
    timeline.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
    for (ts, label) in timeline.iter().take(8) {
        let (reasons, window) = label.split_once(" — ").unwrap_or((label.as_str(), ""));
        let reasons = reasons
            .split(", ")
            .map(|reason| labels.get(crate::assessment::reason_key(reason)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "| {} | {} — {} |\n",
            time(*ts, "%T%.3f"),
            escape(&reasons),
            window
        ));
    }
    out.push_str(&format!("\n## {}\n\n", labels.get("report_availability")));
    let warnings: std::collections::BTreeSet<_> =
        a.warnings.iter().map(|w| labels.diagnostic(w)).collect();
    for warning in warnings.iter().take(12) {
        out.push_str(&format!("- {}\n", escape(warning)));
    }
    out.push_str(&format!("\n{}\n", labels.get("report_technical_note")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_use_selected_language_for_sections_and_reasons() {
        let mut analysis = Analysis::default();
        analysis.reasons.insert("manual_mark".into(), 1);
        for language in crate::i18n::LANGUAGES {
            let text = crate::i18n::Text::new(language);
            let output = render(&analysis, 0, 2000, language);
            assert!(output.contains(text.get("event_manual")), "{language}");
            assert!(
                output.contains(text.get("report_availability")),
                "{language}"
            );
            assert!(!output.contains("{auto}") && !output.contains("{memory}"));
        }
        let en = render(&analysis, 0, 2000, "en");
        assert!(en.contains("Automatic assessment"));
        assert!(!en.contains("Автоматична оцінка"));
    }
    #[test]
    fn report_does_not_export_legacy_private_window_titles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incident-private.jsonl");
        let record = Record::Incident(Incident {
            id: "privacy".into(),
            triggers: vec![Trigger {
                ts_ms: 1000,
                reason: "manual_mark".into(),
                foreground: Foreground {
                    title: "SECRET chat or document title".into(),
                    program: "editor.exe".into(),
                    ..Default::default()
                },
            }],
            ..Default::default()
        });
        fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();
        let mut a = Analysis::default();
        analyze_file(&path, 0, 2000, &Config::default(), &mut a).unwrap();
        let output = render(&a, 0, 2000, "en");
        assert!(!output.contains("SECRET"));
        assert!(output.contains("editor.exe"));
    }
    #[test]
    fn continuous_log_bookmark_is_counted_once_with_or_without_incident() {
        let directory = tempfile::tempdir().unwrap();
        let marker = Trigger {
            ts_ms: 1000,
            reason: "manual_mark".into(),
            foreground: Foreground::default(),
        };
        let main = directory.path().join("stutter-main.jsonl");
        fs::write(
            &main,
            serde_json::to_string(&Record::Mark(marker.clone())).unwrap() + "\n",
        )
        .unwrap();
        let incident = directory.path().join("incident-one.jsonl");
        fs::write(
            &incident,
            serde_json::to_string(&Record::Incident(Incident {
                id: "1".into(),
                start_ms: 0,
                end_ms: 1000,
                complete: true,
                triggers: vec![marker],
                ..Default::default()
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
        for paths in [[&main, &incident], [&incident, &main]] {
            let mut analysis = Analysis::default();
            for path in paths {
                analyze_file(path, 0, 2000, &Config::default(), &mut analysis).unwrap();
            }
            assert_eq!(analysis.manual, 1);
            assert_eq!(analysis.incidents, 0);
            assert_eq!(analysis.malformed, 0);
        }
        let mut analysis = Analysis::default();
        analyze_file(&main, 0, 2000, &Config::default(), &mut analysis).unwrap();
        assert_eq!(analysis.manual, 1);
    }
    #[test]
    fn parser_handles_truncated_unknown_and_blank_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stutter-test.jsonl");
        fs::write(&path,"\n{\"type\":\"event\",\"ts_ms\":1,\"name\":\"test\",\"detail\":\"ok\"}\n{\"type\":\"future\"}\n{\"type\":").unwrap();
        let mut count = 0;
        assert_eq!(read_records(&path, |_| count += 1).unwrap(), 2);
        assert_eq!(count, 1);
    }
    #[test]
    fn report_filters_trigger_time_counts_manual_and_deduplicates_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("incident-test.jsonl");
        let foreground = Foreground::default();
        let header = Incident {
            id: "test".into(),
            start_ms: 0,
            end_ms: 20_000,
            complete: true,
            triggers: vec![
                Trigger {
                    ts_ms: 10_000,
                    reason: "manual_mark".into(),
                    foreground: foreground.clone(),
                },
                Trigger {
                    ts_ms: 10_000,
                    reason: "disk_queue".into(),
                    foreground,
                },
            ],
            ..Default::default()
        };
        let mut s = Sample {
            ts_ms: 10_000,
            ..Default::default()
        };
        s.cpu.top.push(Process {
            name: "test.exe".into(),
            cpu_pct: 5.0,
            ..Default::default()
        });
        let rows = [
            Record::Incident(header),
            Record::Sample(Box::new(s.clone())),
            Record::Sample(Box::new(s)),
        ];
        fs::write(
            &path,
            rows.iter()
                .map(|r| serde_json::to_string(r).unwrap() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let mut a = Analysis::default();
        analyze_file(&path, 9000, 11_000, &Config::default(), &mut a).unwrap();
        assert_eq!(a.manual, 1);
        assert_eq!(a.incidents, 1);
        assert_eq!(a.suspects["test.exe (CPU)"], 1);
        let mut excluded = Analysis::default();
        analyze_file(&path, 11_000, 15_000, &Config::default(), &mut excluded).unwrap();
        assert_eq!(excluded.incidents, 0);
    }
}

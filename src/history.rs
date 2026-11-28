//! Bounded restoration of the graph's last ten minutes, independent of the UI.
use crate::{
    config::Config,
    dashboard::Point,
    model::{Record, Summary},
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

pub fn restore(config: &Config, end: u64) -> Result<Vec<Point>> {
    let start = end.saturating_sub(600_000);
    let cutoff = SystemTime::UNIX_EPOCH + Duration::from_millis(start);
    let mut files = crate::storage::managed_files(&config.logs_dir)?;
    files.retain(|(p, modified, _)| {
        *modified >= cutoff
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("stutter-"))
    });
    files.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
    let mut bytes = 0;
    let mut points = BTreeMap::new();
    for (path, _, size) in files.into_iter().take(2048) {
        bytes += size;
        if bytes > 64 * 1024 * 1024 {
            break;
        }
        crate::report::read_records(&path, |record| {
            let mut add = |p: Point| {
                if p.ts >= start && p.ts <= end {
                    points
                        .entry(p.ts)
                        .and_modify(|old: &mut Point| {
                            old.alerts |= p.alerts;
                            for i in 0..8 {
                                if p.values[i].is_some() {
                                    old.values[i] = p.values[i];
                                }
                            }
                        })
                        .or_insert(p);
                    if points.len() > 6002 {
                        points.pop_first();
                    }
                }
            };
            match record {
                Record::Sample(sample) => add(Point::from_sample(&sample, config)),
                Record::Summary(summary) if summary.end_ms >= start && summary.start_ms <= end => {
                    for p in summary_points(&summary, config) {
                        add(p);
                    }
                }
                _ => {}
            }
        })?;
    }
    Ok(points.into_values().collect())
}
fn summary_points(summary: &Summary, config: &Config) -> Vec<Point> {
    let mut points = Vec::new();
    for (key, stats) in &summary.metrics {
        let metric = match key.as_str() {
            "cpu_pct" => Some(0),
            "ram_used_pct" => Some(1),
            "present_work_ms" => Some(3),
            "window_response_ms" => Some(4),
            "dpc_pct" => Some(5),
            "interrupt_pct" => Some(6),
            _ if key.starts_with("disk.") && key.ends_with("read_latency_ms") => Some(2),
            _ if key.starts_with("disk.") && key.ends_with("write_latency_ms") => Some(7),
            _ => None,
        };
        if let Some(metric) = metric {
            for (ts, value) in [(stats.min_ts_ms, stats.min), (stats.max_ts_ms, stats.max)] {
                let mut values = [None; 8];
                values[metric] = Some(value);
                points.push(Point {
                    ts: if ts == 0 { summary.start_ms } else { ts },
                    values,
                    span_ms: 1,
                    alerts: match metric {
                        0 if value >= 95.0 => crate::assessment::CPU_BUSY,
                        1 if value >= 90.0 => crate::assessment::RAM_BUSY,
                        2 | 7 if value > config.disk_latency_ms => crate::assessment::DISK,
                        3 if value > config.frame_ms => crate::assessment::PRESENT,
                        4 if value > config.hung_ms as f64 => crate::assessment::WINDOW,
                        5 | 6 if value > config.dpc_pct => crate::assessment::DPC,
                        _ => 0,
                    },
                });
            }
        }
    }
    points
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dashboard::Dashboard, model::*, storage::Storage};
    use std::sync::Arc;
    #[test]
    fn closing_the_view_keeps_collecting_graph_history() {
        let mut d = Dashboard::new(&Config::default());
        for i in 0..4000 {
            d.publish(Arc::new(Sample {
                ts_ms: 1_000_000 + i * 500,
                cpu: Cpu {
                    usage_pct: Some(i as f64),
                    ..Default::default()
                },
                ..Default::default()
            }));
        }
        assert!(!d.visible);
        assert_eq!(d.points.len(), 1201);
        assert_eq!(d.points.back().unwrap().values[0], Some(3999.0));
        d.visible = true;
        assert_eq!(d.points.len(), 1201);
        d.visible = false;
        d.publish(Arc::new(Sample {
            ts_ms: 3_000_000,
            ..Default::default()
        }));
        d.visible = true;
        assert_eq!(d.points.back().unwrap().ts, 3_000_000);
    }
    #[test]
    fn restart_restores_compressed_data_and_merges_new_samples() {
        let temp = tempfile::tempdir().unwrap();
        let c = Config {
            logs_dir: temp.path().into(),
            ..Default::default()
        };
        let mut store = Storage::new(&c).unwrap();
        let end = now_ms();
        let samples: Vec<_> = (0..120)
            .map(|i| {
                Arc::new(Sample {
                    ts_ms: end - 60_000 + i * 500,
                    cpu: Cpu {
                        usage_pct: Some(i as f64),
                        ..Default::default()
                    },
                    disk: Disk {
                        devices: vec![DiskDevice {
                            name: "test".into(),
                            read_latency_ms: Some(2.0),
                            write_latency_ms: Some(80.0),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                })
            })
            .collect();
        store.flush_samples(&samples).unwrap();
        let old = restore(&c, end).unwrap();
        assert_eq!(old.len(), 120);
        let mut d = Dashboard::new(&c);
        d.publish(Arc::new(Sample {
            ts_ms: end + 500,
            ..Default::default()
        }));
        d.merge_history(old);
        assert_eq!(d.points.len(), 121);
        assert!(!d.history_loading);
        assert_eq!(d.points[0].values[0], Some(0.0));
        assert_eq!(d.points[0].values[2], Some(2.0));
        assert_eq!(d.points[0].values[7], Some(80.0));
        assert_eq!(d.points.back().unwrap().ts, end + 500);
    }
    #[test]
    fn summary_restores_read_and_write_peaks_independently() {
        let mut summary = Summary {
            start_ms: 1000,
            end_ms: 2000,
            samples: 2,
            metrics: BTreeMap::new(),
            top_cpu: Vec::new(),
            top_memory: Vec::new(),
            top_io: Vec::new(),
            gpu_transitions: Vec::new(),
            boot: Boot::default(),
            capabilities: Vec::new(),
        };
        for (key, max) in [("read", 2.0), ("write", 80.0)] {
            summary.metrics.insert(
                format!("disk.test.{key}_latency_ms"),
                Stats {
                    min: 1.0,
                    max,
                    min_ts_ms: 1000,
                    max_ts_ms: 2000,
                    count: 2,
                    avg: max / 2.0,
                    p95: max,
                },
            );
        }
        let points = summary_points(&summary, &Config::default());
        assert!(
            points
                .iter()
                .any(|p| p.values[2] == Some(2.0) && p.values[7].is_none())
        );
        assert!(
            points
                .iter()
                .any(|p| p.values[7] == Some(80.0)
                    && crate::assessment::severity(p.alerts, &[7]) == 2)
        );
    }
}

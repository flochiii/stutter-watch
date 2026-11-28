use crate::{config::Config, model::*, recorder::Status};
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug)]
pub struct Point {
    pub ts: u64,
    pub values: [Option<f64>; 7],
    pub alerts: u16,
    pub span_ms: u16,
}
impl Point {
    pub fn from_sample(s: &Sample, config: &Config) -> Self {
        Self {
            ts: s.ts_ms,
            alerts: crate::assessment::signals(config, s),
            span_ms: s.interval_ms.max(config.sample_ms).min(3000) as u16,
            values: [
                s.cpu.usage_pct,
                (s.memory.total_bytes > 0)
                    .then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0),
                s.disk
                    .devices
                    .iter()
                    .flat_map(|d| [d.read_latency_ms, d.write_latency_ms])
                    .flatten()
                    .max_by(f64::total_cmp),
                s.frames.present_work_ms,
                s.foreground.response_peak_ms.map(|v| v as f64).or_else(|| {
                    s.foreground
                        .responsive
                        .map(|_| s.foreground.unresponsive_ms as f64)
                }),
                s.dpc.time_pct,
                s.dpc.interrupt_pct,
            ],
        }
    }
}
#[derive(Clone)]
pub struct Dashboard {
    pub config: Config,
    pub visible: bool,
    pub paused: bool,
    pub points: VecDeque<Point>,
    pub latest: Option<Arc<Sample>>,
    pub recording: Status,
    pub observed_seconds: f64,
    pub warnings: Vec<String>,
    pub samples: u64,
    pub history_loading: bool,
}
impl Dashboard {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
            visible: false,
            paused: false,
            points: VecDeque::new(),
            latest: None,
            recording: Status::default(),
            observed_seconds: 0.0,
            warnings: Vec::new(),
            samples: 0,
            history_loading: true,
        }
    }
    pub fn publish(&mut self, s: Arc<Sample>) {
        self.samples += 1;
        let point = Point::from_sample(&s, &self.config);
        let cutoff = point.ts.saturating_sub(600_000);
        while self.points.front().is_some_and(|p| p.ts < cutoff) || self.points.len() >= 6002 {
            self.points.pop_front();
        }
        self.points.push_back(point);
        self.latest = Some(s);
    }
    pub fn merge_history(&mut self, history: Vec<Point>) {
        let mut points = std::collections::BTreeMap::new();
        for p in history.into_iter().chain(self.points.drain(..)) {
            points
                .entry(p.ts)
                .and_modify(|old: &mut Point| {
                    old.alerts |= p.alerts;
                    for i in 0..7 {
                        if p.values[i].is_some() {
                            old.values[i] = p.values[i];
                        }
                    }
                })
                .or_insert(p);
        }
        let cutoff = points
            .last_key_value()
            .map(|(&ts, _)| ts.saturating_sub(600_000))
            .unwrap_or(0);
        self.points = points
            .into_values()
            .filter(|p| p.ts >= cutoff)
            .rev()
            .take(6002)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.history_loading = false;
    }
}

pub fn envelope(
    points: &[Point],
    series: usize,
    start: u64,
    end: u64,
    width: usize,
) -> Vec<(usize, f64, f64)> {
    let mut buckets = vec![None::<(f64, f64)>; width.max(1)];
    for p in points {
        if p.ts < start || p.ts > end {
            continue;
        }
        if let Some(v) = p.values[series].filter(|v| v.is_finite()) {
            let x = ((p.ts - start) as f64 / (end - start).max(1) as f64
                * (width.saturating_sub(1)) as f64) as usize;
            let b = &mut buckets[x];
            *b = Some(b.map(|(lo, hi)| (lo.min(v), hi.max(v))).unwrap_or((v, v)));
        }
    }
    buckets
        .into_iter()
        .enumerate()
        .filter_map(|(i, b)| b.map(|(lo, hi)| (i, lo, hi)))
        .collect()
}

use crate::{
    config::Config,
    model::{Incident, Sample, Trigger},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

pub struct Ring {
    samples: VecDeque<Arc<Sample>>,
    seconds: u64,
    capacity: usize,
}
impl Ring {
    pub fn new(seconds: u64, sample_ms: u64) -> Self {
        let capacity = (seconds * 1000 / sample_ms + 2) as usize;
        Self {
            samples: VecDeque::with_capacity(capacity),
            seconds,
            capacity,
        }
    }
    pub fn push(&mut self, sample: Arc<Sample>) {
        let cutoff = sample.elapsed_ms.saturating_sub(self.seconds * 1000);
        while self.samples.front().is_some_and(|s| s.elapsed_ms < cutoff)
            || self.samples.len() >= self.capacity
        {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
    pub fn recent(&self, seconds: u64) -> Vec<Arc<Sample>> {
        let cutoff = self
            .samples
            .back()
            .map(|s| s.elapsed_ms.saturating_sub(seconds * 1000))
            .unwrap_or(0);
        self.samples
            .iter()
            .filter(|s| s.elapsed_ms >= cutoff)
            .cloned()
            .collect()
    }
    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back().map(Arc::as_ref)
    }
}

#[derive(Clone)]
pub struct Pending {
    pub header: Incident,
    pub samples: Vec<Arc<Sample>>,
    pub deadline_elapsed_ms: u64,
}

pub struct Detector {
    config: Config,
    pub ring: Ring,
    pub pending: Option<Pending>,
    others: VecDeque<Pending>,
    last: HashMap<&'static str, u64>,
    serial: u64,
}

impl Detector {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
            ring: Ring::new(config.ring_seconds, config.sample_ms),
            pending: None,
            others: VecDeque::new(),
            last: HashMap::new(),
            serial: 0,
        }
    }
    pub fn rules(&self, sample: &Sample) -> Vec<&'static str> {
        crate::assessment::reasons(&self.config, sample)
    }

    /// Returns true when the provisional incident must be written immediately.
    pub fn push(&mut self, sample: Arc<Sample>) -> bool {
        if sample.interval_ms > 3_000 || sample.interactive == Some(false) {
            if let Some(mut pending) = self.pending.take() {
                pending.deadline_elapsed_ms = u64::MAX;
                self.others.push_back(pending);
            }
            for pending in &mut self.others {
                pending.deadline_elapsed_ms = u64::MAX;
            }
            self.clear_history();
            return false;
        }
        for pending in &mut self.others {
            if pending.deadline_elapsed_ms != u64::MAX
                && sample.elapsed_ms
                    <= pending
                        .deadline_elapsed_ms
                        .saturating_add(self.config.sample_ms)
            {
                pending.samples.push(Arc::clone(&sample));
                pending.header.end_ms = sample.ts_ms;
            }
        }
        if let Some(pending) = &mut self.pending {
            pending.samples.push(sample.clone());
            pending.header.end_ms = sample.ts_ms;
        }
        let reasons: Vec<_> = self
            .rules(&sample)
            .into_iter()
            .filter(|reason| {
                self.last.get(reason).is_none_or(|last| {
                    sample.elapsed_ms.saturating_sub(*last) >= self.config.cooldown_seconds * 1000
                })
            })
            .collect();
        self.ring.push(sample.clone());
        if reasons.is_empty() {
            return false;
        }
        for reason in &reasons {
            self.last.insert(reason, sample.elapsed_ms);
        }
        let triggers = reasons
            .into_iter()
            .map(|reason| Trigger {
                ts_ms: sample.ts_ms,
                reason: reason.into(),
                foreground: sample.foreground.clone(),
            })
            .collect();
        self.begin(triggers, self.config.pre_seconds, false);
        true
    }

    pub fn mark(&mut self, reason: &str) -> bool {
        let Some(sample) = self.ring.latest() else {
            return false;
        };
        let trigger = Trigger {
            ts_ms: sample.ts_ms,
            reason: reason.into(),
            foreground: sample.foreground.clone(),
        };
        self.begin(vec![trigger], self.config.manual_seconds, true);
        true
    }

    pub fn mark_now(&mut self, reason: &str, foreground: crate::model::Foreground) -> bool {
        if self.ring.latest().is_none() {
            return false;
        }
        self.begin(
            vec![Trigger {
                ts_ms: crate::model::now_ms(),
                reason: reason.into(),
                foreground,
            }],
            self.config.manual_seconds,
            true,
        );
        true
    }

    pub fn clear_history(&mut self) {
        self.ring = Ring::new(self.config.ring_seconds, self.config.sample_ms);
        self.last.clear();
    }

    pub fn shutdown_snapshot(&mut self) {
        if let Some(sample) = self.ring.latest() {
            let trigger = Trigger {
                ts_ms: sample.ts_ms,
                reason: "shutdown".into(),
                foreground: sample.foreground.clone(),
            };
            self.begin(vec![trigger], self.config.ring_seconds, true);
        }
    }

    fn begin(&mut self, triggers: Vec<Trigger>, seconds: u64, manual: bool) {
        let Some(latest) = self.ring.latest() else {
            return;
        };
        // Each manual mark has an independent, finite window. Shared samples make
        // overlapping marks cheap without extending an in-memory incident forever.
        if manual && let Some(pending) = self.pending.take() {
            self.others.push_back(pending);
        }
        if let Some(pending) = &mut self.pending {
            pending.header.triggers.extend(triggers);
            if manual {
                let recent = self.ring.recent(seconds);
                let earliest = pending
                    .samples
                    .first()
                    .map(|s| s.elapsed_ms)
                    .unwrap_or(u64::MAX);
                let mut prefix: Vec<_> = recent
                    .into_iter()
                    .filter(|s| s.elapsed_ms < earliest)
                    .collect();
                prefix.append(&mut pending.samples);
                pending.samples = prefix;
                pending.header.start_ms = pending
                    .samples
                    .first()
                    .map(|s| s.ts_ms)
                    .unwrap_or(latest.ts_ms);
            }
            return;
        }
        self.serial += 1;
        let protected = triggers.iter().any(|t| t.reason == "manual_mark");
        let post_ms = if protected {
            self.config.manual_post_seconds * 1000
        } else if manual {
            0
        } else {
            self.config.post_seconds * 1000
        };
        let trigger_ts = triggers.first().map(|t| t.ts_ms).unwrap_or(latest.ts_ms);
        let offset = trigger_ts
            .saturating_sub(latest.ts_ms)
            .min(self.config.sample_ms);
        let trigger_elapsed = latest.elapsed_ms + offset;
        let samples = self
            .ring
            .recent(seconds)
            .into_iter()
            .filter(|s| s.elapsed_ms >= trigger_elapsed.saturating_sub(seconds * 1000))
            .collect::<Vec<_>>();
        self.pending = Some(Pending {
            header: Incident {
                id: format!("{}-{}-{}", latest.ts_ms, std::process::id(), self.serial),
                start_ms: samples.first().map(|s| s.ts_ms).unwrap_or(latest.ts_ms),
                end_ms: latest.ts_ms,
                complete: false,
                protected,
                requested_start_ms: trigger_ts.saturating_sub(seconds * 1000),
                requested_end_ms: trigger_ts + post_ms,
                triggers,
            },
            samples,
            deadline_elapsed_ms: trigger_elapsed + post_ms,
        });
    }

    pub fn take_ready(&mut self, shutdown: bool) -> Option<Pending> {
        let elapsed = self.ring.latest().map(|s| s.elapsed_ms).unwrap_or(0);
        if let Some(index) = self.others.iter().position(|p| {
            shutdown || p.deadline_elapsed_ms == u64::MAX || elapsed >= p.deadline_elapsed_ms
        }) {
            let mut pending = self.others.remove(index)?;
            pending.header.complete = elapsed >= pending.deadline_elapsed_ms;
            return Some(pending);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| shutdown || elapsed >= p.deadline_elapsed_ms)
        {
            let mut pending = self.pending.take()?;
            pending.header.complete = elapsed >= pending.deadline_elapsed_ms;
            Some(pending)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(ms: u64) -> Sample {
        Sample {
            ts_ms: 1_000_000 + ms,
            elapsed_ms: ms,
            ..Default::default()
        }
    }
    #[test]
    fn static_present_gaps_stale_dpc_and_lock_are_not_incidents() {
        let d = Detector::new(&Config::default());
        let mut s = sample(50_000);
        s.frames.max_frame_time_ms = Some(2000.0);
        s.dpc.spikes.push(crate::model::DriverSpike {
            ts_ms: 1,
            duration_ms: 500.0,
            ..Default::default()
        });
        assert!(d.rules(&s).is_empty());
        s.dpc.spikes[0].ts_ms = s.ts_ms;
        assert_eq!(d.rules(&s), vec!["driver_spike"]);
        s.interactive = Some(false);
        s.frames.present_work_ms = Some(1000.0);
        assert!(d.rules(&s).is_empty());
        s.interactive = Some(true);
        s.interval_ms = 20_000;
        assert!(d.rules(&s).is_empty());
    }
    #[test]
    fn overlapping_manual_marks_each_keep_their_post_window() {
        let mut d = Detector::new(&Config::default());
        for n in 0..=80 {
            d.push(sample(n * 500).into());
        }
        assert!(d.mark("manual_mark"));
        for n in 81..=90 {
            d.push(sample(n * 500).into());
        }
        assert!(d.mark("manual_mark"));
        for n in 91..=100 {
            d.push(sample(n * 500).into());
        }
        let a = d.take_ready(false).unwrap();
        assert_eq!(a.header.start_ms, 1_010_000);
        assert_eq!(a.header.end_ms, 1_050_000);
        assert!(a.header.protected);
        assert!(d.take_ready(false).is_none());
        for n in 101..=110 {
            d.push(sample(n * 500).into());
        }
        let b = d.take_ready(false).unwrap();
        assert_eq!(b.header.start_ms, 1_015_000);
        assert_eq!(b.header.end_ms, 1_055_000);
        assert!(b.header.complete);
        assert!(Arc::ptr_eq(&a.samples[10], &b.samples[0]));
    }
    #[test]
    fn sleep_interrupts_pending_mark_without_claiming_complete_capture() {
        let mut d = Detector::new(&Config::default());
        for n in 0..=80 {
            d.push(sample(n * 500).into());
        }
        d.mark("manual_mark");
        let mut wake = sample(100_000);
        wake.interval_ms = 60_000;
        assert!(!d.push(wake.into()));
        let p = d.take_ready(false).unwrap();
        assert!(p.header.protected);
        assert!(!p.header.complete);
        assert!(d.ring.latest().is_none());
    }
    #[test]
    fn ring_uses_elapsed_time_and_is_bounded() {
        let mut ring = Ring::new(30, 500);
        for n in 0..200 {
            ring.push(sample(n * 500).into());
        }
        assert_eq!(ring.recent(30).len(), 61);
        assert_eq!(ring.recent(15).len(), 31);
        assert_eq!(ring.recent(30)[0].elapsed_ms, 69_500);
        let mut backwards = sample(100_000);
        backwards.ts_ms = 0;
        ring.push(backwards.into());
        assert_eq!(ring.recent(1).len(), 3);
    }
    #[test]
    fn triggers_capture_pre_post_and_cooldown() {
        let mut d = Detector::new(&Config::default());
        for n in 0..60 {
            d.push(sample(n * 500).into());
        }
        let mut s = sample(30_000);
        s.frames.present_work_ms = Some(101.0);
        assert!(d.push(s.clone().into()));
        assert_eq!(d.pending.as_ref().unwrap().samples.len(), 31);
        assert!(d.take_ready(false).is_none());
        for n in 61..=70 {
            s.elapsed_ms = n * 500;
            s.ts_ms = 1_000_000 + n * 500;
            assert!(!d.push(s.clone().into()));
        }
        let p = d.take_ready(false).unwrap();
        assert!(p.header.complete);
        assert_eq!(p.samples.len(), 41);
    }
    #[test]
    fn manual_mark_preserves_thirty_before_and_ten_after() {
        let mut d = Detector::new(&Config::default());
        for n in 0..80 {
            d.push(sample(n * 500).into());
        }
        assert!(d.mark("manual_mark"));
        assert!(d.take_ready(false).is_none());
        for n in 80..100 {
            d.push(sample(n * 500).into());
        }
        let p = d.take_ready(false).unwrap();
        assert_eq!(p.samples.len(), 81);
        assert!(p.header.protected);
        assert!(p.header.complete);
        assert_eq!(p.header.triggers[0].reason, "manual_mark");
    }
    #[test]
    fn thresholds_missing_metrics_and_wakeup_are_precise() {
        let d = Detector::new(&Config::default());
        let mut s = sample(0);
        assert!(d.rules(&s).is_empty());
        s.frames.max_frame_time_ms = Some(100.0);
        assert!(d.rules(&s).is_empty());
        s.foreground.responsive = Some(false);
        s.foreground.unresponsive_ms = 501;
        s.disk.devices.push(crate::model::DiskDevice {
            queue: Some(4.1),
            ..Default::default()
        });
        s.dpc.time_pct = Some(16.0);
        s.gpu.transitions.push(crate::model::GpuTransition {
            from: "unknown".into(),
            to: "active".into(),
            ..Default::default()
        });
        assert_eq!(d.rules(&s), vec!["ui_hang", "disk_queue", "dpc_interrupt"]);
        s.gpu.transitions[0].from = "sleeping".into();
        assert!(d.rules(&s).contains(&"dgpu_wakeup"));
        s.foreground.fullscreen = true;
        assert!(!d.rules(&s).contains(&"dgpu_wakeup"));
    }
}

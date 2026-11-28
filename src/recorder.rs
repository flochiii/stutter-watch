//! One bounded disk worker. A slow disk cannot block the sampler or grow a queue.
use crate::{
    config::Config,
    detector::Pending,
    model::{Record, Sample, now_ms},
    storage::{Storage, Usage},
};
use anyhow::Result;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::JoinHandle,
};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub usage: Usage,
    pub error: String,
    pub omitted_jobs: u64,
}
enum Job {
    Append(Record),
    Samples(Vec<Arc<Sample>>),
    Incident(Pending),
    Configure(Box<Config>),
    Barrier(mpsc::Sender<()>),
    Stop,
}
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    omitted: u64,
}
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    space: Condvar,
    status: Mutex<Status>,
}
pub struct Recorder {
    pub root: PathBuf,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Recorder {
    pub fn new(config: &Config) -> Result<Self> {
        let mut store = Storage::new(config)?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            space: Condvar::new(),
            status: Mutex::new(Status {
                usage: store.usage.clone(),
                ..Default::default()
            }),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("history-writer".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                loop {
                    let (job, missed) = {
                        let mut q = worker.queue.lock().unwrap_or_else(|p| p.into_inner());
                        while q.jobs.is_empty() {
                            q = worker.ready.wait(q).unwrap_or_else(|p| p.into_inner());
                        }
                        let job = q.jobs.pop_front().expect("queued job");
                        let missed = std::mem::take(&mut q.omitted);
                        worker.space.notify_all();
                        (job, missed)
                    };
                    if matches!(job, Job::Stop) {
                        break;
                    }
                    if missed > 0 {
                        let _ = store.append(&Record::Event {
                            ts_ms: now_ms(),
                            name: "recording_incomplete".into(),
                            detail: format!("{missed} jobs omitted by bounded writer queue"),
                        });
                    }
                    let result = match job {
                        Job::Append(record) => store.append(&record),
                        Job::Samples(samples) => store.flush_samples(&samples),
                        Job::Incident(pending) => store.incident(&pending).map(|_| ()),
                        Job::Configure(config) => Storage::new(&config).map(|s| store = s),
                        Job::Barrier(reply) => {
                            let _ = reply.send(());
                            continue;
                        }
                        Job::Stop => break,
                    };
                    let mut status = worker.status.lock().unwrap_or_else(|p| p.into_inner());
                    status.usage = store.usage.clone();
                    if let Err(e) = result {
                        status.omitted_jobs += 1;
                        status.error = format!("{e:#}");
                    } else {
                        status.error.clear();
                    }
                }
            })?;
        Ok(Self {
            root: config.logs_dir.clone(),
            shared,
            thread: Some(thread),
        })
    }
    pub fn status(&self) -> Status {
        self.shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    fn send(&self, job: Job) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        if let Job::Incident(pending) = &job {
            // A final snapshot replaces its queued provisional version without retaining two buffers.
            if let Some(existing) = q
                .jobs
                .iter_mut()
                .find(|j| matches!(j,Job::Incident(p) if p.header.id==pending.header.id))
            {
                *existing = job;
                self.shared.ready.notify_one();
                return;
            }
        }
        let full = if matches!(job, Job::Samples(_)) {
            q.jobs
                .iter()
                .filter(|j| matches!(j, Job::Samples(_)))
                .count()
                >= 2
        } else {
            q.jobs.len() >= 18
        };
        if full {
            q.omitted += 1;
            let mut status = self.shared.status.lock().unwrap_or_else(|p| p.into_inner());
            status.omitted_jobs += 1;
            status.error = "writer_queue_full".into();
            return;
        }
        q.jobs.push_back(job);
        self.shared.ready.notify_one();
    }
    fn control(&self, job: Job) {
        let mut q = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        while q.jobs.len() >= 18 {
            q = self.shared.space.wait(q).unwrap_or_else(|p| p.into_inner());
        }
        q.jobs.push_back(job);
        self.shared.ready.notify_one();
    }
    pub fn append(&self, record: Record) {
        self.send(Job::Append(record));
    }
    pub fn samples(&self, samples: Vec<Arc<Sample>>) {
        if !samples.is_empty() {
            self.send(Job::Samples(samples));
        }
    }
    pub fn incident(&self, pending: &Pending) {
        self.send(Job::Incident(pending.clone()));
    }
    pub fn configure(&self, config: &Config) {
        self.control(Job::Configure(Box::new(config.clone())));
    }
    pub fn barrier(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.control(Job::Barrier(tx));
        rx.recv()?;
        Ok(())
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.control(Job::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

//! Reproducible synthetic recording for screenshots; never reads machine metrics.
use stutter_watch::{config::Config, model::*, storage::Storage};
fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Provide an empty output directory"))?,
    );
    std::fs::create_dir_all(&root)?;
    anyhow::ensure!(
        std::fs::read_dir(&root)?.next().is_none(),
        "Output directory must be empty"
    );
    let mut c = Config {
        logs_dir: root.canonicalize()?.join("logs"),
        etw: false,
        kernel_etw: false,
        nvml: false,
        ..Default::default()
    };
    c.collection.set_flags([false; 14]);
    c.collection.cpu_total = true;
    c.collection.memory = true;
    c.collection.disks = true;
    c.save(&root.join("config.toml"))?;
    let end = now_ms();
    let mut samples = Vec::new();
    for i in 0..1200 {
        let peak = (870..876).contains(&i);
        samples.push(Sample {
            ts_ms: end - 600_000 + i * 500,
            interval_ms: 500,
            cpu: Cpu {
                usage_pct: Some(if peak {
                    98.0
                } else {
                    12.0 + (i as f64 / 27.0).sin() * 8.0
                }),
                ..Default::default()
            },
            memory: Memory {
                total_bytes: 16_000_000_000,
                used_bytes: 10_200_000_000,
                available_bytes: 5_800_000_000,
                ..Default::default()
            },
            disk: Disk {
                devices: vec![DiskDevice {
                    name: "Synthetic disk".into(),
                    read_latency_ms: Some(if peak {
                        125.0
                    } else {
                        1.2 + (i as f64 / 13.0).sin().abs() * 2.0
                    }),
                    write_latency_ms: Some(1.5),
                    ..Default::default()
                }],
                ..Default::default()
            },
            pressure: Some(Pressure {
                cpu: Some(PressureResource {
                    some_pct: Some(if peak { 18.0 } else { 0.2 }),
                    ..Default::default()
                }),
                memory: Some(PressureResource {
                    full_pct: Some(0.0),
                    ..Default::default()
                }),
                io: Some(PressureResource {
                    full_pct: Some(if peak { 8.0 } else { 0.1 }),
                    ..Default::default()
                }),
            }),
            interactive: Some(true),
            sources: Some(c.collection.clone()),
            ..Default::default()
        });
    }
    let mut storage = Storage::new(&c)?;
    let samples: Vec<_> = samples.into_iter().map(std::sync::Arc::new).collect();
    storage.flush_samples(&samples)?;
    let incident = Incident {
        id: format!("synthetic-{end}"),
        start_ms: samples[840].ts_ms,
        end_ms: samples[920].ts_ms,
        protected: true,
        complete: true,
        triggers: vec![Trigger {
            ts_ms: samples[870].ts_ms,
            reason: "disk_latency".into(),
            foreground: Foreground::default(),
        }],
        ..Default::default()
    };
    storage.incident(&stutter_watch::detector::Pending {
        header: incident,
        samples: samples[840..=920].to_vec(),
        deadline_elapsed_ms: 0,
    })?;
    println!("Synthetic fixture: {}", root.display());
    Ok(())
}

use crate::{branding, config::Config, i18n::Text, model::Sample};

/// Formats existing telemetry only; changing tray choices never starts collectors.
pub fn tooltip(
    config: &Config,
    text: &Text,
    sample: Option<&Sample>,
    paused: bool,
    warning: bool,
) -> String {
    let display = &config.tray_display;
    let mut lines = vec![branding::NAME.to_owned()];
    if display.status {
        lines.push(format!(
            "{}{} · {} {}/{} {}",
            text.get(if paused { "pause" } else { "active" }),
            if warning { " [!]" } else { "" },
            config.sample_ms,
            text.get("unit_ms"),
            config.flush_seconds,
            text.get("unit_second")
        ));
    }
    let sample = sample.filter(|_| !paused);
    let mut metrics = Vec::new();
    let milliseconds = format!(" {}", text.get("unit_ms"));
    let number = |value: Option<f64>, unit: &str| {
        value
            .filter(|v| v.is_finite())
            .map(|v| format!("{v:.0}{unit}"))
            .unwrap_or_else(|| "—".into())
    };
    if display.cpu {
        metrics.push(format!(
            "CPU: {}",
            number(
                sample
                    .filter(|_| config.collection.cpu_total)
                    .and_then(|s| s.cpu.usage_pct),
                "%"
            )
        ));
    }
    if display.memory {
        metrics.push(format!(
            "RAM: {}",
            number(
                sample
                    .filter(|_| config.collection.memory)
                    .and_then(|s| (s.memory.total_bytes > 0)
                        .then(|| s.memory.used_bytes as f64 / s.memory.total_bytes as f64 * 100.0)),
                "%"
            )
        ));
    }
    if display.gpu {
        metrics.push(format!(
            "GPU: {}",
            number(
                sample
                    .filter(|_| config.collection.gpu_source())
                    .and_then(|s| s
                        .gpu
                        .adapters
                        .iter()
                        .filter_map(|a| a.usage_pct)
                        .max_by(f64::total_cmp)),
                "%"
            )
        ));
    }
    if display.disk_latency {
        metrics.push(format!(
            "{}: {}",
            text.get("tray_disk"),
            number(
                sample.filter(|_| config.collection.disks).and_then(|s| s
                    .disk
                    .devices
                    .iter()
                    .flat_map(|d| [d.read_latency_ms, d.write_latency_ms])
                    .flatten()
                    .max_by(f64::total_cmp)),
                &milliseconds
            )
        ));
    }
    if display.dpc {
        metrics.push(format!(
            "DPC: {}",
            number(
                sample
                    .filter(|_| config.collection.dpc)
                    .and_then(|s| s.dpc.time_pct),
                "%"
            )
        ));
    }
    if display.window_response {
        metrics.push(format!(
            "UI: {}",
            number(
                sample
                    .filter(|_| config.collection.window_response)
                    .and_then(
                        |s| s.foreground.response_peak_ms.map(|v| v as f64).or_else(|| s
                            .foreground
                            .responsive
                            .map(|_| s.foreground.unresponsive_ms as f64))
                    ),
                &milliseconds
            )
        ));
    }
    for row in metrics.chunks(3) {
        lines.push(row.join(" · "));
    }
    // Shell only receives the stable product name. The custom popup can show
    // every selected metric without the legacy szTip character limit.
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_metrics_handle_disabled_missing_and_paused_sources() {
        let mut config = Config::default();
        config.tray_display.status = false;
        let mut sample = Sample::default();
        sample.cpu.usage_pct = Some(23.4);
        sample.memory.total_bytes = 100;
        sample.memory.used_bytes = 75;
        assert_eq!(
            tooltip(
                &config,
                &Text::new(&config.language),
                Some(&sample),
                false,
                false
            ),
            format!("{}\nCPU: 23% · RAM: 75%", branding::NAME)
        );
        config.collection.cpu_total = false;
        assert!(
            tooltip(
                &config,
                &Text::new(&config.language),
                Some(&sample),
                false,
                false
            )
            .contains("CPU: —")
        );
        assert!(
            tooltip(
                &config,
                &Text::new(&config.language),
                Some(&sample),
                true,
                false
            )
            .contains("RAM: —")
        );
        config.tray_display.set_flags([false; 7]);
        assert_eq!(
            tooltip(&config, &Text::new(&config.language), None, false, false),
            branding::NAME
        );
    }
    #[test]
    fn all_selected_metrics_remain_complete_in_every_locale() {
        let mut config = Config::default();
        config.tray_display.set_flags([true; 7]);
        config.sample_ms = 5000;
        config.flush_seconds = 300;
        let mut sample = Sample::default();
        sample.cpu.usage_pct = Some(100.0);
        sample.memory.total_bytes = 100;
        sample.memory.used_bytes = 100;
        sample.gpu.adapters.push(crate::model::Adapter {
            usage_pct: Some(100.0),
            ..Default::default()
        });
        sample.disk.devices.push(crate::model::DiskDevice {
            read_latency_ms: Some(10_000.0),
            ..Default::default()
        });
        sample.dpc.time_pct = Some(100.0);
        sample.foreground.response_peak_ms = Some(10_000);
        for language in crate::i18n::LANGUAGES {
            config.language = language.into();
            let text = Text::new(&config.language);
            let tooltip = tooltip(&config, &text, Some(&sample), false, true);
            assert!(
                tooltip.contains(&format!("UI: 10000 {}", text.get("unit_ms"))),
                "{language}: final metric was truncated: {tooltip}"
            );
        }
    }
}

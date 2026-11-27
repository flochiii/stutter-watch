# Data and privacy

[README](../README.md) · [Getting started](GETTING_STARTED.md) · [Architecture](ARCHITECTURE.md)

## What is collected

Enabled sources can record CPU and memory usage, disk counters, selected process summaries, foreground process identity and response, GPU activity and optional NVIDIA sensors, Present/DWM work, DPC/ISR activity and driver events, and a bounded startup observation. The **Data** page controls their lifecycle. Hiding a graph does not stop collection.

Window titles are **off by default** (`privacy.window_titles = false`). The recorder does not store full process command lines or executable paths as ordinary telemetry. Process basenames, PIDs, hardware and driver information, and timing can still be sensitive. Enabling title collection can expose document, browser, or chat names.

There are no analytics, accounts, automatic log uploads, or automatic updates in the recorder. GitHub hosts the project and website separately. Installing dependencies and running GitHub Actions use those services; this does not add network telemetry to the app.

## Storage and retention

Logs resolve relative to the configuration file. New ordinary history defaults to **500,000,000 bytes (500 MB)** and **24 hours**, with the older history removed when either limit applies. The panel uses decimal MB. Legacy `max_disk_mb` and `rotate_mb` settings use MiB where documented.

| Data | Behavior |
| --- | --- |
| `lossless` ordinary history | Every received selected sample is stored in checksummed JSONL/LZ4 segments, preserving recorded timestamps and `f64` values. |
| `summary` ordinary history | Lossy time buckets with count/min/max/average/p95 and extrema timestamps. Detection happens before aggregation. |
| Automatic incidents | Full selected samples in checksummed JSONL/LZ4, within ordinary retention. |
| Manual captures | Full selected samples in compressed protected storage; excluded from ordinary rotation. |

**Protected captures can grow beyond the ordinary budget.** Review and archive or delete them yourself after stopping the recorder. There is no automatic cleanup of protected recordings. The UI reports ordinary and protected usage separately, plus observed MB/hour.

The default free-space reserve is 256 MB plus segment headroom. Low disk space or writer failures are surfaced; a bounded writer queue can drop work under overload and records gaps. No storage scheme guarantees the requested history duration on a full or failing disk.

Manual marks request 30 seconds before and ten seconds after the trigger. Starting, pausing, sleep/lock, or shutdown can shorten or interrupt that interval. Requested and actual bounds and completion state are kept separately. Legacy JSONL remains readable alongside compressed segments.

Completed segments and incident snapshots are compressed immediately; there is no ten-minute wait. To compress legacy JSONL, stop the recorder and run `--config <path> compress-history`. This offline migration compares every decoded byte before removing an original and preserves its modification time and protected status. It does not reserialize values, merge unrelated incidents, or silently overwrite a mismatched archive.

## Sharing diagnostic files

Review each file before sharing. Prefer a minimal excerpt or screenshot with unrelated details removed. Old recordings may contain titles even if your current config disables them. Markdown reports suppress old title fields when current title privacy is off, but this is not a general-purpose anonymizer.

Never attach your whole logs folder by default. Public GitHub issues and attachments are public. Report vulnerabilities privately using [SECURITY.md](../SECURITY.md).

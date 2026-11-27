<p align="center"><img src="assets/app.svg" width="80" height="80" alt="Stutter Watch icon"></p>

<h1 align="center">Stutter Watch</h1>

<p align="center">Catch intermittent Windows stutters. Keep the context. Inspect it locally.</p>

[![Windows CI](https://github.com/flochiii/stutter-watch/actions/workflows/ci.yml/badge.svg)](https://github.com/flochiii/stutter-watch/actions/workflows/ci.yml)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/License-PolyForm%20Noncommercial%201.0.0-6a9ed8)](LICENSE)
![Windows x64](https://img.shields.io/badge/platform-Windows%20x64-555)
![Rust 1.97+](https://img.shields.io/badge/Rust-1.97%2B-b7410e)

Stutter Watch is a native Windows performance recorder written in Rust. It keeps a rolling history while you work, saves the time around a manual bookmark or an automatic signal, and opens the recording in local charts. It is designed for brief UI freezes that are difficult to reproduce on demand.

[Getting started](docs/GETTING_STARTED.md) · [Data & privacy](docs/DATA_AND_PRIVACY.md) · [Roadmap](docs/ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [License](LICENSE)

> **Early development, source available now.** There are no official downloadable builds yet. Build from source using the instructions below. A detected incident is evidence to inspect, not a proven explanation for a visible freeze.

## What it does

- **Four local charts:** CPU/RAM, disk latency, measured Present API work/window response, and DPC/ISR activity. Reopening the monitor restores up to ten minutes of collected history.
- **Protected bookmarks:** press **Ctrl+Shift+F12** to preserve the preceding 30 seconds and following 10 seconds. Manual captures survive ordinary history rotation.
- **Automatic signals:** highlight high load in yellow and measured delays or critical memory pressure in red. Each incident includes a short assessment and peak values.
- **Selectable collection:** choose CPU, memory, disks, process summaries, window response, GPU, ETW events, and other sources. Display toggles and collection controls are separate.
- **Bounded ordinary history:** default 24-hour retention and a 500 MB budget, whichever limit is reached first. Choose full selected samples with LZ4 compression or a smaller, lossy summary mode.
- **Tray operation:** closing the panel keeps recording. Choose the metrics shown in its live hover panel. English is the default interface language; ten language catalogs are included.

## Build and try it

Requirements: **Windows x64**, **Rust 1.97 or later** using the MSVC toolchain, **Visual Studio Build Tools with C++ support**, and the **Windows SDK**. Windows 11 has been exercised locally; broader Windows 10/11 coverage is still needed.

```powershell
git clone https://github.com/flochiii/stutter-watch.git
cd stutter-watch
cargo build --locked --release
Copy-Item config.example.toml config.toml
.\target\release\stutter-watch.exe --config .\config.toml --panel
```

1. Leave the recorder running while you work.
2. When you notice a stutter, press **Ctrl+Shift+F12**. Allow another ten seconds for the capture to finish.
3. Open **Incidents** and inspect the time around the bookmark. The latest recording is selected automatically.

Run without administrator privileges first. Some ETW sources need elevation or appropriate Windows permissions; an unavailable source shows its reason in **Data** while other sources continue. NVIDIA sensors are gated by confirmed activity and power-state checks to avoid polling an idle discrete GPU unnecessarily.

The tray menu provides pause, bookmark, report, and exit. **X closes only the panel.** Use **Quit** or `shutdown` to stop recording. See [the guide](docs/GETTING_STARTED.md) for background startup, updates, and troubleshooting.

## Understand the evidence

High CPU use alone is not a stutter. Present API duration is not display scan-out latency, and a window-response probe is not the end-to-end time of every click. Missing or disabled measurements appear as unavailable rather than a fabricated zero. Idle frame gaps, lock/sleep, and stale events are filtered from automatic delay detection.

Stutter Watch complements [PresentMon](https://github.com/GameTechDev/PresentMon) and [Windows Performance Recorder/Analyzer](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/). It does not replace their frame correlation or stack-level analysis. See [architecture and limits](docs/ARCHITECTURE.md).

## Your data stays on your machine

The recorder has no analytics, accounts, or automatic log upload. Window titles are off by default; process names and PIDs can still identify applications. Protected captures have their own storage usage and are not automatically deleted. Review any capture before sharing it publicly.

Read [data, retention, and privacy](docs/DATA_AND_PRIVACY.md) before leaving it running for a long time or attaching diagnostic files to an issue.

## Development

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

A Windows CI workflow is included for these checks. Tests cover storage fidelity and retention, protected capture windows, source selection, configuration migration, chart history, missing measurements, and native popup lifetime. Unit tests do not establish detection accuracy across every driver, game, or Windows configuration.

For an isolated development recorder, use `scripts/run-dev.ps1` after building. It stores test data in `.verification/` and uses **Ctrl+Alt+Shift+F11** to avoid the usual bookmark shortcut.

Useful next contributions: reproduce a UI bug, review a translation, or test source availability on another GPU. See [CONTRIBUTING.md](CONTRIBUTING.md), [open issues](https://github.com/flochiii/stutter-watch/issues), and the [roadmap](docs/ROADMAP.md).

## License

Stutter Watch is available under the [PolyForm Noncommercial License 1.0.0](LICENSE). Personal study, experiments, hobby projects, and the other permitted uses are covered by its terms. Commercial uses outside the license's permitted purposes require separate permission from the copyright holder. Read the full license for its requirements and provisions for noncommercial organizations.

Dependencies retain their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.md).

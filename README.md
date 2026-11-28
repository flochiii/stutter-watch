<p align="center"><img src="assets/app.svg" width="72" alt="Stutter Watch"></p>
<h1 align="center">Stutter Watch</h1>
<p align="center">A local performance recorder for Windows and Linux.</p>

<p align="center">
  <a href="https://github.com/flochiii/stutter-watch/actions/workflows/ci.yml"><img src="https://github.com/flochiii/stutter-watch/actions/workflows/ci.yml/badge.svg?branch=main" alt="Windows CI"></a>
  <a href="https://github.com/flochiii/stutter-watch/actions/workflows/linux.yml"><img src="https://github.com/flochiii/stutter-watch/actions/workflows/linux.yml/badge.svg?branch=main" alt="Linux CI"></a>
  <a href="https://github.com/flochiii/stutter-watch/releases"><img src="https://img.shields.io/github/v/release/flochiii/stutter-watch" alt="Release"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.97.1-orange?logo=rust" alt="Rust 1.97.1"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-PolyForm%20Noncommercial-6a9ed8" alt="License"></a>
</p>

<p align="center">
  <a href="https://github.com/flochiii/stutter-watch/releases">Download</a> · <a href="docs/WINDOWS_PORTABLE.md">Windows</a> · <a href="docs/LINUX.md">Linux</a> · <a href="docs/GETTING_STARTED.md">Documentation</a>
</p>

Stutter Watch records system activity in the background and saves the context around freezes. Open an incident to inspect its charts and measurements. No accounts or log uploads.

![Monitor](docs/images/windows-monitor.png)

## Features

- CPU, memory, disk and GPU monitoring.
- Automatic highlights for high load and measured delays.
- Bookmarks that preserve 30 seconds before a stutter and 10 seconds after it.
- Compressed history with configurable storage limits.
- Background recording, tray controls and a global bookmark shortcut.
- Ten interface languages.

![Settings](docs/images/linux-data.png)

## Use

1. Open Stutter Watch and leave it running.
2. When a freeze happens, press **Ctrl+Shift+F12** or **Mark stutter**.
3. Open **Incidents** to view the recording.

Closing the window keeps recording. **Quit** stops the application. History defaults to **500 MB / 24 hours**; bookmarked incidents are protected from rotation.

## Build

Requires Rust 1.97.1 and the [Windows](docs/WINDOWS_PORTABLE.md) or [Linux](docs/LINUX.md) build dependencies.

```sh
cargo build --locked --release
```

[Data & privacy](docs/DATA_AND_PRIVACY.md) · [Contributing](CONTRIBUTING.md) · [Roadmap](docs/ROADMAP.md)

## License

[PolyForm Noncommercial 1.0.0](LICENSE). Commercial use outside the license's permitted purposes requires permission. Dependencies retain their [own licenses](THIRD_PARTY_NOTICES.md).

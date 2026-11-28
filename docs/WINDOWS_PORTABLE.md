# Stutter Watch for Windows

[Project](https://github.com/flochiii/stutter-watch) · [Releases](https://github.com/flochiii/stutter-watch/releases) · [License](https://github.com/flochiii/stutter-watch/blob/main/LICENSE)

## Start

**Setup:** open the `-setup.exe` download to install for the current user. The default location is `%LOCALAPPDATA%\stutter-watch`. Use the Start menu shortcut to open it.

**Portable ZIP:**

1. Extract the entire ZIP to a writable local folder, outside OneDrive or other sync folders.
2. Open **stutter-watch.exe**. The monitor panel opens; configuration and recordings are created beside the executable.
3. Leave it running. Press **Ctrl+Shift+F12** after a stutter and wait ten seconds, then inspect **Incidents**.

Opening the executable again shows the existing monitor. Closing the panel keeps recording in the tray; **Quit** stops it. Run normally first. Some ETW sources require administrator privileges; each unavailable source explains why in **Data**.

This build is for **Windows x64**. Windows 11 has been exercised; Windows 10 compatibility needs more testing. The C runtime is statically linked; no separate Visual C++ runtime installation is required. The app is unsigned, so Windows may show a publisher/reputation warning. Download only from this project's releases and verify the checksum before running.

English is the initial language. Ten interface languages are included. Ordinary history defaults to 500 MB / 24 hours; protected captures use additional space and are kept until you remove them. Window titles are off by default. No telemetry is uploaded.

## Verify the download

Download the matching `.sha256` file. In PowerShell, compare its digest with the package:

```powershell
Get-FileHash .\stutter-watch-0.2.0-windows-x64-setup.exe -Algorithm SHA256
Get-Content .\stutter-watch-0.2.0-windows-x64-setup.exe.sha256
```

The hashes must match. Build identity, source commit, and executable checksum are in `BUILD.json`. GitHub build provenance is attached to official release archives; with GitHub CLI installed, verify it using:

```powershell
gh attestation verify .\stutter-watch-0.2.0-windows-x64-setup.exe --repo flochiii/stutter-watch
```

## Update

For an installed copy, run the new setup and select the existing installation folder. Setup asks the recorder to save and stop before replacing it, preserving configuration, recordings and its startup task. Uninstalling also keeps user data.

For a portable copy, quit the recorder and wait for it to exit. Extract the new ZIP into a temporary folder, then copy its program and documentation files into your existing folder, preserving `config.toml` and `logs`. Start it again. Updates are manual.

For Task Scheduler startup or notification identity, keep the executable in a stable location and use the [full guide](https://github.com/flochiii/stutter-watch/blob/main/docs/GETTING_STARTED.md). A copy is included at `docs/GETTING_STARTED.md` inside the archive.

## Limits and license

Yellow indicates load pressure; red indicates measured delays or critical resource signals. An incident does not prove the cause of a visible freeze. Source availability depends on hardware, drivers, and Windows permissions. This package is for Windows x64; see the [Linux guide](LINUX.md) for Linux packages. ARM64 builds are not provided.

Stutter Watch uses **PolyForm Noncommercial License 1.0.0**; see `LICENSE`. Dependencies retain their own terms in `THIRD_PARTY_NOTICES.md` and `licenses/`. The inventory includes normal and build dependencies for this target, plus Rust library notices; development-only test/probe packages are excluded.
